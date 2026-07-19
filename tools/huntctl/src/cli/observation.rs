//! Authenticated observation-spec publication and inspection adapters.

use crate::{option, usage_error};
use huntctl::observation_view::{MOVEMENT_STATE_V2_ID, ObservationSpec, movement_state_v2_spec};
use serde_json::json;
use std::error::Error;
use std::fs;
use std::path::PathBuf;

pub(crate) fn command_observe(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("spec") if args.get(1).map(String::as_str) == Some(MOVEMENT_STATE_V2_ID) => {
            let spec = movement_state_v2_spec();
            let bytes = spec.canonical_bytes()?;
            if let Some(output) = option(&args[2..], "--output") {
                let output = PathBuf::from(output);
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
                        "output": output,
                        "id": spec.id,
                        "digest": spec.digest()?,
                        "feature_count": spec.feature_count(),
                    }))?
                );
            } else {
                println!("{}", serde_json::to_string_pretty(&spec)?);
            }
            Ok(())
        }
        Some("inspect") if args.len() == 2 => {
            let spec: ObservationSpec = serde_json::from_slice(&fs::read(&args[1])?)?;
            spec.validate()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "path": args[1],
                    "id": spec.id,
                    "objective": spec.objective,
                    "phase": spec.phase,
                    "digest": spec.digest()?,
                    "feature_count": spec.feature_count(),
                    "categorical_features": spec.categorical_features(),
                    "channels": spec.channels,
                    "features": spec.features,
                }))?
            );
            Ok(())
        }
        _ => usage_error(),
    }
}
