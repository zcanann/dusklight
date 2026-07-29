//! Search, evaluation, optimizer, and tournament command adapters.

use crate::{
    flag, option, repeated_option, required_path, timeout_option, u32_option, u64_option,
    usage_error, usize_option,
};
use huntctl::candidate_envelope::{CandidateEnvelope, CandidateEnvelopeSet};
use huntctl::continuous_search::{ContinuousAxes, ContinuousMethod};
use huntctl::harness::run_contract::HarnessRunRequest;
use huntctl::learning::planning_priors::QBeamPriorTable;
use huntctl::milestone_dsl;
use huntctl::motion_path::{MotionPathPlan, PathCancellationHit};
use huntctl::motion_path_golf::{MotionPathGolfSteps, golf_motion_path};
use huntctl::optimization_request::OptimizationRequest;
use huntctl::option_execution::OptionExecution;
use huntctl::option_golf::{RollGolfSteps, golf_roll_option};
use huntctl::roll_option::{RollCancellationHit, RollOptionPlan};
use huntctl::route_workbench::{MaterializeTarget, materialize_lineage};
use huntctl::search::{
    Candidate, CandidateResult, EvaluationArtifact, EvolutionConfig, PopulationManifest,
    RESULTS_SCHEMA, SearchResults, SegmentProfile, collect_results, evolve_population,
    rank_population, write_explicit_population_with_seed, write_seed_population,
};
use huntctl::search_evaluator::native_residual_campaign::NativeResidualExecutionBinding;
use huntctl::search_evaluator::native_suffix_result::NativeTerminalBinding;
use huntctl::search_evaluator::native_suffix_worker::{
    NativeSuffixPrevalidatedFileIdentities, NativeSuffixWorkerLaunch, NativeSuffixWorkerSession,
};
use huntctl::search_evaluator::{
    AnchoredInputGolfConfig, AnchoredObjectiveConfig, AnchoredRouteMinimizeConfig,
    AnchoredSearchRunConfig, BayesianSearchRunConfig, BeamSearchConfig, BootGolfConfig,
    BootMinimizeConfig, ContinuousSearchRunConfig, EvaluateConfig, HarnessEvaluateConfig,
    ProposerTournamentConfig, SearchRunConfig, TournamentDefinition, evaluate_population,
    golf_anchored_inputs, golf_boot, minimize_anchored_route, minimize_boot, run_anchored_search,
    run_bayesian_search, run_beam_search, run_continuous_search, run_proposer_tournament,
    run_search,
};
use huntctl::suffix_batch::{
    NativeSuffixBatch, SuffixProposalMethod, ordon_exit_edge_distance,
    propose_ranked_suffix_refinement, propose_suffix_batch,
};
use huntctl::tape::InputTape;
use serde_json::{Value, json};
use std::error::Error;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

mod execution_commands;
mod optimization_commands;
mod population_commands;
mod suffix_commands;

struct SearchExecutionConfig {
    game: PathBuf,
    dvd: PathBuf,
    working_directory: PathBuf,
    game_args_prefix: Vec<String>,
    timeout: Duration,
    harness: Option<HarnessEvaluateConfig>,
}

fn search_execution_config(args: &[String]) -> Result<SearchExecutionConfig, Box<dyn Error>> {
    if let Some(path) = option(args, "--run-request") {
        if option(args, "--game").is_some()
            || option(args, "--dvd").is_some()
            || option(args, "--working-directory").is_some()
            || option(args, "--timeout-ms").is_some()
            || option(args, "--timeout-seconds").is_some()
            || !repeated_option(args, "--game-arg").is_empty()
        {
            return Err("--run-request is the sole execution authority; do not combine it with --game, --dvd, --working-directory, --game-arg, or timeout options".into());
        }
        let repository_root = fs::canonicalize(
            option(args, "--repository-root")
                .map(PathBuf::from)
                .unwrap_or(std::env::current_dir()?),
        )?;
        let request_template: HarnessRunRequest = serde_json::from_slice(&fs::read(path)?)?;
        request_template.validate_files(&repository_root)?;
        return Ok(SearchExecutionConfig {
            game: repository_root.join(&request_template.executable.path),
            dvd: repository_root.join(&request_template.game_data.path),
            working_directory: repository_root.clone(),
            game_args_prefix: Vec::new(),
            timeout: Duration::from_secs(u64::from(request_template.host_timeout_seconds)),
            harness: Some(HarnessEvaluateConfig {
                repository_root,
                request_template,
            }),
        });
    }
    Ok(SearchExecutionConfig {
        game: required_path(args, "--game")?,
        dvd: required_path(args, "--dvd")?,
        working_directory: option(args, "--working-directory")
            .map(PathBuf::from)
            .unwrap_or(std::env::current_dir()?),
        game_args_prefix: repeated_option(args, "--game-arg"),
        timeout: timeout_option(args)?,
        harness: None,
    })
}

