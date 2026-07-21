use dusklight_route_planner::execution::{PlannerExecutionState, PlannerExecutionStateDocument};
use dusklight_route_planner::logic::FactCatalog;
use dusklight_route_planner::snapshot::StateSnapshot;
use dusklight_route_planner::transition::MechanicsCatalog;
use dusklight_route_planner_runtime::{
    RuntimeEvidenceMode, RuntimeFeasibilityMode, RuntimeSolveOptions, solve_catalog_goal,
};
use serde_json::json;
use std::env;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    if let Err(error) = run() {
        eprintln!("route-planner: {error}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    match args.first().map(String::as_str) {
        Some("state-from-snapshot") => state_from_snapshot(&args[1..]),
        Some("solve") => solve(&args[1..]),
        Some("help" | "--help" | "-h") | None => {
            print_usage();
            Ok(())
        }
        _ => {
            print_usage();
            Err("unknown route-planner command".into())
        }
    }
}

fn state_from_snapshot(args: &[String]) -> Result<(), Box<dyn Error>> {
    let snapshot_path = required_path(args, "--snapshot")?;
    let output = required_path(args, "--output")?;
    let snapshot = StateSnapshot::decode_canonical(&fs::read(&snapshot_path)?)?;
    let document = PlannerExecutionState::new(snapshot)?.to_document()?;
    let bytes = document.canonical_bytes()?;
    write_file(&output, &bytes)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": document.schema,
            "output": output,
            "sha256": document.digest()?,
            "bytes": bytes.len(),
        }))?
    );
    Ok(())
}

fn solve(args: &[String]) -> Result<(), Box<dyn Error>> {
    let state_path = required_path(args, "--state")?;
    let facts_path = required_path(args, "--facts")?;
    let mechanics_path = required_path(args, "--mechanics")?;
    let output = required_path(args, "--output")?;
    let goal_id = option(args, "--goal").ok_or("missing required --goal ID")?;
    let state =
        PlannerExecutionStateDocument::decode_canonical(&fs::read(state_path)?)?.into_state()?;
    let facts = FactCatalog::decode_canonical(&fs::read(facts_path)?)?;
    let mechanics = MechanicsCatalog::decode_canonical(&fs::read(mechanics_path)?)?;
    let defaults = RuntimeSolveOptions::default();
    let report = solve_catalog_goal(
        state,
        &facts,
        &mechanics,
        &[],
        &goal_id,
        RuntimeSolveOptions {
            max_depth: usize_option(args, "--max-depth", defaults.max_depth)?,
            max_states: usize_option(args, "--max-states", defaults.max_states)?,
            max_resolution_combinations: usize_option(
                args,
                "--max-resolution-combinations",
                defaults.max_resolution_combinations,
            )?,
            feasibility_mode: if flag(args, "--upper-bound") {
                RuntimeFeasibilityMode::UpperBound
            } else {
                RuntimeFeasibilityMode::Modeled
            },
            evidence_mode: if flag(args, "--research") {
                RuntimeEvidenceMode::Research
            } else {
                RuntimeEvidenceMode::EstablishedOnly
            },
        },
    )?;
    let bytes = serde_json::to_vec_pretty(&report)?;
    write_file(&output, &bytes)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn option(args: &[String], name: &str) -> Option<String> {
    args.windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].clone())
}

fn flag(args: &[String], name: &str) -> bool {
    args.iter().any(|argument| argument == name)
}

fn required_path(args: &[String], name: &str) -> Result<PathBuf, Box<dyn Error>> {
    option(args, name)
        .map(PathBuf::from)
        .ok_or_else(|| format!("missing required {name} <path>").into())
}

fn usize_option(args: &[String], name: &str, default: usize) -> Result<usize, Box<dyn Error>> {
    Ok(option(args, name)
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(default))
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, bytes)?;
    Ok(())
}

fn print_usage() {
    eprintln!(
        "Independent TP route planner:\n  route-planner state-from-snapshot --snapshot SNAPSHOT.json --output STATE.json\n  route-planner solve --state STATE.json --facts FACTS.json --mechanics MECHANICS.json --goal ID --output REPORT.json [--max-depth N] [--max-states N] [--max-resolution-combinations N] [--upper-bound] [--research]"
    );
}
