//! Semantic and comparative oracle evaluation adapters.

use crate::{option, required_path};
use huntctl::comparison_oracle::{ComparisonEvidence, ComparisonOracleProgram};
use huntctl::oracle_pipeline::OracleCompositionManifest;
use huntctl::semantic_oracle::{
    RunOutcomeEvidence, SemanticOracleProgram, SupplementalObservations,
};
use std::error::Error;
use std::fs;
use std::path::PathBuf;

pub(crate) fn command_oracle(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("evaluate") => {
            let oracle_args = &args[1..];
            let program: SemanticOracleProgram = serde_json::from_slice(&fs::read(
                required_path(oracle_args, "--program")?,
            )?)?;
            let trace = huntctl::trace::decode(&fs::read(required_path(
                oracle_args,
                "--trace",
            )?)?)?;
            let mut supplemental: SupplementalObservations =
                if let Some(path) = option(oracle_args, "--supplemental") {
                    serde_json::from_slice(&fs::read(path)?)?
                } else {
                    SupplementalObservations::default()
                };
            if let Some(path) = option(oracle_args, "--run-outcome") {
                if supplemental.run_outcome.is_some() {
                    return Err(
                        "run outcome was supplied in both --supplemental and --run-outcome".into(),
                    );
                }
                supplemental.run_outcome = Some(serde_json::from_slice::<RunOutcomeEvidence>(
                    &fs::read(path)?,
                )?);
            }
            write_report(
                &program.evaluate(&trace, &supplemental)?,
                option(oracle_args, "--output"),
            )
        }
        Some("compare") => {
            let oracle_args = &args[1..];
            let program: ComparisonOracleProgram = serde_json::from_slice(&fs::read(
                required_path(oracle_args, "--program")?,
            )?)?;
            let evidence: ComparisonEvidence = serde_json::from_slice(&fs::read(required_path(
                oracle_args,
                "--evidence",
            )?)?)?;
            write_report(
                &program.evaluate(&evidence)?,
                option(oracle_args, "--output"),
            )
        }
        Some("compose") => {
            let oracle_args = &args[1..];
            let manifest: OracleCompositionManifest = serde_json::from_slice(&fs::read(
                required_path(oracle_args, "--manifest")?,
            )?)?;
            write_report(&manifest.compose()?, option(oracle_args, "--output"))
        }
        _ => Err("oracle command: evaluate --program ORACLES.json --trace RUN.trace [--supplemental OBSERVATIONS.json] [--run-outcome OUTCOME.json] [--output REPORT.json] | compose --manifest COMPOSITION.json [--output EVIDENCE.json] | compare --program ORACLES.json --evidence COMPARISON.json [--output REPORT.json]".into()),
    }
}

fn write_report(
    report: &impl serde::Serialize,
    output: Option<String>,
) -> Result<(), Box<dyn Error>> {
    let encoded = serde_json::to_vec_pretty(report)?;
    if let Some(path) = output.map(PathBuf::from) {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, &encoded)?;
    }
    println!("{}", String::from_utf8(encoded)?);
    Ok(())
}
