//! Controller DSL compilation, inspection, and static flattening adapters.

use crate::{option, usage_error};
use huntctl::controller_compilation::{ControllerObservationProvenance, compile_static_controller};
use huntctl::controller_program::{
    ControllerProgram, CoordinateFrame, Layer, Operation, StickBlend,
};
use serde_json::json;
use std::error::Error;
use std::fs;
use std::path::PathBuf;

pub(crate) fn command_controller(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("compile") if args.len() == 3 => {
            let source = fs::read_to_string(&args[1])?;
            let program = ControllerProgram::parse(&source)?;
            let bytes = program.encode()?;
            let output = PathBuf::from(&args[2]);
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            fs::write(&output, &bytes)?;
            println!(
                "wrote {} frames, {} layers ({} bytes) to {}",
                program.duration_frames,
                program.layers.len(),
                bytes.len(),
                output.display()
            );
            Ok(())
        }
        Some("inspect") if args.len() == 2 => {
            let bytes = fs::read(&args[1])?;
            let program = ControllerProgram::decode(&bytes)?;
            let version_major = u16::from_le_bytes(bytes[8..10].try_into()?);
            let version_minor = u16::from_le_bytes(bytes[10..12].try_into()?);
            let provenance = ControllerObservationProvenance::for_program(&program);
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "format": "DUSKCTRL",
                    "version": { "major": version_major, "minor": version_minor },
                    "duration_frames": program.duration_frames,
                    "layer_count": program.layers.len(),
                    "static_tape_compilable": provenance.is_static(),
                    "observation_provenance": provenance,
                    "layers": program.layers,
                }))?
            );
            Ok(())
        }
        Some("flatten") if args.len() == 3 => {
            let program = ControllerProgram::decode(&fs::read(&args[1])?)?;
            let tape = compile_static_controller(&program)?;
            let output = PathBuf::from(&args[2]);
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            fs::write(&output, tape.encode()?)?;
            println!(
                "flattened {} controller frames to {}",
                tape.frames.len(),
                output.display()
            );
            Ok(())
        }
        Some("derive-waypoint-policy") if args.len() >= 2 => derive_waypoint_policy(&args[1..]),
        Some("waypoint-policy") if args.len() >= 1 => waypoint_policy(&args[1..]),
        _ => usage_error(),
    }
}

