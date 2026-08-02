//! Authenticated harness artifacts, execution, and campaign command adapters.

use crate::{flag, option, repeated_option, required_path, u32_option, u64_option, usize_option};
use huntctl::harness::execution::execute_request;
use huntctl::harness::objective_suite::ObjectiveSuite;
use huntctl::harness::run_contract::HarnessRunRequest;
use huntctl::learning::native_goal_frozen_policy::NativeGoalFrozenPolicyConfig;
use huntctl::learning::native_goal_reachability::NativeGoalReachabilityConfig;
use huntctl::learning::native_goal_trajectory::NativeGoalTrajectoryConfig;
use huntctl::optimization_request::OptimizationRequest;
use huntctl::optimization_resume::{
    OptimizationResumeEvent, append_optimization_resume_event, initialize_optimization_resume,
    load_optimization_resume,
};
use huntctl::search_evaluator::TournamentDefinition;
use std::error::Error;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

mod helpers;
use helpers::{
    inspect, is_runtime_dependency_path, learning_value_cell_draft_from_directory,
    pin_curriculum_source_checkpoint, refuse_existing_output, repository_artifact,
    repository_build_output, repository_file, repository_game_data, repository_root,
    validate_or_seal_result, write_new_file,
};

pub(crate) fn command_campaign(args: &[String]) -> Result<(), Box<dyn Error>> {
    if args.first().map(String::as_str) == Some("materialize-learning-value-refinement-cell") {
        let command_args = &args[1..];
        let repository_root = repository_root(command_args)?.canonicalize()?;
        let plan_path = repository_file(
            &repository_root,
            &required_path(command_args, "--plan")?,
            "learning-value comparison plan",
        )?;
        let optimization_path = repository_file(
            &repository_root,
            &required_path(command_args, "--request")?,
            "learning-value learning request",
        )?;
        let loop_request_path = repository_file(
            &repository_root,
            &required_path(command_args, "--loop-request")?,
            "learning-value loop request",
        )?;
        let loop_state_path = repository_file(
            &repository_root,
            &required_path(command_args, "--loop-state")?,
            "learning-value loop state",
        )?;
        let plan: huntctl::search_evaluator::learning_value_comparison::LearningValueComparisonPlan =
            serde_json::from_slice(&fs::read(plan_path)?)?;
        let optimization: OptimizationRequest =
            serde_json::from_slice(&fs::read(optimization_path)?)?;
        let loop_request: huntctl::search_evaluator::native_goal_learning_loop::NativeGoalLearningLoopRequest =
            serde_json::from_slice(&fs::read(loop_request_path)?)?;
        let loop_state: huntctl::search_evaluator::native_goal_learning_loop::NativeGoalLearningLoopState =
            serde_json::from_slice(&fs::read(loop_state_path)?)?;
        let checkpoint_id =
            option(command_args, "--checkpoint").ok_or("missing required --checkpoint <id>")?;
        let deterministic_seed = option(command_args, "--seed")
            .ok_or("missing required --seed <u64>")?
            .parse()?;
        let (request, incumbent) = huntctl::search_evaluator::learning_value_matrix::materialize_learning_refinement_request(
            &plan,
            &checkpoint_id,
            deterministic_seed,
            &optimization,
            &loop_request,
            &loop_state,
            &repository_root,
        )?;
        let report = request.validate_files(&repository_root)?;
        let output = repository_build_output(
            &repository_root,
            &required_path(command_args, "--output")?,
            "learning-value refinement cell request",
        )?;
        refuse_existing_output(&output, "learning-value refinement cell request")?;
        write_new_file(&output, request.to_pretty_json()?)?;
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "request": report,
                "learning_incumbent": {
                    "tape": incumbent.tape,
                    "first_hit_tick": incumbent.first_hit_tick,
                    "generation": incumbent.generation,
                    "rollout": incumbent.rollout,
                }
            }))?
        );
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("materialize-learning-value-loop-cell") {
        let command_args = &args[1..];
        let repository_root = repository_root(command_args)?.canonicalize()?;
        let plan_path = repository_file(
            &repository_root,
            &required_path(command_args, "--plan")?,
            "learning-value comparison plan",
        )?;
        let optimization_path = repository_file(
            &repository_root,
            &required_path(command_args, "--request")?,
            "learning-value optimization request",
        )?;
        let execution_path = repository_file(
            &repository_root,
            &required_path(command_args, "--execution")?,
            "learning-value native execution",
        )?;
        let plan: huntctl::search_evaluator::learning_value_comparison::LearningValueComparisonPlan =
            serde_json::from_slice(&fs::read(plan_path)?)?;
        let optimization: OptimizationRequest =
            serde_json::from_slice(&fs::read(optimization_path)?)?;
        let execution: huntctl::search_evaluator::native_residual_campaign::NativeResidualExecutionBinding =
            serde_json::from_slice(&fs::read(execution_path)?)?;
        let checkpoint_id =
            option(command_args, "--checkpoint").ok_or("missing required --checkpoint <id>")?;
        let deterministic_seed = option(command_args, "--seed")
            .ok_or("missing required --seed <u64>")?
            .parse()?;
        let treatment =
            huntctl::search_evaluator::learning_value_matrix::learning_treatment_from_slug(
                &option(command_args, "--treatment").ok_or(
                    "missing required --treatment <demonstration-assisted-state-reactive|from-scratch-state-reactive|learned-then-residual-refinement>",
                )?,
            )?;
        let initial_replay_corpus = repository_artifact(
            &repository_root,
            &required_path(command_args, "--initial-corpus")?,
            "initial replay corpus",
        )?;
        let shard_paths = repeated_option(command_args, "--input");
        if shard_paths.is_empty() {
            return Err(
                "learning-value loop cell requires at least one --input EPISODES.dseps".into(),
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
        let request = huntctl::search_evaluator::learning_value_matrix::materialize_learning_cell_loop_request(
            &plan,
            &checkpoint_id,
            deterministic_seed,
            treatment,
            &optimization,
            &execution,
            initial_replay_corpus,
            initial_episode_shards,
            &repository_root,
        )?;
        let report = request.validate_files(&repository_root, &optimization, &execution)?;
        let output = repository_build_output(
            &repository_root,
            &required_path(command_args, "--output")?,
            "learning-value loop cell request",
        )?;
        refuse_existing_output(&output, "learning-value loop cell request")?;
        write_new_file(&output, request.to_pretty_json()?)?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("materialize-learning-value-learning-cell") {
        let command_args = &args[1..];
        let repository_root = repository_root(command_args)?.canonicalize()?;
        let plan_path = repository_file(
            &repository_root,
            &required_path(command_args, "--plan")?,
            "learning-value comparison plan",
        )?;
        let base_path = repository_file(
            &repository_root,
            &required_path(command_args, "--base-request")?,
            "base optimization request",
        )?;
        let plan: huntctl::search_evaluator::learning_value_comparison::LearningValueComparisonPlan =
            serde_json::from_slice(&fs::read(plan_path)?)?;
        let base: OptimizationRequest = serde_json::from_slice(&fs::read(base_path)?)?;
        let checkpoint_id =
            option(command_args, "--checkpoint").ok_or("missing required --checkpoint <id>")?;
        let deterministic_seed = option(command_args, "--seed")
            .ok_or("missing required --seed <u64>")?
            .parse()?;
        let treatment =
            huntctl::search_evaluator::learning_value_matrix::learning_treatment_from_slug(
                &option(command_args, "--treatment").ok_or(
                    "missing required --treatment <demonstration-assisted-state-reactive|from-scratch-state-reactive|learned-then-residual-refinement>",
                )?,
            )?;
        let request =
            huntctl::search_evaluator::learning_value_matrix::materialize_learning_cell_optimization(
                &plan,
                &checkpoint_id,
                deterministic_seed,
                treatment,
                &base,
                &repository_root,
            )?;
        let output = repository_build_output(
            &repository_root,
            &required_path(command_args, "--output")?,
            "learning-value learning cell authority",
        )?;
        refuse_existing_output(&output, "learning-value learning cell authority")?;
        write_new_file(&output, request.to_pretty_json()?)?;
        println!(
            "{}",
            serde_json::to_string_pretty(&request.validate_files(&repository_root)?)?
        );
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("materialize-learning-value-residual-cell") {
        let command_args = &args[1..];
        let repository_root = repository_root(command_args)?.canonicalize()?;
        let plan_path = repository_file(
            &repository_root,
            &required_path(command_args, "--plan")?,
            "learning-value comparison plan",
        )?;
        let base_path = repository_file(
            &repository_root,
            &required_path(command_args, "--base-request")?,
            "base optimization request",
        )?;
        let plan: huntctl::search_evaluator::learning_value_comparison::LearningValueComparisonPlan =
            serde_json::from_slice(&fs::read(plan_path)?)?;
        let base: OptimizationRequest = serde_json::from_slice(&fs::read(base_path)?)?;
        let checkpoint_id =
            option(command_args, "--checkpoint").ok_or("missing required --checkpoint <id>")?;
        let deterministic_seed = option(command_args, "--seed")
            .ok_or("missing required --seed <u64>")?
            .parse()?;
        let treatment =
            huntctl::search_evaluator::learning_value_matrix::residual_treatment_from_slug(
                &option(command_args, "--treatment").ok_or(
                    "missing required --treatment <independent-random-residual|cem-residual>",
                )?,
            )?;
        let request =
            huntctl::search_evaluator::learning_value_matrix::materialize_residual_cell_request(
                &plan,
                &checkpoint_id,
                deterministic_seed,
                treatment,
                &base,
                &repository_root,
            )?;
        let output = repository_build_output(
            &repository_root,
            &required_path(command_args, "--output")?,
            "learning-value residual cell request",
        )?;
        refuse_existing_output(&output, "learning-value residual cell request")?;
        write_new_file(&output, request.to_pretty_json()?)?;
        println!(
            "{}",
            serde_json::to_string_pretty(&request.validate_files(&repository_root)?)?
        );
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("seal-learning-value-comparison-report") {
        let command_args = &args[1..];
        let repository_root = repository_root(command_args)?.canonicalize()?;
        let plan = repository_artifact(
            &repository_root,
            &required_path(command_args, "--plan")?,
            "learning-value comparison plan",
        )?;
        let cell_paths = repeated_option(command_args, "--cell");
        if cell_paths.is_empty() {
            return Err("learning-value comparison report requires --cell CELL.json...".into());
        }
        let cells = cell_paths
            .iter()
            .map(|path| {
                repository_artifact(
                    &repository_root,
                    Path::new(path),
                    "learning-value cell evidence",
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let negative_control = repository_artifact(
            &repository_root,
            &required_path(command_args, "--negative-control")?,
            "learning-value negative-control report",
        )?;
        let report =
            huntctl::search_evaluator::learning_value_report::LearningValueComparisonReport::seal(
                plan,
                cells,
                negative_control,
                &repository_root,
            )?;
        let output = repository_build_output(
            &repository_root,
            &required_path(command_args, "--output")?,
            "learning-value comparison report",
        )?;
        refuse_existing_output(&output, "learning-value comparison report")?;
        write_new_file(&output, report.to_pretty_json()?)?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("validate-learning-value-comparison-report") {
        let command_args = &args[1..];
        let repository_root = repository_root(command_args)?.canonicalize()?;
        let input_path = repository_file(
            &repository_root,
            &required_path(command_args, "--input")?,
            "learning-value comparison report",
        )?;
        let report: huntctl::search_evaluator::learning_value_report::LearningValueComparisonReport =
            serde_json::from_slice(&fs::read(input_path)?)?;
        report.validate_files(&repository_root)?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("seal-learning-value-cell-evidence") {
        let command_args = &args[1..];
        let repository_root = repository_root(command_args)?.canonicalize()?;
        let plan_path = repository_file(
            &repository_root,
            &required_path(command_args, "--plan")?,
            "learning-value comparison plan",
        )?;
        let plan: huntctl::search_evaluator::learning_value_comparison::LearningValueComparisonPlan =
            serde_json::from_slice(&fs::read(plan_path)?)?;
        let draft_input = option(command_args, "--input");
        let cell_root = option(command_args, "--cell-root");
        let draft = match (draft_input, cell_root) {
            (Some(path), None) => {
                let draft_path = repository_file(
                    &repository_root,
                    Path::new(&path),
                    "learning-value cell draft",
                )?;
                serde_json::from_slice(&fs::read(draft_path)?)?
            }
            (None, Some(path)) => {
                let checkpoint_id = option(command_args, "--checkpoint")
                    .ok_or("directory-backed learning-value evidence requires --checkpoint <id>")?;
                let deterministic_seed = option(command_args, "--seed")
                    .ok_or("directory-backed learning-value evidence requires --seed <u64>")?
                    .parse()?;
                let treatment = option(command_args, "--treatment").ok_or(
                    "directory-backed learning-value evidence requires --treatment <kind>",
                )?;
                let treatment =
                    serde_json::from_value(serde_json::Value::String(treatment.replace('-', "_")))?;
                learning_value_cell_draft_from_directory(
                    &repository_root,
                    Path::new(&path),
                    checkpoint_id,
                    deterministic_seed,
                    treatment,
                )?
            }
            (Some(_), Some(_)) => {
                return Err(
                    "learning-value evidence accepts exactly one of --input or --cell-root".into(),
                );
            }
            (None, None) => {
                return Err(
                    "learning-value evidence requires --input DRAFT.json or --cell-root DIR".into(),
                );
            }
        };
        let evidence =
            huntctl::search_evaluator::learning_value_evidence::LearningValueCellEvidence::seal(
                draft,
                &plan,
                &repository_root,
            )?;
        let output = repository_build_output(
            &repository_root,
            &required_path(command_args, "--output")?,
            "learning-value cell evidence",
        )?;
        refuse_existing_output(&output, "learning-value cell evidence")?;
        write_new_file(&output, evidence.to_pretty_json()?)?;
        println!("{}", serde_json::to_string_pretty(&evidence)?);
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("validate-learning-value-cell-evidence") {
        let command_args = &args[1..];
        let repository_root = repository_root(command_args)?.canonicalize()?;
        let plan_path = repository_file(
            &repository_root,
            &required_path(command_args, "--plan")?,
            "learning-value comparison plan",
        )?;
        let input_path = repository_file(
            &repository_root,
            &required_path(command_args, "--input")?,
            "learning-value cell evidence",
        )?;
        let plan: huntctl::search_evaluator::learning_value_comparison::LearningValueComparisonPlan =
            serde_json::from_slice(&fs::read(plan_path)?)?;
        let evidence: huntctl::search_evaluator::learning_value_evidence::LearningValueCellEvidence =
            serde_json::from_slice(&fs::read(input_path)?)?;
        evidence.validate_files(&plan, &repository_root)?;
        println!("{}", serde_json::to_string_pretty(&evidence)?);
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("seal-learning-value-comparison-plan") {
        let command_args = &args[1..];
        let repository_root = repository_root(command_args)?.canonicalize()?;
        let input = repository_file(
            &repository_root,
            &required_path(command_args, "--input")?,
            "learning-value comparison plan draft",
        )?;
        let draft: huntctl::search_evaluator::learning_value_comparison::LearningValueComparisonPlan =
            serde_json::from_slice(&fs::read(input)?)?;
        let plan = draft.seal(&repository_root)?;
        let report = plan.validate_files(&repository_root)?;
        let output = repository_build_output(
            &repository_root,
            &required_path(command_args, "--output")?,
            "learning-value comparison plan",
        )?;
        refuse_existing_output(&output, "learning-value comparison plan")?;
        write_new_file(&output, plan.to_pretty_json()?)?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("validate-learning-value-comparison-plan") {
        let command_args = &args[1..];
        let repository_root = repository_root(command_args)?.canonicalize()?;
        let input = repository_file(
            &repository_root,
            &required_path(command_args, "--input")?,
            "learning-value comparison plan",
        )?;
        let plan: huntctl::search_evaluator::learning_value_comparison::LearningValueComparisonPlan =
            serde_json::from_slice(&fs::read(input)?)?;
        let report = plan.validate_files(&repository_root)?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("validate-residual-winner-minimization") {
        let command_args = &args[1..];
        let repository_root = repository_root(command_args)?.canonicalize()?;
        let input = repository_file(
            &repository_root,
            &required_path(command_args, "--input")?,
            "residual winner minimization summary",
        )?;
        let summary: huntctl::search_evaluator::residual_winner_minimization::ResidualWinnerMinimizationSummary =
            serde_json::from_slice(&fs::read(input)?)?;
        summary.validate_files(&repository_root)?;
        println!("{}", serde_json::to_string_pretty(&summary)?);
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("prove-residual-winner-cold-replay") {
        let command_args = &args[1..];
        let repository_root = repository_root(command_args)?.canonicalize()?;
        let request_path = repository_file(
            &repository_root,
            &required_path(command_args, "--request")?,
            "residual winner cold replay request",
        )?;
        let execution_path = repository_file(
            &repository_root,
            &required_path(command_args, "--execution")?,
            "residual winner cold replay execution",
        )?;
        let minimization_artifact = repository_artifact(
            &repository_root,
            &required_path(command_args, "--minimization")?,
            "residual winner minimization summary",
        )?;
        let optimization: OptimizationRequest = serde_json::from_slice(&fs::read(request_path)?)?;
        let execution: huntctl::search_evaluator::native_residual_campaign::NativeResidualExecutionBinding =
            serde_json::from_slice(&fs::read(execution_path)?)?;
        let minimization: huntctl::search_evaluator::residual_winner_minimization::ResidualWinnerMinimizationSummary =
            serde_json::from_slice(&fs::read(repository_root.join(&minimization_artifact.path))?)?;
        let output_relative = required_path(command_args, "--output")?;
        if output_relative.is_absolute()
            || output_relative
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
            || !output_relative.starts_with("build")
        {
            return Err(
                "residual winner cold replay output must be a repository-relative build/ path"
                    .into(),
            );
        }
        let proof = huntctl::search_evaluator::residual_winner_cold_replay::run_residual_winner_cold_replay(
            &huntctl::search_evaluator::residual_winner_cold_replay::ResidualWinnerColdReplayConfig {
                repository_root: &repository_root,
                optimization: &optimization,
                execution: &execution,
                minimization_summary: &minimization,
                minimization_summary_artifact: minimization_artifact,
                timeout: Duration::from_secs(u64::from(u32_option(
                    command_args,
                    "--timeout-seconds",
                    120,
                )?)),
                output_root: &repository_root.join(output_relative),
            },
        )?;
        println!("{}", serde_json::to_string_pretty(&proof)?);
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("validate-residual-winner-cold-replay") {
        let command_args = &args[1..];
        let repository_root = repository_root(command_args)?.canonicalize()?;
        let request_path = repository_file(
            &repository_root,
            &required_path(command_args, "--request")?,
            "residual winner cold replay request",
        )?;
        let execution_path = repository_file(
            &repository_root,
            &required_path(command_args, "--execution")?,
            "residual winner cold replay execution",
        )?;
        let proof_path = repository_file(
            &repository_root,
            &required_path(command_args, "--input")?,
            "residual winner cold replay proof",
        )?;
        if proof_path.file_name().and_then(|name| name.to_str())
            != Some(
                huntctl::search_evaluator::residual_winner_cold_replay::RESIDUAL_WINNER_COLD_REPLAY_PROOF_FILE,
            )
        {
            return Err("residual winner cold replay input must be proof.json".into());
        }
        let optimization: OptimizationRequest = serde_json::from_slice(&fs::read(request_path)?)?;
        let execution: huntctl::search_evaluator::native_residual_campaign::NativeResidualExecutionBinding =
            serde_json::from_slice(&fs::read(execution_path)?)?;
        let proof = huntctl::search_evaluator::residual_winner_cold_replay::read_and_validate_residual_winner_cold_replay(
            &repository_root,
            &optimization,
            &execution,
            proof_path
                .parent()
                .ok_or("residual winner cold replay proof has no parent")?,
        )?;
        println!("{}", serde_json::to_string_pretty(&proof)?);
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("promote-residual-winner") {
        let command_args = &args[1..];
        let repository_root = repository_root(command_args)?.canonicalize()?;
        let request_artifact = repository_artifact(
            &repository_root,
            &required_path(command_args, "--request")?,
            "residual winner source request",
        )?;
        let execution_artifact = repository_artifact(
            &repository_root,
            &required_path(command_args, "--execution")?,
            "residual winner source execution",
        )?;
        let selection_path = repository_file(
            &repository_root,
            &required_path(command_args, "--selection")?,
            "residual winner selection",
        )?;
        let output = repository_build_output(
            &repository_root,
            &required_path(command_args, "--output")?,
            "promoted residual request",
        )?;
        refuse_existing_output(&output, "promoted residual request")?;
        let optimization: OptimizationRequest =
            serde_json::from_slice(&fs::read(repository_root.join(&request_artifact.path))?)?;
        let execution: huntctl::search_evaluator::native_residual_campaign::NativeResidualExecutionBinding =
            serde_json::from_slice(&fs::read(repository_root.join(&execution_artifact.path))?)?;
        let selection: huntctl::search_evaluator::native_residual_winner_selection::NativeResidualWinnerSelection =
            serde_json::from_slice(&fs::read(selection_path)?)?;
        if selection.source_request != request_artifact
            || selection.source_execution != execution_artifact
        {
            return Err(
                "residual winner selection differs from the supplied source request or execution"
                    .into(),
            );
        }
        let promoted_id = option(command_args, "--id").ok_or("missing required --id ID")?;
        let promoted = huntctl::search_evaluator::native_residual_winner_promotion::promote_native_residual_winner(
            &huntctl::search_evaluator::native_residual_winner_promotion::NativeResidualWinnerPromotionConfig {
                repository_root: &repository_root,
                optimization: &optimization,
                execution: &execution,
                selection: &selection,
                promoted_id: &promoted_id,
                output_request: &output,
            },
        )?;
        write_new_file(&output, promoted.to_pretty_json()?)?;
        println!("{}", serde_json::to_string_pretty(&promoted)?);
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("minimize-residual-winner") {
        let command_args = &args[1..];
        let repository_root = repository_root(command_args)?.canonicalize()?;
        let source_request = repository_artifact(
            &repository_root,
            &required_path(command_args, "--request")?,
            "minimization source optimization request",
        )?;
        let source_execution = repository_artifact(
            &repository_root,
            &required_path(command_args, "--execution")?,
            "minimization source execution binding",
        )?;
        let source_checkpoint = repository_artifact(
            &repository_root,
            &required_path(command_args, "--checkpoint")?,
            "minimization source checkpoint",
        )?;
        let source_candidate = repository_artifact(
            &repository_root,
            &required_path(command_args, "--candidate")?,
            "minimization source candidate",
        )?;
        let optimization: OptimizationRequest =
            serde_json::from_slice(&fs::read(repository_root.join(&source_request.path))?)?;
        let execution: huntctl::search_evaluator::native_residual_campaign::NativeResidualExecutionBinding =
            serde_json::from_slice(&fs::read(repository_root.join(&source_execution.path))?)?;
        let checkpoint: huntctl::search_evaluator::residual_campaign::ResidualCampaignCheckpoint =
            serde_json::from_slice(&fs::read(repository_root.join(&source_checkpoint.path))?)?;
        let candidate: huntctl::search_evaluator::residual_campaign::ResidualCampaignCandidate =
            serde_json::from_slice(&fs::read(repository_root.join(&source_candidate.path))?)?;
        let output_relative = required_path(command_args, "--output")?;
        if output_relative.is_absolute()
            || output_relative
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
            || !output_relative.starts_with("build")
        {
            return Err(
                "residual winner minimization output must be a repository-relative build/ path"
                    .into(),
            );
        }
        let output = repository_root.join(output_relative);
        let summary = huntctl::search_evaluator::residual_winner_minimization::run_residual_winner_minimization(
            &huntctl::search_evaluator::residual_winner_minimization::ResidualWinnerMinimizationConfig {
                repository_root: &repository_root,
                optimization: &optimization,
                execution: &execution,
                checkpoint: &checkpoint,
                source_request,
                source_execution,
                source_checkpoint,
                source_candidate,
                candidate: &candidate,
                output_root: &output,
                candidate_budget: u64_option(command_args, "--candidate-budget", 256)?,
                resume: flag(command_args, "--resume"),
                cancellation: None,
            },
        )?;
        println!("{}", serde_json::to_string_pretty(&summary)?);
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("seed-residual-reverse-curriculum") {
        let command_args = &args[1..];
        let repository_root = repository_root(command_args)?.canonicalize()?;
        let source_request = repository_artifact(
            &repository_root,
            &required_path(command_args, "--request")?,
            "reverse curriculum source optimization request",
        )?;
        let optimization: OptimizationRequest =
            serde_json::from_slice(&fs::read(repository_root.join(&source_request.path))?)?;
        let required_u64 = |name: &str| -> Result<u64, Box<dyn Error>> {
            Ok(option(command_args, name)
                .ok_or_else(|| format!("reverse curriculum seed requires {name} N"))?
                .parse()?)
        };
        let child = huntctl::search_evaluator::residual_reverse_curriculum::seed_residual_reverse_curriculum_request(
            &optimization,
            source_request,
            option(command_args, "--id")
                .ok_or("reverse curriculum seed requires --id NEW_ID")?,
            huntctl::search_evaluator::residual_reverse_curriculum::ReverseCurriculumSupportPolicy {
                initial_tail_ticks: required_u64("--initial-tail-ticks")?,
                expansion_step_ticks: required_u64("--expansion-step-ticks")?,
                minimum_successes: required_u64("--minimum-successes")?,
                minimum_behavior_classes: required_u64("--minimum-behavior-classes")?,
                minimum_success_millionths: u32::try_from(required_u64(
                    "--minimum-success-millionths",
                )?)?,
            },
        )?;
        let report = child.validate_files(&repository_root)?;
        let output = repository_build_output(
            &repository_root,
            &required_path(command_args, "--output")?,
            "reverse curriculum seed request",
        )?;
        refuse_existing_output(&output, "reverse curriculum seed request")?;
        write_new_file(&output, child.to_pretty_json()?)?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("expand-residual-reverse-curriculum") {
        let command_args = &args[1..];
        let repository_root = repository_root(command_args)?.canonicalize()?;
        let source_request = repository_artifact(
            &repository_root,
            &required_path(command_args, "--request")?,
            "reverse curriculum source optimization request",
        )?;
        let source_execution = repository_artifact(
            &repository_root,
            &required_path(command_args, "--execution")?,
            "reverse curriculum source execution binding",
        )?;
        let source_checkpoint = repository_artifact(
            &repository_root,
            &required_path(command_args, "--checkpoint")?,
            "reverse curriculum source checkpoint",
        )?;
        let source_checkpoint_bytes = fs::read(repository_root.join(&source_checkpoint.path))?;
        let optimization: OptimizationRequest =
            serde_json::from_slice(&fs::read(repository_root.join(&source_request.path))?)?;
        let execution: huntctl::search_evaluator::native_residual_campaign::NativeResidualExecutionBinding =
            serde_json::from_slice(&fs::read(repository_root.join(&source_execution.path))?)?;
        execution.validate_files(&repository_root, &optimization)?;
        let checkpoint: huntctl::search_evaluator::residual_campaign::ResidualCampaignCheckpoint =
            serde_json::from_slice(&source_checkpoint_bytes)?;
        let output = repository_build_output(
            &repository_root,
            &required_path(command_args, "--output")?,
            "expanded reverse curriculum request",
        )?;
        refuse_existing_output(&output, "expanded reverse curriculum request")?;
        let pinned_source_checkpoint = pin_curriculum_source_checkpoint(
            &repository_root,
            &output,
            &source_checkpoint,
            &source_checkpoint_bytes,
        )?;
        let incumbent = optimization
            .incumbent
            .as_ref()
            .ok_or("reverse curriculum source requires an incumbent")?;
        let incumbent_bytes = fs::read(repository_root.join(&incumbent.tape.path))?;
        let incumbent_tape = huntctl::tape::InputTape::decode(&incumbent_bytes)?.tape;
        let child = huntctl::search_evaluator::residual_reverse_curriculum::expand_residual_reverse_curriculum_request(
            &optimization,
            source_request,
            source_execution,
            pinned_source_checkpoint,
            &checkpoint,
            &incumbent_tape,
            &incumbent_bytes,
            option(command_args, "--id")
                .ok_or("reverse curriculum expansion requires --id NEW_ID")?,
        )?;
        let report = child.validate_files(&repository_root)?;
        write_new_file(&output, child.to_pretty_json()?)?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("tighten-residual-horizon") {
        let command_args = &args[1..];
        let repository_root = repository_root(command_args)?.canonicalize()?;
        let request_path = required_path(command_args, "--request")?;
        let execution_path = required_path(command_args, "--execution")?;
        let checkpoint_path = required_path(command_args, "--checkpoint")?;
        let source_request = repository_artifact(
            &repository_root,
            &request_path,
            "horizon source optimization request",
        )?;
        let source_execution = repository_artifact(
            &repository_root,
            &execution_path,
            "horizon source execution binding",
        )?;
        let source_checkpoint = repository_artifact(
            &repository_root,
            &checkpoint_path,
            "horizon source checkpoint",
        )?;
        let optimization: OptimizationRequest =
            serde_json::from_slice(&fs::read(repository_root.join(&source_request.path))?)?;
        let execution: huntctl::search_evaluator::native_residual_campaign::NativeResidualExecutionBinding =
            serde_json::from_slice(&fs::read(repository_root.join(&source_execution.path))?)?;
        execution.validate_files(&repository_root, &optimization)?;
        let checkpoint: huntctl::search_evaluator::residual_campaign::ResidualCampaignCheckpoint =
            serde_json::from_slice(&fs::read(repository_root.join(&source_checkpoint.path))?)?;
        let required_u64 = |name: &str| -> Result<u64, Box<dyn Error>> {
            Ok(option(command_args, name)
                .ok_or_else(|| format!("horizon tightening requires {name} N"))?
                .parse()?)
        };
        let minimum_support_millionths =
            u32::try_from(required_u64("--minimum-support-millionths")?)?;
        let child = huntctl::search_evaluator::residual_horizon_tightening::tighten_residual_horizon_request(
            &optimization,
            source_request,
            source_execution,
            source_checkpoint,
            &checkpoint,
            option(command_args, "--id")
                .ok_or("horizon tightening requires --id NEW_ID")?,
            required_u64("--proposed-horizon")?,
            huntctl::search_evaluator::residual_horizon_tightening::HorizonSupportPolicy {
                minimum_successes: required_u64("--minimum-successes")?,
                minimum_behavior_classes: required_u64("--minimum-behavior-classes")?,
                minimum_support_millionths,
            },
        )?;
        let report = child.validate_files(&repository_root)?;
        let output = required_path(command_args, "--output")?;
        refuse_existing_output(&output, "tightened optimization request")?;
        write_new_file(&output, child.to_pretty_json()?)?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
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
        let demonstration_mode_option = option(command_args, "--demonstration-mode");
        let demonstration_mode = match demonstration_mode_option
            .as_deref()
            .unwrap_or("behavior_cloning_warm_start")
        {
            "absent" => dusklight_learning::native_replay_corpus::DemonstrationMode::Absent,
            "replay_only" => dusklight_learning::native_replay_corpus::DemonstrationMode::ReplayOnly,
            "behavior_cloning_warm_start" => dusklight_learning::native_replay_corpus::DemonstrationMode::BehaviorCloningWarmStart,
            "reverse_curriculum_checkpoints" => dusklight_learning::native_replay_corpus::DemonstrationMode::ReverseCurriculumCheckpoints,
            _ => return Err("--demonstration-mode must be absent, replay_only, behavior_cloning_warm_start, or reverse_curriculum_checkpoints".into()),
        };
        let request = huntctl::search_evaluator::native_goal_learning_loop::NativeGoalLearningLoopRequest::seal(
            &optimization,
            &execution,
            initial_replay_corpus,
            initial_episode_shards,
            generation_limit,
            rollouts_per_generation,
            u64_option(command_args, "--simulated-tick-budget", minimum_tick_budget)?,
            NativeGoalTrajectoryConfig {
                demonstration_mode,
                ..NativeGoalTrajectoryConfig::default()
            },
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
    if args.first().map(String::as_str) == Some("evaluate-native-goal-learning-checkpoints") {
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
        let report = huntctl::search_evaluator::native_goal_learning_loop::evaluate_native_goal_learning_checkpoints(
            &state,
            &repository_root,
        )?;
        let output = repository_build_output(
            &repository_root,
            &required_path(command_args, "--output")?,
            "native goal learning checkpoint report",
        )?;
        refuse_existing_output(&output, "native goal learning checkpoint report")?;
        write_new_file(&output, report.to_pretty_json()?)?;
        println!("{}", serde_json::to_string_pretty(&report)?);
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
        let game_data =
            repository_game_data(&repository_root, &required_path(command_args, "--dvd")?)?;
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
        let pinned_executable = output.join(
            executable
                .file_name()
                .ok_or("game executable path has no file name")?,
        );
        fs::copy(&executable, &pinned_executable)?;
        let executable_parent = executable
            .parent()
            .ok_or("game executable path has no parent directory")?;
        let mut runtime_dependencies = fs::read_dir(executable_parent)?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<Result<Vec<_>, _>>()?;
        runtime_dependencies.retain(|path| path.is_file() && is_runtime_dependency_path(path));
        runtime_dependencies.sort();
        for dependency in runtime_dependencies {
            let file_name = dependency
                .file_name()
                .ok_or("runtime dependency path has no file name")?;
            fs::copy(&dependency, output.join(file_name))?;
        }
        let tape_path = output.join("process-route.tape");
        let program_path = output.join("terminal.dmsp");
        let binding_path = output.join("execution.json");
        let card_fixture_manifest =
            huntctl::search_evaluator::native_residual_campaign::resolve_card_fixture_manifest(
                &repository_root,
                &optimization,
            )?;
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
            &pinned_executable,
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
        let repository_root = repository_root(command_args)?.canonicalize()?;
        let source_request = repository_artifact(
            &repository_root,
            &required_path(command_args, "--request")?,
            "residual optimization request",
        )?;
        let optimization: OptimizationRequest =
            serde_json::from_slice(&fs::read(repository_root.join(&source_request.path))?)?;
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
            let source_execution = repository_artifact(
                &repository_root,
                Path::new(&path),
                "residual optimization execution",
            )?;
            let execution: huntctl::search_evaluator::native_residual_campaign::NativeResidualExecutionBinding =
                serde_json::from_slice(&fs::read(repository_root.join(&source_execution.path))?)?;
            let mut report = huntctl::search_evaluator::native_residual_campaign_runner::run_native_residual_campaign(
                &huntctl::search_evaluator::native_residual_campaign_runner::NativeResidualCampaignRunConfig {
                    repository_root: &repository_root,
                    optimization: &optimization,
                    execution: &execution,
                    cancellation: None,
                },
            )?;
            report.selected_winner = huntctl::search_evaluator::native_residual_winner_selection::select_native_residual_winner(
                &huntctl::search_evaluator::native_residual_winner_selection::NativeResidualWinnerSelectionConfig {
                    repository_root: &repository_root,
                    optimization: &optimization,
                    execution: &execution,
                    source_request,
                    source_execution,
                    minimization_candidate_budget: u64_option(
                        command_args,
                        "--minimization-candidate-budget",
                        256,
                    )?,
                    cold_replay_timeout: Duration::from_secs(u64::from(u32_option(
                        command_args,
                        "--selection-timeout-seconds",
                        120,
                    )?)),
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

#[cfg(test)]
#[path = "harness/tests.rs"]
mod tests;
