//! Controller DSL compilation, inspection, and static flattening adapters.

use crate::usage_error;
use huntctl::controller_compilation::{ControllerObservationProvenance, compile_static_controller};
use huntctl::controller_program::ControllerProgram;
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
        _ => usage_error(),
    }
}
