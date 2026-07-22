//! Authenticated harness artifacts, execution, and campaign command adapters.

use crate::{flag, option, repeated_option, required_path, u32_option, u64_option, usize_option};
use huntctl::harness::execution::execute_request;
use huntctl::harness::inspection::inspect_objective;
use huntctl::harness::objective_suite::ArtifactReference;
use huntctl::harness::objective_suite::ObjectiveSuite;
use huntctl::harness::run_contract::{HarnessRunRequest, HarnessRunResult};
use huntctl::learning::native_goal_frozen_policy::NativeGoalFrozenPolicyConfig;
use huntctl::learning::native_goal_reachability::NativeGoalReachabilityConfig;
use huntctl::learning::native_goal_trajectory::NativeGoalTrajectoryConfig;
use huntctl::optimization_request::OptimizationRequest;
use huntctl::optimization_resume::{
    OptimizationResumeEvent, append_optimization_resume_event, initialize_optimization_resume,
    load_optimization_resume,
};
use huntctl::search_evaluator::TournamentDefinition;
use sha2::{Digest as _, Sha256};
use std::env;
use std::error::Error;
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

pub(crate) fn command_campaign(args: &[String]) -> Result<(), Box<dyn Error>> {
    if args.first().map(String::as_str) == Some("seal-native-goal-learning-loop") {
        let command_args = &args[1..];
        let repository_root = repository_root(command_args)?.canonicalize()?;
        let optimization: OptimizationRequest =
            serde_json::from_slice(&fs::read(required_path(command_args, "--request")?)?)?;
        let execution: huntctl::search_evaluator::native_residual_campaign::NativeResidualExecutionBinding =
            serde_json::from_slice(&fs::read(required_path(command_args, "--execution")?)?)?;
        let initial_replay_corpus = repository_artifact(
            &repository_root,
            &required_path(command_args, "--initial-corpus")?,
            "initial replay corpus",
        )?;
        let shard_paths = repeated_option(command_args, "--input");
        if shard_paths.is_empty() {
            return Err(
                "native goal learning-loop request requires at least one --input EPISODES.dseps"
                    .into(),
            );
        }
        let initial_episode_shards = shard_paths
            .iter()
            .map(|path| {
                repository_artifact(
                    &repository_root,
                    Path::new(path),
                    "initial native episode shard",
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let generation_limit = u16::try_from(usize_option(command_args, "--generation-limit", 3)?)?;
        let rollouts_per_generation = u16::try_from(usize_option(
            command_args,
            "--rollouts-per-generation",
            usize::from(optimization.execution.workers),
        )?)?;
        let minimum_tick_budget = u64::from(generation_limit)
            .checked_mul(u64::from(rollouts_per_generation))
            .and_then(|count| count.checked_mul(optimization.budgets.exploration_horizon_ticks))
            .ok_or("native goal learning-loop minimum tick budget overflowed")?;
        let request = huntctl::search_evaluator::native_goal_learning_loop::NativeGoalLearningLoopRequest::seal(
            &optimization,
            &execution,
            initial_replay_corpus,
            initial_episode_shards,
            generation_limit,
            rollouts_per_generation,
            u64_option(command_args, "--simulated-tick-budget", minimum_tick_budget)?,
            NativeGoalTrajectoryConfig::default(),
            NativeGoalReachabilityConfig::default(),
            NativeGoalFrozenPolicyConfig::default(),
            huntctl::search_evaluator::native_goal_learning_loop::NativeGoalLearningLoopResume {
                journal_path: option(command_args, "--journal-path")
                    .ok_or("native goal learning-loop request requires --journal-path PATH")?,
                state_path: option(command_args, "--state-path")
                    .ok_or("native goal learning-loop request requires --state-path PATH")?,
                artifact_root: option(command_args, "--artifact-root")
                    .ok_or("native goal learning-loop request requires --artifact-root PATH")?,
            },
        )?;
        let report = request.validate_files(&repository_root, &optimization, &execution)?;
        let output = required_path(command_args, "--output")?;
        refuse_existing_output(&output, "native goal learning-loop request")?;
        write_new_file(&output, request.to_pretty_json()?)?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("validate-native-goal-learning-loop") {
        let command_args = &args[1..];
        let repository_root = repository_root(command_args)?;
        let request: huntctl::search_evaluator::native_goal_learning_loop::NativeGoalLearningLoopRequest =
            serde_json::from_slice(&fs::read(required_path(command_args, "--loop-request")?)?)?;
        let optimization: OptimizationRequest =
            serde_json::from_slice(&fs::read(required_path(command_args, "--request")?)?)?;
        let execution: huntctl::search_evaluator::native_residual_campaign::NativeResidualExecutionBinding =
            serde_json::from_slice(&fs::read(required_path(command_args, "--execution")?)?)?;
        let report = request.validate_files(&repository_root, &optimization, &execution)?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("status-native-goal-learning-loop") {
        let command_args = &args[1..];
        let repository_root = repository_root(command_args)?;
        let request: huntctl::search_evaluator::native_goal_learning_loop::NativeGoalLearningLoopRequest =
            serde_json::from_slice(&fs::read(required_path(command_args, "--loop-request")?)?)?;
        let optimization: OptimizationRequest =
            serde_json::from_slice(&fs::read(required_path(command_args, "--request")?)?)?;
        let execution: huntctl::search_evaluator::native_residual_campaign::NativeResidualExecutionBinding =
            serde_json::from_slice(&fs::read(required_path(command_args, "--execution")?)?)?;
        let state =
            huntctl::search_evaluator::native_goal_learning_loop::load_native_goal_learning_loop(
                &request,
                &repository_root,
                &optimization,
                &execution,
            )?;
        println!("{}", serde_json::to_string_pretty(&state)?);
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("run-native-goal-learning-loop") {
        let command_args = &args[1..];
        let repository_root = repository_root(command_args)?;
        let request: huntctl::search_evaluator::native_goal_learning_loop::NativeGoalLearningLoopRequest =
            serde_json::from_slice(&fs::read(required_path(command_args, "--loop-request")?)?)?;
        let optimization: OptimizationRequest =
            serde_json::from_slice(&fs::read(required_path(command_args, "--request")?)?)?;
        let execution: huntctl::search_evaluator::native_residual_campaign::NativeResidualExecutionBinding =
            serde_json::from_slice(&fs::read(required_path(command_args, "--execution")?)?)?;
        let report = huntctl::search_evaluator::native_goal_learning_loop_runner::run_native_goal_learning_loop(
            &huntctl::search_evaluator::native_goal_learning_loop_runner::NativeGoalLearningLoopRunConfig {
                repository_root: &repository_root,
                request: &request,
                optimization: &optimization,
                execution: &execution,
                cancellation: None,
            },
        )?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("materialize-native-residual-execution") {
        let command_args = &args[1..];
        let repository_root = repository_root(command_args)?.canonicalize()?;
        let optimization: OptimizationRequest =
            serde_json::from_slice(&fs::read(required_path(command_args, "--request")?)?)?;
        optimization.validate_files(&repository_root)?;
        let output_arg = required_path(command_args, "--output")?;
        if output_arg.is_absolute()
            || output_arg
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err("native residual execution output must be repository-relative".into());
        }
        let output = repository_root.join(output_arg);
        let executable = repository_file(
            &repository_root,
            &required_path(command_args, "--game")?,
            "game executable",
        )?;
        let game_data = repository_file(
            &repository_root,
            &required_path(command_args, "--dvd")?,
            "game data",
        )?;
        let world_context = repository_file(
            &repository_root,
            &required_path(command_args, "--world-context")?,
            "world context",
        )?;
        if output.exists() {
            return Err(format!(
                "native residual execution output already exists: {}",
                output.display()
            )
            .into());
        }
        fs::create_dir_all(&output)?;
        let tape_path = output.join("process-route.tape");
        let program_path = output.join("terminal.dmsp");
        let binding_path = output.join("execution.json");
        let card_fixture_manifest = repository_root
            .join(&optimization.route.timeline.path)
            .with_extension("")
            .join("benchmarks/process_boot.fixture.json");
        let tape = huntctl::search_evaluator::native_residual_campaign::materialize_native_residual_process_tape(
            &repository_root,
            &optimization,
        )?;
        write_new_file(&tape_path, tape.encode()?)?;
        let predicate_source = repository_root.join(&optimization.terminal_predicate.source.path);
        let compiled =
            huntctl::milestone_dsl::compile_source(&fs::read_to_string(predicate_source)?)?;
        write_new_file(&program_path, compiled.bytes)?;
        let binding = huntctl::search_evaluator::native_residual_campaign::NativeResidualExecutionBinding::seal(
            &repository_root,
            &optimization,
            &executable,
            &game_data,
            &tape_path,
            &program_path,
            &world_context,
            &card_fixture_manifest,
            u64::try_from(usize_option(command_args, "--checkpoint-validation-ticks", 8)?)?,
            flag(command_args, "--verify-state-hashes"),
        )?;
        write_new_file(&binding_path, binding.to_pretty_json()?)?;
        let report = binding.validate_files(&repository_root, &optimization)?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("run-residual-optimization") {
        let command_args = &args[1..];
        let repository_root = repository_root(command_args)?;
        let optimization: OptimizationRequest =
            serde_json::from_slice(&fs::read(required_path(command_args, "--request")?)?)?;
        if let Some(path) = option(command_args, "--execution") {
            if option(command_args, "--run-request").is_some()
                || option(command_args, "--game").is_some()
                || option(command_args, "--dvd").is_some()
            {
                return Err(
                    "native residual execution cannot be combined with harness or loose game inputs"
                        .into(),
                );
            }
            let execution: huntctl::search_evaluator::native_residual_campaign::NativeResidualExecutionBinding =
                serde_json::from_slice(&fs::read(path)?)?;
            let report = huntctl::search_evaluator::native_residual_campaign_runner::run_native_residual_campaign(
                &huntctl::search_evaluator::native_residual_campaign_runner::NativeResidualCampaignRunConfig {
                    repository_root: &repository_root,
                    optimization: &optimization,
                    execution: &execution,
                    cancellation: None,
                },
            )?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            return Ok(());
        }
        let template: HarnessRunRequest = if let Some(path) = option(command_args, "--run-request")
        {
            if option(command_args, "--game").is_some() || option(command_args, "--dvd").is_some() {
                return Err(
                    "residual campaign accepts either --run-request or --game/--dvd, not both"
                        .into(),
                );
            }
            serde_json::from_slice(&fs::read(path)?)?
        } else {
            huntctl::residual_campaign_runner::materialize_residual_harness_template(
                &repository_root,
                &optimization,
                &required_path(command_args, "--game")?,
                &required_path(command_args, "--dvd")?,
                u32_option(command_args, "--timeout-seconds", 120)?,
            )?
        };
        if let Some(output) = option(command_args, "--output-run-request").map(PathBuf::from) {
            refuse_existing_output(&output, "residual run-request")?;
            write_new_file(&output, template.to_pretty_json()?)?;
        }
        let report = huntctl::residual_campaign_runner::run_residual_campaign(
            &huntctl::residual_campaign_runner::ResidualCampaignRunConfig {
                repository_root: &repository_root,
                optimization: &optimization,
                harness_template: &template,
            },
        )?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
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

fn repository_file(
    repository_root: &Path,
    path: &Path,
    label: &str,
) -> Result<PathBuf, Box<dyn Error>> {
    let unresolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        repository_root.join(path)
    };
    let resolved = unresolved.canonicalize()?;
    if !resolved.starts_with(repository_root) || !resolved.is_file() {
        return Err(format!("{label} must be a file inside the repository").into());
    }
    Ok(resolved)
}

fn repository_artifact(
    repository_root: &Path,
    path: &Path,
    label: &str,
) -> Result<ArtifactReference, Box<dyn Error>> {
    let resolved = repository_file(repository_root, path, label)?;
    let bytes = fs::read(&resolved)?;
    let relative = resolved
        .strip_prefix(repository_root)?
        .to_str()
        .ok_or_else(|| format!("{label} path is not UTF-8"))?
        .replace(std::path::MAIN_SEPARATOR, "/");
    Ok(ArtifactReference {
        path: relative,
        sha256: huntctl::Digest(Sha256::digest(bytes).into()),
    })
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