fn bind_route_origin_card_fixture(
    timeline: &huntctl::timeline::Timeline,
    boot: &huntctl::tape::TapeBoot,
    execution: &mut SearchExecutionConfig,
) -> Result<(), Box<dyn Error>> {
    if !execution.game_args_prefix.is_empty() {
        return Err(
            "route-aware search does not accept --game-arg; its execution contract is fixed".into(),
        );
    }
    if !matches!(boot, huntctl::tape::TapeBoot::Process) {
        return Ok(());
    }
    let Some(relative) = timeline
        .origin
        .as_ref()
        .and_then(|origin| origin.card_fixture.as_deref())
    else {
        return Ok(());
    };
    if execution.harness.is_some() {
        return Err(
            "timeline origin card fixtures are not representable by the authenticated run-request/v2 contract"
                .into(),
        );
    }
    let fixture = validated_route_card_fixture_root(relative, &execution.working_directory)?;
    execution
        .game_args_prefix
        .push("--automation-card-fixture".into());
    execution.game_args_prefix.push(
        fixture
            .to_str()
            .ok_or("timeline origin card fixture path is not UTF-8")?
            .to_owned(),
    );
    Ok(())
}

fn validated_route_card_fixture_root(
    relative: &Path,
    repository_root: &Path,
) -> Result<PathBuf, Box<dyn Error>> {
    if relative.as_os_str().is_empty()
        || relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("timeline origin card fixture is not a contained relative path".into());
    }
    let root = fs::canonicalize(repository_root)?;
    let mut candidate = root.clone();
    for component in relative.components() {
        candidate.push(component.as_os_str());
        let metadata = fs::symlink_metadata(&candidate).map_err(|error| {
            format!(
                "cannot inspect timeline origin card fixture {}: {error}",
                candidate.display()
            )
        })?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "timeline origin card fixture {} contains a symbolic link",
                candidate.display()
            )
            .into());
        }
    }
    if !candidate.is_dir() {
        return Err(format!(
            "timeline origin card fixture {} is not a directory",
            candidate.display()
        )
        .into());
    }
    let fixture = fs::canonicalize(&candidate)?;
    if !fixture.starts_with(&root) {
        return Err(format!(
            "timeline origin card fixture {} escapes repository root {}",
            fixture.display(),
            root.display()
        )
        .into());
    }
    Ok(fixture)
}

pub(crate) fn command_search(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some(
            command @ ("evaluate-suffix-batch"
            | "suffix-select"
            | "candidate-to-tape"
            | "suffix-promote-failure"
            | "suffix-refine"
            | "suffix-batch"
            | "candidate-from-tape"),
        ) => suffix_commands::command_suffix(command, args),
        Some(command @ ("evaluate" | "run-route")) => {
            execution_commands::command_execution(command, args)
        }
        Some(
            command @ ("golf-route-inputs"
            | "run"
            | "beam"
            | "continuous"
            | "bayesian"
            | "tournament"
            | "prepare-tournament-lane"
            | "minimize-route"
            | "minimize-boot"
            | "golf-inputs"
            | "golf-boot"
            | "golf-option"
            | "golf-path"),
        ) => optimization_commands::command_optimization(command, args),
        Some(
            command @ ("import-tape" | "seed" | "evolve" | "rank" | "collect" | "inspect"
            | "mock-evaluate"),
        ) => population_commands::command_population(command, args),
        _ => usage_error(),
    }
}

#[cfg(test)]
#[path = "search/tests.rs"]
mod route_card_fixture_tests;
