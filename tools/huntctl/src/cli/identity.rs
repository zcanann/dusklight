//! Artifact-identity compatibility inspection adapter.

use crate::{option, required_path};
use huntctl::{ArtifactIdentity, CompatibilityMode, ensure_compatible};
use serde_json::json;
use std::error::Error;
use std::fs;

pub(crate) fn command_identity(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("compare") => {
            let compare_args = &args[1..];
            let mode: CompatibilityMode = option(compare_args, "--mode")
                .ok_or("identity compare requires --mode MODE")?
                .parse()?;
            let expected_path = required_path(compare_args, "--expected")?;
            let actual_path = required_path(compare_args, "--actual")?;
            let expected: ArtifactIdentity = serde_json::from_slice(&fs::read(&expected_path)?)?;
            let actual: ArtifactIdentity = serde_json::from_slice(&fs::read(&actual_path)?)?;
            expected.validate().map_err(|error| {
                format!(
                    "invalid expected identity {}: {error}",
                    expected_path.display()
                )
            })?;
            actual.validate().map_err(|error| {
                format!(
                    "invalid actual identity {}: {error}",
                    actual_path.display()
                )
            })?;
            ensure_compatible(mode, &expected, &actual)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": "huntctl-identity-comparison/v1",
                    "mode": mode.as_str(),
                    "compatible": true,
                    "expected": expected_path,
                    "actual": actual_path,
                }))?
            );
            Ok(())
        }
        _ => Err("identity command: compare --mode replay|trace-merge|model-training|checkpoint-restore|cross-build-comparison|cross-fidelity-comparison --expected EXPECTED.json --actual ACTUAL.json".into()),
    }
}