fn waypoint_policy(args: &[String]) -> Result<(), Box<dyn Error>> {
    const FLAGS: &[&str] = &[
        "--duration",
        "--waypoints",
        "--roll-frames",
        "--roll-hold",
        "--magnitude",
        "--button-layers",
    ];
    if args.is_empty() || !(args.len() - 1).is_multiple_of(2) {
        return usage_error();
    }
    for pair in args[1..].chunks_exact(2) {
        if !FLAGS.contains(&pair[0].as_str()) {
            return Err(format!("unknown waypoint-policy option {}", pair[0]).into());
        }
    }

    let output = PathBuf::from(&args[0]);
    let duration = required_u32(args, "--duration")?;
    let waypoints = parse_waypoint_list(
        &option(args, "--waypoints").ok_or("missing required --waypoints LIST")?,
    )?;
    let roll_frames = parse_frame_list(
        &option(args, "--roll-frames").ok_or("missing required --roll-frames LIST")?,
    )?;
    let roll_hold = required_u32(args, "--roll-hold")?;
    let magnitude = required_u32(args, "--magnitude")?;
    let button_layers = option(args, "--button-layers")
        .map(|value| parse_button_layer_list(&value))
        .transpose()?
        .unwrap_or_default();
    if duration == 0 || roll_hold == 0 {
        return Err("duration and roll-hold must be nonzero".into());
    }
    if magnitude == 0 || magnitude > 127 {
        return Err("magnitude must be in 1..=127".into());
    }
    if waypoints.is_empty() {
        return Err("waypoint list must not be empty".into());
    }
    if waypoints[0].0 != 0
        || waypoints.windows(2).any(|pair| pair[0].0 >= pair[1].0)
        || waypoints.iter().any(|(start, _)| *start >= duration)
    {
        return Err(
            "waypoint starts must begin at zero, be strictly increasing, and precede duration"
                .into(),
        );
    }
    if roll_frames.windows(2).any(|pair| pair[0] >= pair[1])
        || roll_frames.iter().any(|frame| *frame >= duration)
    {
        return Err("roll frames must be strictly increasing and less than duration".into());
    }
    if button_layers.iter().any(|(start, count, mask)| {
        *count == 0 || *mask == 0 || start.checked_add(*count).is_none_or(|end| end > duration)
    }) {
        return Err(
            "button layers require nonzero duration/mask and must fit inside duration".into(),
        );
    }

    let mut layers = Vec::with_capacity(waypoints.len() + roll_frames.len() + button_layers.len());
    for (index, (start, target)) in waypoints.iter().copied().enumerate() {
        let end = waypoints
            .get(index + 1)
            .map(|(next, _)| *next)
            .unwrap_or(duration);
        layers.push(Layer {
            start_frame: start,
            duration_frames: end - start,
            operation: Operation::SeekCoordinate {
                blend: StickBlend::Replace,
                frame: CoordinateFrame::World,
                target,
                offset: [0.0; 3],
                stop_radius: 0.0,
                magnitude: magnitude as u8,
            },
        });
    }
    for pulse in roll_frames.iter().copied() {
        layers.push(Layer {
            start_frame: pulse,
            duration_frames: roll_hold.min(duration - pulse),
            operation: Operation::Buttons { mask: 0x0100 },
        });
    }
    for (start, count, mask) in button_layers.iter().copied() {
        layers.push(Layer {
            start_frame: start,
            duration_frames: count,
            operation: Operation::Buttons { mask },
        });
    }
    let program = ControllerProgram {
        duration_frames: duration,
        layers,
    };
    program.validate()?;
    let bytes = program.encode()?;
    if let Some(parent) = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(&output, &bytes)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": "dusklight-waypoint-policy/v1",
            "duration": duration,
            "waypoints": waypoints.iter().map(|(start, target)| json!({
                "start_frame": start,
                "target": target,
            })).collect::<Vec<_>>(),
            "roll_frames": roll_frames,
            "roll_hold": roll_hold,
            "button_layers": button_layers.iter().map(|(start, count, mask)| json!({
                "start_frame": start,
                "duration_frames": count,
                "mask": mask,
            })).collect::<Vec<_>>(),
            "magnitude": magnitude,
            "controller": output,
            "controller_bytes": bytes.len(),
        }))?
    );
    Ok(())
}

