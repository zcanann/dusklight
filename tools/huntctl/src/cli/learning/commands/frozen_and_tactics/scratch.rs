use super::*;
use huntctl::search_evaluator::native_scratch_heading::{
    NativeScratchHeadingRunConfig, inspect_native_scratch_heading_checkpoint,
    run_native_scratch_heading_refinement,
};
use huntctl::search_evaluator::native_scratch_learner::{
    NativeScratchRunConfig, run_native_scratch_learner,
};

pub(super) fn command(command: &str, learn_args: &[String]) -> Result<(), Box<dyn Error>> {
    let repository_root = fs::canonicalize(
        option(learn_args, "--repository-root")
            .map(PathBuf::from)
            .unwrap_or(std::env::current_dir()?),
    )?;
    if command == "inspect-scratch-headings" {
        let input = resolve_path(learn_args, "--input", &repository_root)?;
        println!(
            "{}",
            serde_json::to_string_pretty(&inspect_native_scratch_heading_checkpoint(&input)?)?
        );
        return Ok(());
    }
    let request: OptimizationRequest =
        serde_json::from_slice(&fs::read(required_path(learn_args, "--request")?)?)?;
    let execution: NativeResidualExecutionBinding =
        serde_json::from_slice(&fs::read(required_path(learn_args, "--execution")?)?)?;
    match command {
        "scratch-route" => run_route(learn_args, &repository_root, &request, &execution),
        "refine-scratch-headings" => {
            refine_headings(learn_args, &repository_root, &request, &execution, false)
        }
        "refine-scratch-fine-headings" => {
            refine_headings(learn_args, &repository_root, &request, &execution, true)
        }
        _ => usage_error(),
    }
}

fn run_route(
    args: &[String],
    repository_root: &Path,
    request: &OptimizationRequest,
    execution: &NativeResidualExecutionBinding,
) -> Result<(), Box<dyn Error>> {
    let output = resolve_path(args, "--output", repository_root)?;
    let report = run_native_scratch_learner(&NativeScratchRunConfig {
        repository_root,
        optimization: request,
        execution,
        output_root: &output,
        seed: u64_option(args, "--seed", 0)?,
        episodes: u64_option(args, "--episodes", 100)?,
        maximum_episode_ticks: u32::try_from(u64_option(args, "--maximum-episode-ticks", 900)?)?,
        epsilon_per_million: u32::try_from(u64_option(args, "--epsilon-per-million", 200_000)?)?,
        maximum_wall_time: Duration::from_secs(u64_option(args, "--wall-time-seconds", 600)?),
        cold_replay_timeout: Duration::from_secs(u64_option(
            args,
            "--cold-replay-timeout-seconds",
            120,
        )?),
    })?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": report.schema,
            "report": output.join("report.json"),
            "completed_episodes": report.completed_episodes,
            "stop_reason": report.stop_reason,
            "unique_transitions": report.unique_transitions,
            "terminal_episodes": report.terminal_episodes,
            "fastest_selected_ticks": report.fastest_selected_ticks,
            "learner_updates": report.learner_updates,
            "changed_choices": report.changed_choices,
            "deletion_attempts": report.deletion_attempts,
            "deletion_terminal_attempts": report.deletion_terminal_attempts,
            "deletion_strict_winners": report.deletion_strict_winners,
            "deletion_candidates_remaining": report.deletion_candidates_remaining,
            "native_ticks": report.native_ticks,
            "native_wall_micros": report.native_wall_micros,
            "wall_micros": report.wall_micros,
            "first_terminal_wall_micros": report.first_terminal_wall_micros,
        }))?
    );
    Ok(())
}

fn refine_headings(
    args: &[String],
    repository_root: &Path,
    request: &OptimizationRequest,
    execution: &NativeResidualExecutionBinding,
    fine: bool,
) -> Result<(), Box<dyn Error>> {
    let source = resolve_path(args, "--source", repository_root)?;
    let scratch_source = if fine {
        resolve_path(args, "--scratch-source", repository_root)?
    } else {
        source.clone()
    };
    let output = resolve_path(args, "--output", repository_root)?;
    let scratch = NativeScratchRunConfig {
        repository_root,
        optimization: request,
        execution,
        output_root: &scratch_source,
        seed: u64_option(args, "--seed", 0)?,
        episodes: 1_000_000,
        maximum_episode_ticks: u32::try_from(u64_option(args, "--maximum-episode-ticks", 900)?)?,
        epsilon_per_million: u32::try_from(u64_option(args, "--epsilon-per-million", 200_000)?)?,
        maximum_wall_time: Duration::from_secs(1),
        cold_replay_timeout: Duration::from_secs(u64_option(
            args,
            "--cold-replay-timeout-seconds",
            120,
        )?),
    };
    let report = run_native_scratch_heading_refinement(&NativeScratchHeadingRunConfig {
        scratch,
        source_heading_root: fine.then_some(source.as_path()),
        heading_count: if fine { 32 } else { 16 },
        output_root: &output,
        candidate_limit: u64_option(args, "--candidate-limit", 1_000)?,
        maximum_wall_time: Duration::from_secs(u64_option(args, "--wall-time-seconds", 600)?),
    })?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": report.schema,
            "report": output.join("report.json"),
            "stop_reason": report.stop_reason,
            "attempted_candidates": report.attempted_candidates,
            "terminal_candidates": report.terminal_candidates,
            "strict_winners": report.strict_winners,
            "candidates_remaining": report.candidates_remaining,
            "fastest_selected_ticks": report.fastest_selected_ticks,
            "native_ticks": report.native_ticks,
            "native_wall_micros": report.native_wall_micros,
            "wall_micros": report.wall_micros,
        }))?
    );
    Ok(())
}

fn resolve_path(
    args: &[String],
    argument: &str,
    repository_root: &Path,
) -> Result<PathBuf, Box<dyn Error>> {
    let path = required_path(args, argument)?;
    Ok(if path.is_absolute() {
        path
    } else {
        repository_root.join(path)
    })
}
