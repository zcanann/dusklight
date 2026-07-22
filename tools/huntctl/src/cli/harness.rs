//! Authenticated harness artifacts, execution, and campaign command adapters.

use crate::{flag, option, repeated_option, required_path, u32_option, usize_option};
use huntctl::harness::execution::execute_request;
use huntctl::harness::inspection::inspect_objective;
use huntctl::harness::objective_suite::ObjectiveSuite;
use huntctl::harness::run_contract::{HarnessRunRequest, HarnessRunResult};
use huntctl::optimization_request::OptimizationRequest;
use huntctl::optimization_resume::{
    OptimizationResumeEvent, append_optimization_resume_event, initialize_optimization_resume,
    load_optimization_resume,
};
use huntctl::search_evaluator::TournamentDefinition;
use std::env;
use std::error::Error;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

pub(crate) fn command_campaign(args: &[String]) -> Result<(), Box<dyn Error>> {
    if args.first().map(String::as_str) == Some("init-optimization-resume") {
        let command_args = &args[1..];
        let request: OptimizationRequest =
            serde_json::from_slice(&fs::read(required_path(command_args, "--request")?)?)?;
        let state = initialize_optimization_resume(&request, &repository_root(command_args)?)?;
        println!("{}", serde_json::to_string_pretty(&state)?);
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("status-optimization-resume") {
        let command_args = &args[1..];
        let request: OptimizationRequest =
            serde_json::from_slice(&fs::read(required_path(command_args, "--request")?)?)?;
        let state = load_optimization_resume(&request, &repository_root(command_args)?)?;
        println!("{}", serde_json::to_string_pretty(&state)?);
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("append-optimization-resume") {
        let command_args = &args[1..];
        let request: OptimizationRequest =
            serde_json::from_slice(&fs::read(required_path(command_args, "--request")?)?)?;
        let event: OptimizationResumeEvent =
            serde_json::from_slice(&fs::read(required_path(command_args, "--event")?)?)?;
        let state =
            append_optimization_resume_event(&request, &repository_root(command_args)?, event)?;
        println!("{}", serde_json::to_string_pretty(&state)?);
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("seal-optimization-request") {
        let command_args = &args[1..];
        let input = required_path(command_args, "--input")?;
        let output = required_path(command_args, "--output")?;
        refuse_existing_output(&output, "optimization-request")?;
        let repository_root = repository_root(command_args)?;
        let mut request: OptimizationRequest = serde_json::from_slice(&fs::read(input)?)?;
        request.refresh_content_sha256()?;
        let report = request.validate_files(&repository_root)?;
        write_new_file(&output, request.to_pretty_json()?)?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("validate-optimization-request") {
        let command_args = &args[1..];
        let input = required_path(command_args, "--input")?;
        let request: OptimizationRequest = serde_json::from_slice(&fs::read(input)?)?;
        let report = request.validate_files(&repository_root(command_args)?)?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    let proposer_names = repeated_option(args, "--proposer");
    let tournament_definition_path = option(args, "--definition").map(PathBuf::from);
    let proposers = if proposer_names.is_empty()
        && let Some(path) = tournament_definition_path.as_ref()
    {
        let definition: TournamentDefinition = serde_json::from_slice(&fs::read(path)?)?;
        huntctl::harness::campaign::campaign_proposers_from_definition(&definition)?
    } else if proposer_names.is_empty() {
        vec![huntctl::harness::campaign::CampaignProposer::Scripted]
    } else {
        proposer_names
            .iter()
            .map(|name| name.parse())
            .collect::<Result<Vec<_>, _>>()?
    };
    let repository_root = repository_root(args)?;
    let suite = required_path(args, "--suite")?;
    let case = option(args, "--case").ok_or("missing required --case ID")?;
    let output = required_path(args, "--output")?;
    let plan_config = huntctl::harness::campaign::CampaignPlanConfig {
        repository_root: &repository_root,
        suite_path: &suite,
        case_id: &case,
        output_root: &output,
        proposers: &proposers,
    };
    if flag(args, "--dry-run") {
        let plan = huntctl::harness::campaign::resolve_campaign_plan(&plan_config)?;
        println!("{}", serde_json::to_string_pretty(&plan)?);
        return Ok(());
    }
    let request_template = required_path(args, "--run-request")?;
    let tournament_definition = tournament_definition_path
        .as_deref()
        .ok_or("campaign execution requires --definition TOURNAMENT.json")?;
    let report =
        huntctl::harness::campaign::run_campaign(&huntctl::harness::campaign::CampaignRunConfig {
            plan: plan_config,
            request_template_path: &request_template,
            tournament_definition_path: tournament_definition,
            workers: usize_option(args, "--workers", 4)?,
        })?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    if !report.passed {
        let blocker = report.first_blocker.as_ref().map(|blocker| {
            format!(
                "; first {} {}: {}; artifact: {}",
                blocker.kind,
                blocker.value,
                blocker.message,
                blocker.artifact.display()
            )
        });
        return Err(format!(
            "campaign did not meet expected terminal class{}; report: {}",
            blocker.as_deref().unwrap_or(""),
            report.plan.outputs.report.display(),
        )
        .into());
    }
    Ok(())
}

pub(crate) fn command_harness(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("validate-suite") => {
            let command_args = &args[1..];
            let suite_path = required_path(command_args, "--suite")?;
            let repository_root = repository_root(command_args)?;
            let suite: ObjectiveSuite = serde_json::from_slice(&fs::read(&suite_path)?)?;
            let report = suite.validate_files(&repository_root)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Some("seal-suite") => {
            let command_args = &args[1..];
            let input = required_path(command_args, "--input")?;
            let output = required_path(command_args, "--output")?;
            if output.exists() {
                return Err(format!(
                    "objective suite output already exists: {}",
                    output.display()
                )
                .into());
            }
            let repository_root = repository_root(command_args)?;
            let mut suite: ObjectiveSuite = serde_json::from_slice(&fs::read(&input)?)?;
            suite.refresh_content_sha256()?;
            let report = suite.validate_files(&repository_root)?;
            write_new_file(&output, suite.to_pretty_json()?)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Some("validate-run-request") => {
            let command_args = &args[1..];
            let request: HarnessRunRequest = serde_json::from_slice(&fs::read(required_path(
                command_args,
                "--request",
            )?)?)?;
            let report = request.validate_files(&repository_root(command_args)?)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Some("seal-run-request") => {
            let command_args = &args[1..];
            let input = required_path(command_args, "--input")?;
            let output = required_path(command_args, "--output")?;
            refuse_existing_output(&output, "run-request")?;
            let repository_root = repository_root(command_args)?;
            let mut request: HarnessRunRequest = serde_json::from_slice(&fs::read(&input)?)?;
            request.refresh_content_sha256()?;
            let report = request.validate_files(&repository_root)?;
            write_new_file(&output, request.to_pretty_json()?)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Some("validate-run-result") => validate_or_seal_result(&args[1..], false),
        Some("seal-run-result") => validate_or_seal_result(&args[1..], true),
        Some("execute") => {
            let command_args = &args[1..];
            let request: HarnessRunRequest = serde_json::from_slice(&fs::read(required_path(
                command_args,
                "--request",
            )?)?)?;
            let result = execute_request(
                &request,
                &repository_root(command_args)?,
                u32_option(command_args, "--attempt", 1)?,
            )?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            Ok(())
        }
        Some("inspect-objective") => inspect(args),
        _ => Err("harness command: validate-suite|seal-suite|validate-run-request|seal-run-request|validate-run-result|seal-run-result|execute|inspect-objective (use --help for arguments)".into()),
    }
}

fn inspect(args: &[String]) -> Result<(), Box<dyn Error>> {
    let command_args = &args[1..];
    let request: HarnessRunRequest =
        serde_json::from_slice(&fs::read(required_path(command_args, "--request")?)?)?;
    let result_path = option(command_args, "--result").map(PathBuf::from);
    let result: Option<HarnessRunResult> = result_path
        .as_ref()
        .map(|path| -> Result<_, Box<dyn Error>> { Ok(serde_json::from_slice(&fs::read(path)?)?) })
        .transpose()?;
    let artifact_root = option(command_args, "--artifact-root").map(PathBuf::from);
    if result.is_some() != artifact_root.is_some() {
        return Err(
            "harness inspect-objective requires --result and --artifact-root together".into(),
        );
    }
    let inspection = inspect_objective(
        &request,
        &repository_root(command_args)?,
        result.as_ref().zip(artifact_root.as_deref()),
    )?;
    print!("{inspection}");
    Ok(())
}

fn validate_or_seal_result(args: &[String], seal: bool) -> Result<(), Box<dyn Error>> {
    let result_path = required_path(args, if seal { "--input" } else { "--result" })?;
    let output = seal.then(|| required_path(args, "--output")).transpose()?;
    if let Some(output) = &output {
        refuse_existing_output(output, "run-result")?;
    }
    let request: HarnessRunRequest =
        serde_json::from_slice(&fs::read(required_path(args, "--request")?)?)?;
    request.validate_files(&repository_root(args)?)?;
    let artifact_root = required_path(args, "--artifact-root")?;
    let mut result: HarnessRunResult = serde_json::from_slice(&fs::read(&result_path)?)?;
    if seal {
        result.refresh_content_sha256()?;
    }
    let report = result.validate_files(&request, &artifact_root)?;
    if let Some(output) = output {
        write_new_file(&output, result.to_pretty_json()?)?;
    }
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn repository_root(args: &[String]) -> Result<PathBuf, Box<dyn Error>> {
    Ok(option(args, "--repository-root")
        .map(PathBuf::from)
        .unwrap_or(env::current_dir()?))
}

fn refuse_existing_output(path: &Path, label: &str) -> Result<(), Box<dyn Error>> {
    if path.exists() {
        return Err(format!("harness {label} output already exists: {}", path.display()).into());
    }
    Ok(())
}

fn write_new_file(path: &Path, bytes: Vec<u8>) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(&bytes)?;
    file.flush()?;
    Ok(())
}