fn derive_waypoint_policy(args: &[String]) -> Result<(), Box<dyn Error>> {
    const FLAGS: &[&str] = &[
        "--source-frame",
        "--duration",
        "--interval",
        "--waypoint-phase",
        "--lookahead",
        "--roll-period",
        "--roll-phase",
        "--roll-hold",
        "--roll-frames",
        "--magnitude",
        "--terminal-extension",
    ];
    if args.len() < 2 || !(args.len() - 2).is_multiple_of(2) {
        return usage_error();
    }
    for pair in args[2..].chunks_exact(2) {
        if !FLAGS.contains(&pair[0].as_str()) {
            return Err(format!("unknown derive-waypoint-policy option {}", pair[0]).into());
        }
    }

    let source_frame = required_u64(args, "--source-frame")?;
    let duration = required_u32(args, "--duration")?;
    let interval = required_u32(args, "--interval")?;
    let waypoint_phase = required_u32(args, "--waypoint-phase")?;
    let lookahead = required_u32(args, "--lookahead")?;
    let explicit_roll_frames = option(args, "--roll-frames")
        .map(|value| parse_frame_list(&value))
        .transpose()?;
    let roll_period = option(args, "--roll-period")
        .map(|value| value.parse())
        .transpose()?;
    let roll_phase = option(args, "--roll-phase")
        .map(|value| value.parse())
        .transpose()?;
    let roll_hold = required_u32(args, "--roll-hold")?;
    let magnitude = required_u32(args, "--magnitude")?;
    let terminal_extension = required_f32(args, "--terminal-extension")?;
    if duration == 0 || interval == 0 || lookahead == 0 || roll_hold == 0 {
        return Err("duration, interval, lookahead, and roll-hold must be nonzero".into());
    }
    if magnitude == 0 || magnitude > 127 {
        return Err("magnitude must be in 1..=127".into());
    }
    if waypoint_phase >= interval {
        return Err("waypoint-phase must be less than interval".into());
    }
    let roll_frames = match (explicit_roll_frames, roll_period, roll_phase) {
        (Some(frames), None, None) => frames,
        (None, Some(period), Some(phase)) if period != 0 && phase < period => {
            (phase..duration).step_by(period as usize).collect()
        }
        (Some(_), _, _) => {
            return Err(
                "--roll-frames cannot be combined with --roll-period or --roll-phase".into(),
            );
        }
        _ => {
            return Err("provide either --roll-frames LIST or both --roll-period N and --roll-phase N; periodic values require period > 0 and phase < period".into());
        }
    };
    if roll_frames.windows(2).any(|pair| pair[0] >= pair[1])
        || roll_frames.iter().any(|frame| *frame >= duration)
    {
        return Err("roll frames must be strictly increasing and less than duration".into());
    }
    if !terminal_extension.is_finite() || !(0.0..=10_000.0).contains(&terminal_extension) {
        return Err("terminal-extension must be finite and in 0..=10000".into());
    }

    let trace_path = PathBuf::from(&args[0]);
    let output = PathBuf::from(&args[1]);
    let trace = huntctl::trace::decode(&fs::read(&trace_path)?)?;
    let usable = trace
        .records
        .iter()
        .filter_map(|record| {
            let tape_frame = record.tape_frame?;
            (tape_frame >= source_frame && record.player_present()).then_some((tape_frame, record))
        })
        .collect::<Vec<_>>();
    if usable.is_empty() {
        return Err(format!(
            "trace {} has no player-present records at or after source frame {source_frame}",
            trace_path.display()
        )
        .into());
    }

    let final_frame = usable.last().expect("usable trace is non-empty").0;
    let final_position = usable.last().expect("usable trace is non-empty").1.position;
    let terminal_direction = usable
        .iter()
        .rev()
        .skip(1)
        .find_map(|(_, record)| {
            let dx = final_position[0] - record.position[0];
            let dz = final_position[2] - record.position[2];
            let length = dx.hypot(dz);
            (length > f32::EPSILON && length.is_finite()).then_some((dx / length, dz / length))
        })
        .unwrap_or((0.0, 0.0));
    let mut layers = Vec::new();
    let mut targets = Vec::new();
    let mut starts = vec![0];
    let mut start = if waypoint_phase == 0 {
        interval
    } else {
        waypoint_phase
    };
    while start < duration {
        starts.push(start);
        start = start
            .checked_add(interval)
            .ok_or("waypoint start frame overflowed")?;
    }
    for (index, start) in starts.iter().copied().enumerate() {
        let requested = source_frame
            .checked_add(u64::from(start))
            .and_then(|frame| frame.checked_add(u64::from(lookahead)))
            .ok_or("derived target frame overflowed")?;
        let target_frame = requested.min(final_frame);
        let (observed_frame, record) = usable
            .iter()
            .find(|(frame, _)| *frame >= target_frame)
            .copied()
            .unwrap_or_else(|| *usable.last().expect("usable trace is non-empty"));
        let layer_duration = starts
            .get(index + 1)
            .copied()
            .unwrap_or(duration)
            .checked_sub(start)
            .ok_or("waypoint layer duration underflowed")?;
        let mut target = record.position;
        if requested >= final_frame {
            target[0] += terminal_direction.0 * terminal_extension;
            target[2] += terminal_direction.1 * terminal_extension;
        }
        layers.push(Layer {
            start_frame: start,
            duration_frames: layer_duration,
            operation: Operation::SeekCoordinate {
                blend: StickBlend::Replace,
                frame: CoordinateFrame::World,
                target,
                offset: [0.0; 3],
                stop_radius: 0.0,
                magnitude: magnitude as u8,
            },
        });
        targets.push(json!({
            "start_frame": start,
            "duration_frames": layer_duration,
            "requested_trace_frame": requested,
            "observed_trace_frame": observed_frame,
            "target": target,
        }));
    }

    for pulse in roll_frames.iter().copied() {
        layers.push(Layer {
            start_frame: pulse,
            duration_frames: roll_hold.min(duration - pulse),
            operation: Operation::Buttons { mask: 0x0100 },
        });
    }
    let program = ControllerProgram {
        duration_frames: duration,
        layers,
    };
    program.validate()?;
    let bytes = program.encode()?;
    if let Some(parent) = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(&output, &bytes)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": "dusklight-derived-waypoint-policy/v4",
            "source_trace": trace_path,
            "source_frame": source_frame,
            "final_trace_frame": final_frame,
            "duration": duration,
            "interval": interval,
            "waypoint_phase": waypoint_phase,
            "lookahead": lookahead,
            "roll_period": roll_period,
            "roll_phase": roll_phase,
            "roll_frames": roll_frames,
            "roll_hold": roll_hold,
            "magnitude": magnitude,
            "terminal_extension": terminal_extension,
            "targets": targets,
            "controller": output,
            "controller_bytes": bytes.len(),
        }))?
    );
    Ok(())
}

