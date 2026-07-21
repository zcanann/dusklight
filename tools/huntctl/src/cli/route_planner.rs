//! Headless causal route-planner commands.

use crate::{flag, option, required_path, usage_error, usize_option};
use huntctl::route_planner::evaluation::{EvidencePolicy, FeasibilityMode};
use huntctl::route_planner::execution::{PlannerExecutionState, PlannerExecutionStateDocument};
use huntctl::route_planner::logic::FactCatalog;
use huntctl::route_planner::snapshot::StateSnapshot;
use huntctl::route_planner::solver::{ForwardSolver, SolverOptions};
use huntctl::route_planner::transition::MechanicsCatalog;
use serde_json::json;
use std::error::Error;
use std::fs;

pub(crate) fn command_route_planner(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("solve") => solve(&args[1..]),
        Some("state-from-snapshot") => state_from_snapshot(&args[1..]),
        _ => usage_error(),
    }
}

fn state_from_snapshot(args: &[String]) -> Result<(), Box<dyn Error>> {
    let snapshot_path = required_path(args, "--snapshot")?;
    let output = required_path(args, "--output")?;
    let snapshot = StateSnapshot::decode_canonical(&fs::read(&snapshot_path)?)?;
    let document = PlannerExecutionState::new(snapshot)?.to_document()?;
    let bytes = document.canonical_bytes()?;
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
        PlannerExecutionStateDocument::decode_canonical(&fs::read(&state_path)?)?.into_state()?;
    let facts = FactCatalog::decode_canonical(&fs::read(&facts_path)?)?;
    let mechanics = MechanicsCatalog::decode_canonical(&fs::read(&mechanics_path)?)?;
    let goal = mechanics
        .goals
        .iter()
        .find(|goal| goal.id == goal_id)
        .ok_or_else(|| format!("mechanics catalog does not contain goal {goal_id}"))?;
    let options = SolverOptions {
        max_depth: usize_option(args, "--max-depth", SolverOptions::default().max_depth)?,
        max_states: usize_option(args, "--max-states", SolverOptions::default().max_states)?,
        max_resolution_combinations: usize_option(
            args,
            "--max-resolution-combinations",
            SolverOptions::default().max_resolution_combinations,
        )?,
        feasibility_mode: if flag(args, "--upper-bound") {
            FeasibilityMode::UpperBound
        } else {
            FeasibilityMode::Modeled
        },
        evidence_policy: if flag(args, "--research") {
            EvidencePolicy::RESEARCH
        } else {
            EvidencePolicy::ESTABLISHED_ONLY
        },
    };
    let solver = ForwardSolver::new(&facts, &mechanics, &[], options)?;
    let result = solver.solve(state, &goal.predicate)?;
    let report = json!({
        "schema": "dusklight.route-planner.solve-report/v1",
        "goal_id": goal.id,
        "state": state_path,
        "facts": facts_path,
        "mechanics": mechanics_path,
        "result": result,
    });
    if let Some(parent) = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(&output, serde_json::to_vec_pretty(&report)?)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
