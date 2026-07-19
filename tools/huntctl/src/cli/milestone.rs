//! Milestone DSL compilation, formatting, and inspection adapters.

use crate::usage_error;
use huntctl::Digest;
use huntctl::milestone_dsl;
use serde_json::json;
use std::error::Error;
use std::fs;
use std::path::PathBuf;

pub(crate) fn command_milestone(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("compile") if args.len() == 3 => {
            let source = fs::read_to_string(&args[1])?;
            let compiled = milestone_dsl::compile_source(&source)?;
            let output = PathBuf::from(&args[2]);
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            fs::write(&output, &compiled.bytes)?;
            println!(
                "wrote {} milestones ({} bytes, sha256 {}) to {}",
                compiled.definitions.len(),
                compiled.bytes.len(),
                Digest(compiled.program_sha256),
                output.display()
            );
            Ok(())
        }
        Some("inspect") if args.len() == 2 => {
            let decoded = milestone_dsl::decode(&fs::read(&args[1])?)?;
            let definitions = decoded
                .definitions
                .iter()
                .zip(&decoded.program.definitions)
                .map(|(definition, ast)| -> Result<_, milestone_dsl::BinaryError> {
                    let projections = ast
                        .projections
                        .iter()
                        .map(|projection| {
                            Ok(json!({
                                "name": projection.name,
                                "identity": Digest(milestone_dsl::value_projection_identity(projection)?),
                                "items": projection.items,
                            }))
                        })
                        .collect::<Result<Vec<_>, milestone_dsl::BinaryError>>()?;
                    Ok(json!({
                        "id": definition.name,
                        "sha256": Digest(definition.sha256),
                        "value_projections": projections,
                    }))
                })
                .collect::<Result<Vec<_>, _>>()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "format": "DMSP",
                    "program_sha256": Digest(decoded.program_sha256),
                    "definitions": definitions,
                    "source": milestone_dsl::format(&decoded.program)?,
                }))?
            );
            Ok(())
        }
        Some("format") if args.len() == 2 => {
            let source = fs::read_to_string(&args[1])?;
            println!(
                "{}",
                milestone_dsl::format(&milestone_dsl::parse(&source)?)?
            );
            Ok(())
        }
        _ => usage_error(),
    }
}