fn required_u32(args: &[String], flag: &str) -> Result<u32, Box<dyn Error>> {
    option(args, flag)
        .ok_or_else(|| format!("missing required {flag} N").into())
        .and_then(|value| value.parse().map_err(Into::into))
}

fn required_u64(args: &[String], flag: &str) -> Result<u64, Box<dyn Error>> {
    option(args, flag)
        .ok_or_else(|| format!("missing required {flag} N").into())
        .and_then(|value| value.parse().map_err(Into::into))
}

fn required_f32(args: &[String], flag: &str) -> Result<f32, Box<dyn Error>> {
    option(args, flag)
        .ok_or_else(|| format!("missing required {flag} N").into())
        .and_then(|value| value.parse().map_err(Into::into))
}

fn parse_frame_list(value: &str) -> Result<Vec<u32>, Box<dyn Error>> {
    if value.is_empty() {
        return Ok(Vec::new());
    }
    value
        .split(',')
        .map(|frame| {
            frame
                .parse::<u32>()
                .map_err(|error| format!("invalid roll frame {frame:?}: {error}").into())
        })
        .collect()
}

fn parse_waypoint_list(value: &str) -> Result<Vec<(u32, [f32; 3])>, Box<dyn Error>> {
    if value.is_empty() {
        return Ok(Vec::new());
    }
    value
        .split(';')
        .map(|waypoint| {
            let fields = waypoint.split(':').collect::<Vec<_>>();
            if fields.len() != 4 {
                return Err(format!("invalid waypoint {waypoint:?}; expected START:X:Y:Z").into());
            }
            let start = fields[0].parse::<u32>()?;
            let target = [
                fields[1].parse::<f32>()?,
                fields[2].parse::<f32>()?,
                fields[3].parse::<f32>()?,
            ];
            if target.iter().any(|value| !value.is_finite()) {
                return Err(format!("waypoint {waypoint:?} contains a non-finite target").into());
            }
            Ok((start, target))
        })
        .collect()
}

fn parse_button_layer_list(value: &str) -> Result<Vec<(u32, u32, u16)>, Box<dyn Error>> {
    if value.is_empty() {
        return Ok(Vec::new());
    }
    value
        .split(',')
        .map(|layer| {
            let fields = layer.split(':').collect::<Vec<_>>();
            if fields.len() != 3 {
                return Err(format!(
                    "invalid button layer {layer:?}; expected START:DURATION:MASK"
                )
                .into());
            }
            let mask = fields[2]
                .strip_prefix("0x")
                .map(|hex| u16::from_str_radix(hex, 16))
                .unwrap_or_else(|| fields[2].parse::<u16>())?;
            Ok((fields[0].parse()?, fields[1].parse()?, mask))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{parse_button_layer_list, parse_frame_list, parse_waypoint_list};

    #[test]
    fn parses_explicit_roll_frame_schedule() {
        assert_eq!(parse_frame_list("0,19,40,61").unwrap(), [0, 19, 40, 61]);
        assert!(parse_frame_list("0,nope,40").is_err());
    }

    #[test]
    fn parses_timed_world_waypoints() {
        let waypoints = parse_waypoint_list("0:-360:800:-2260;40:-1130:800:-2600").unwrap();
        assert_eq!(waypoints.len(), 2);
        assert_eq!(waypoints[0], (0, [-360.0, 800.0, -2260.0]));
        assert!(parse_waypoint_list("0:1:2").is_err());
        assert!(parse_waypoint_list("0:NaN:2:3").is_err());
    }

    #[test]
    fn parses_timed_button_layers() {
        assert_eq!(
            parse_button_layer_list("0:1:0x0200,8:4:64").unwrap(),
            [(0, 1, 0x0200), (8, 4, 64)]
        );
        assert!(parse_button_layer_list("0:1").is_err());
        assert!(parse_button_layer_list("0:1:nope").is_err());
    }
}
