//! Suite-wide native/offline conformance command adapter.

use crate::{option, required_path};
use huntctl::harness::conformance::{ConformanceConfig, run_conformance};
use huntctl::harness::run_contract::HarnessFidelityMode;
use std::env;
use std::error::Error;
use std::path::PathBuf;

pub(crate) fn command_conformance(args: &[String]) -> Result<(), Box<dyn Error>> {
    let repository_root = option(args, "--repository-root")
        .map(PathBuf::from)
        .unwrap_or(env::current_dir()?);
    let suite = required_path(args, "--suite")?;
    let executable = required_path(args, "--executable")?;
    let game_data = required_path(args, "--game-data")?;
    let output = required_path(args, "--output")?;
    let fidelity = match option(args, "--fidelity").as_deref() {
        None | Some("headless") => HarnessFidelityMode::Headless,
        Some("unpaced-headful") => HarnessFidelityMode::UnpacedHeadful,
        Some("realtime-headful") => HarnessFidelityMode::RealtimeHeadful,
        Some(value) => {
            return Err(format!(
                "unknown --fidelity {value:?}; expected headless, unpaced-headful, or realtime-headful"
            )
            .into());
        }
    };
    let report = run_conformance(&ConformanceConfig {
        repository_root: &repository_root,
        suite_path: &suite,
        executable: &executable,
        game_data: &game_data,
        output_root: &output,
        fidelity,
    })?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    if !report.passed {
        return Err(format!(
            "objective conformance failed: {}; report: {}",
            report.first_failure.as_deref().unwrap_or("unknown failure"),
            report.report.display()
        )
        .into());
    }
    Ok(())
}
