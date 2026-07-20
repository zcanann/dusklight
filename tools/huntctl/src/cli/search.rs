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
use huntctl::option_execution::OptionExecution;
use huntctl::option_golf::{RollGolfSteps, golf_roll_option};
use huntctl::roll_option::{RollCancellationHit, RollOptionPlan};
use huntctl::route_workbench::{MaterializeTarget, materialize_lineage};
use huntctl::search::{
    Candidate, CandidateResult, EvaluationArtifact, EvolutionConfig, PopulationManifest,
    RESULTS_SCHEMA, SearchResults, SegmentProfile, collect_results, evolve_population,
    rank_population, write_explicit_population_with_seed, write_seed_population,
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
use std::path::PathBuf;
use std::time::Duration;

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

pub(crate) fn command_search(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("suffix-select") => {
            let search_args = &args[1..];
            let candidate_path = required_path(search_args, "--candidate")?;
            let batch_path = required_path(search_args, "--batch")?;
            let selected_id = option(search_args, "--id").ok_or("missing required --id")?;
            let output = required_path(search_args, "--output")?;
            let seed: Candidate = serde_json::from_slice(&fs::read(candidate_path)?)?;
            let batch: NativeSuffixBatch = serde_json::from_slice(&fs::read(batch_path)?)?;
            let selected = batch
                .candidates
                .iter()
                .find(|candidate| candidate.id == selected_id)
                .ok_or("selected suffix candidate is absent from its batch")?;
            let candidate = Candidate {
                schema: huntctl::search::CANDIDATE_SCHEMA.into(),
                segment: seed.segment,
                boot: seed.boot.clone(),
                actions: selected.actions.clone(),
                ancestry: huntctl::search::Ancestry {
                    generation: seed.ancestry.generation.saturating_add(1),
                    parent_id: Some(seed.id()?),
                    mutation: Some(format!("selected native suffix {selected_id}")),
                    intervention: None,
                },
            };
            candidate.validate()?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            let mut encoded = serde_json::to_vec_pretty(&candidate)?;
            encoded.push(b'\n');
            fs::write(&output, encoded)?;
            println!("selected {selected_id} into {}", output.display());
            Ok(())
        }
        Some("candidate-to-tape") => {
            let search_args = &args[1..];
            let input = required_path(search_args, "--input")?;
            let output = required_path(search_args, "--output")?;
            let candidate: Candidate = serde_json::from_slice(&fs::read(input)?)?;
            let tape = candidate.compile()?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            fs::write(&output, tape.encode()?)?;
            println!(
                "wrote {} candidate frames to {}",
                tape.frames.len(),
                output.display()
            );
            Ok(())
        }
        Some("suffix-promote-failure") => {
            let search_args = &args[1..];
            let candidate_path = required_path(search_args, "--candidate")?;
            let batch_path = required_path(search_args, "--batch")?;
            let results_path = required_path(search_args, "--results")?;
            let output = required_path(search_args, "--output")?;
            let seed: Candidate = serde_json::from_slice(&fs::read(candidate_path)?)?;
            let batch: NativeSuffixBatch = serde_json::from_slice(&fs::read(batch_path)?)?;
            let results: Value = serde_json::from_slice(&fs::read(results_path)?)?;
            if results.get("status").and_then(Value::as_str) != Some("passed") {
                return Err("failure promotion requires a passed native batch result".into());
            }
            let (winner_id, distance) = results
                .get("candidates")
                .and_then(Value::as_array)
                .ok_or("native batch result has no candidates")?
                .iter()
                .filter(|result| result.get("success").and_then(Value::as_bool) == Some(false))
                .filter_map(|result| {
                    let id = result.get("id")?.as_str()?;
                    let position = result
                        .pointer("/terminal_observation/position")?
                        .as_array()?;
                    let x = position.first()?.as_f64()?;
                    let z = position.get(2)?.as_f64()?;
                    (x.is_finite() && z.is_finite()).then_some((id, ordon_exit_edge_distance(x, z)))
                })
                .min_by(|left, right| left.1.total_cmp(&right.1).then_with(|| left.0.cmp(right.0)))
                .ok_or("native batch result has no finite failed terminal observations")?;
            let selected = batch
                .candidates
                .iter()
                .find(|candidate| candidate.id == winner_id)
                .ok_or("best failed result is absent from its batch")?;
            let promoted = Candidate {
                schema: huntctl::search::CANDIDATE_SCHEMA.into(),
                segment: seed.segment,
                boot: seed.boot.clone(),
                actions: selected.actions.clone(),
                ancestry: huntctl::search::Ancestry {
                    generation: seed.ancestry.generation.saturating_add(1),
                    parent_id: Some(seed.id()?),
                    mutation: Some(format!(
                        "native failure promotion {winner_id}; exit-edge distance {distance:.6}"
                    )),
                    intervention: None,
                },
            };
            promoted.validate()?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            let mut encoded = serde_json::to_vec_pretty(&promoted)?;
            encoded.push(b'\n');
            fs::write(&output, encoded)?;
            println!(
                "promoted failed candidate {winner_id} at signed exit-edge distance {distance:.6} to {}",
                output.display()
            );
            Ok(())
        }
        Some("suffix-refine") => {
            let search_args = &args[1..];
            let candidate_path = required_path(search_args, "--candidate")?;
            let batch_path = required_path(search_args, "--batch")?;
            let results_path = required_path(search_args, "--results")?;
            let output = required_path(search_args, "--output")?;
            let candidate: Candidate = serde_json::from_slice(&fs::read(candidate_path)?)?;
            let batch: NativeSuffixBatch = serde_json::from_slice(&fs::read(batch_path)?)?;
            let results: Value = serde_json::from_slice(&fs::read(results_path)?)?;
            if results.get("status").and_then(Value::as_str) != Some("passed") {
                return Err(
                    "ranked suffix refinement requires a passed native batch result".into(),
                );
            }
            let terminal_observations = results
                .get("candidates")
                .and_then(Value::as_array)
                .ok_or("native batch result has no candidates")?
                .iter()
                .filter(|result| result.get("success").and_then(Value::as_bool) == Some(false))
                .map(|result| {
                    let id = result
                        .get("id")
                        .and_then(Value::as_str)
                        .ok_or("native batch candidate has no id")?;
                    let position = result
                        .pointer("/terminal_observation/position")
                        .and_then(Value::as_array)
                        .ok_or("native batch candidate has no terminal position")?;
                    let x = position
                        .first()
                        .and_then(Value::as_f64)
                        .ok_or("native batch candidate terminal x is absent or non-finite")?;
                    let z = position
                        .get(2)
                        .and_then(Value::as_f64)
                        .ok_or("native batch candidate terminal z is absent or non-finite")?;
                    if !x.is_finite() || !z.is_finite() {
                        return Err("native batch candidate terminal position is non-finite");
                    }
                    Ok((id.to_owned(), x, z))
                })
                .collect::<Result<Vec<_>, &str>>()?;
            let refined = propose_ranked_suffix_refinement(
                &candidate,
                &batch,
                &terminal_observations,
                usize_option(search_args, "--candidate-budget", 107)?,
            )?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            let mut encoded = serde_json::to_vec_pretty(&refined)?;
            encoded.push(b'\n');
            fs::write(&output, encoded)?;
            println!(
                "wrote {} progress-ranked suffix refinements to {}",
                refined.candidates.len(),
                output.display()
            );
            Ok(())
        }
        Some("suffix-batch") => {
            let search_args = &args[1..];
            let candidate_path = required_path(search_args, "--candidate")?;
            let output = required_path(search_args, "--output")?;
            let candidate: Candidate = serde_json::from_slice(&fs::read(candidate_path)?)?;
            let method = match option(search_args, "--method")
                .ok_or(
                    "missing required --method deletion|delete-hold|button-edge|heading|corner|corner-wide|collision|fine-heading|fine-terminal|lane-shift|fine-lane-shift|early-lane-shift|magnitude|asymmetric-lane-shift|post-collision|recovery-bias|timing|path|terminal",
                )?
                .as_str()
            {
                "deletion" => SuffixProposalMethod::Deletion,
                "delete-hold" => SuffixProposalMethod::DeleteHold,
                "button-edge" => SuffixProposalMethod::ButtonEdge,
                "heading" => SuffixProposalMethod::Heading,
                "corner" => SuffixProposalMethod::Corner,
                "corner-wide" => SuffixProposalMethod::CornerWide,
                "collision" => SuffixProposalMethod::Collision,
                "fine-heading" => SuffixProposalMethod::FineHeading,
                "fine-terminal" => SuffixProposalMethod::FineTerminal,
                "lane-shift" => SuffixProposalMethod::LaneShift,
                "fine-lane-shift" => SuffixProposalMethod::FineLaneShift,
                "early-lane-shift" => SuffixProposalMethod::EarlyLaneShift,
                "magnitude" => SuffixProposalMethod::Magnitude,
                "asymmetric-lane-shift" => SuffixProposalMethod::AsymmetricLaneShift,
                "post-collision" => SuffixProposalMethod::PostCollision,
                "recovery-bias" => SuffixProposalMethod::RecoveryBias,
                "timing" => SuffixProposalMethod::Timing,
                "path" => SuffixProposalMethod::Path,
                "terminal" => SuffixProposalMethod::Terminal,
                value => return Err(format!("unknown suffix proposal method {value:?}").into()),
            };
            let batch = propose_suffix_batch(
                &candidate,
                usize_option(search_args, "--source-frame", 440)?,
                &option(search_args, "--source-boundary-fingerprint")
                    .ok_or("missing required --source-boundary-fingerprint VALUE")?,
                usize_option(search_args, "--maximum-ticks", 125)?,
                usize_option(search_args, "--candidate-budget", 126)?,
                method,
            )?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            let mut encoded = serde_json::to_vec_pretty(&batch)?;
            encoded.push(b'\n');
            fs::write(&output, encoded)?;
            println!(
                "wrote {} native suffix candidates ({} ticks each) to {}",
                batch.candidates.len(),
                batch.maximum_ticks,
                output.display()
            );
            Ok(())
        }
        Some("candidate-from-tape") => {
            if args.len() < 2 {
                return usage_error();
            }
            let search_args = &args[1..];
            let input = required_path(search_args, "--input")?;
            let output = required_path(search_args, "--output")?;
            let segment = option(search_args, "--segment")
                .ok_or("missing required --segment PROFILE")?
                .parse::<SegmentProfile>()?;
            let start = usize_option(search_args, "--start", 0)?;
            let decoded = InputTape::decode(&fs::read(&input)?)?;
            let available = decoded
                .tape
                .frames
                .len()
                .checked_sub(start)
                .ok_or("candidate tape start exceeds input length")?;
            let frames = usize_option(search_args, "--frames", available)?;
            let end = start
                .checked_add(frames)
                .ok_or("candidate tape range overflows")?;
            if frames == 0 || end > decoded.tape.frames.len() {
                return Err("candidate tape range must be nonempty and inside the input".into());
            }
            let mut tape = InputTape {
                frames: decoded.tape.frames[start..end].to_vec(),
                ..decoded.tape
            };
            if flag(search_args, "--normalize-port-one") {
                let disconnected = huntctl::tape::RawPadState {
                    connected: false,
                    error: -1,
                    ..huntctl::tape::RawPadState::default()
                };
                for frame in &mut tape.frames {
                    frame.owned_ports = 0x01;
                    frame.pads[1..].fill(disconnected);
                }
            }
            let candidate = Candidate::from_absolute_tape(segment, &tape)?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            fs::write(&output, serde_json::to_vec_pretty(&candidate)?)?;
            println!(
                "wrote {} frames as {} actions to {} (port-one-normalized: {})",
                candidate.frame_count(),
                candidate.actions.len(),
                output.display(),
                flag(search_args, "--normalize-port-one")
            );
            Ok(())
        }
        Some("evaluate") => {
            let search_args = &args[1..];
            let population = required_path(search_args, "--population")?;
            let output = required_path(search_args, "--output")?;
            let results = option(search_args, "--results")
                .map(PathBuf::from)
                .unwrap_or_else(|| output.join("results.json"));
            let execution = search_execution_config(search_args)?;
            let report = evaluate_population(&EvaluateConfig {
                population_path: population,
                game: execution.game,
                dvd: execution.dvd,
                output_root: output,
                episode_store: option(search_args, "--episode-store").map(PathBuf::from),
                results_path: results,
                working_directory: execution.working_directory,
                game_args_prefix: execution.game_args_prefix,
                workers: usize_option(search_args, "--workers", 4)?,
                repetitions: u32_option(search_args, "--repetitions", 3)?,
                timeout: execution.timeout,
                harness: execution.harness,
            })?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Some("run-route") => {
            let search_args = &args[1..];
            let timeline_path = required_path(search_args, "--timeline")?;
            let timeline =
                huntctl::timeline::Timeline::parse(&fs::read_to_string(&timeline_path)?)?;
            let artifact_root = timeline_path
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."));
            timeline.validate_artifacts(Some(artifact_root))?;
            let segment_name = option(search_args, "--segment")
                .ok_or("missing required --segment TIMELINE_SEGMENT")?;
            let segment = timeline
                .segments
                .get(&segment_name)
                .ok_or_else(|| format!("unknown timeline segment {segment_name:?}"))?;
            if !matches!(
                segment.profile,
                SegmentProfile::Fsp103ToFsp104 | SegmentProfile::LinkControlToTunnelCrawlStart
            ) {
                return Err(format!(
                    "route search requires an anchored movement profile, got {}",
                    segment.profile.as_str()
                )
                .into());
            }
            let lineage = option(search_args, "--lineage").unwrap_or_else(|| "main".into());
            let parent_segment = segment
                .parent
                .as_ref()
                .ok_or("anchored route search requires a child segment with an explicit parent")?;
            let prefix = materialize_lineage(
                &timeline,
                artifact_root,
                &lineage,
                MaterializeTarget::ThroughSegment(parent_segment.clone()),
            )?;
            let through_goal = huntctl::route_workbench::materialize_segment_chain(
                &timeline,
                artifact_root,
                &segment.id,
            )?;
            if through_goal.steps.len() != prefix.steps.len() + 1
                || through_goal.steps.last().map(|step| step.segment.as_str())
                    != Some(segment_name.as_str())
                || through_goal.steps[..prefix.steps.len()]
                    .iter()
                    .map(|step| step.segment.as_str())
                    .ne(prefix.steps.iter().map(|step| step.segment.as_str()))
                || through_goal.tape.frames.len() <= prefix.tape.frames.len()
            {
                return Err(format!(
                    "segment {segment_name:?} is not an exact structural child of parent {parent_segment:?} on lineage {lineage:?}"
                )
                .into());
            }
            let source_segment_id = prefix
                .steps
                .last()
                .map(|step| step.segment.as_str())
                .ok_or("anchored route search requires a nonempty immutable prefix")?;
            let source_fingerprint = timeline.segments[source_segment_id].end_fingerprint.clone();
            let suffix = InputTape {
                tick_rate_numerator: through_goal.tape.tick_rate_numerator,
                tick_rate_denominator: through_goal.tape.tick_rate_denominator,
                frames: through_goal.tape.frames[prefix.tape.frames.len()..].to_vec(),
                ..InputTape::default()
            };
            let observed_candidate = Candidate::from_absolute_tape(segment.profile, &suffix)?;
            let seed_candidate = if let Some(path) = option(search_args, "--candidate") {
                let candidate: Candidate = serde_json::from_slice(&fs::read(path)?)?;
                candidate.validate()?;
                if candidate.segment != segment.profile {
                    return Err("route-search candidate profile does not match the segment".into());
                }
                candidate
            } else {
                observed_candidate
            };

            let output = required_path(search_args, "--output")?;
            let execution = search_execution_config(search_args)?;
            let game = execution.game;
            let dvd = execution.dvd;
            let working_directory = execution.working_directory;
            if !game.is_file() || !dvd.is_file() || !working_directory.is_dir() {
                return Err(
                    "route search requires existing game/DVD files and working directory".into(),
                );
            }
            let size = usize_option(search_args, "--size", 16)?;
            let generations = u32_option(search_args, "--generations", 2)?;
            let elite_count = usize_option(search_args, "--elites", (size / 4).max(1))?;
            let workers = usize_option(search_args, "--workers", 4)?;
            let repetitions = u32_option(search_args, "--repetitions", 3)?;
            let timeout = execution.timeout;
            let rng_seed = u64_option(search_args, "--rng-seed", 1)?;
            if !execution.game_args_prefix.is_empty() {
                return Err(
                    "route search does not accept --game-arg; its execution contract is fixed"
                        .into(),
                );
            }
            if generations == 0
                || size == 0
                || elite_count == 0
                || elite_count > size
                || workers == 0
                || repetitions == 0
            {
                return Err(
                    "route search counts and elite bounds must be nonzero and valid".into(),
                );
            }
            let output_name = output
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or("route-search output requires a UTF-8 final path component")?;
            let objective_root = output.with_file_name(format!("{output_name}.objective"));
            if objective_root.exists() {
                return Err(format!(
                    "route-search objective directory already exists: {}",
                    objective_root.display()
                )
                .into());
            }
            fs::create_dir_all(&objective_root)?;
            let prefix_path = objective_root.join("prefix.tape");
            fs::write(&prefix_path, prefix.tape.encode()?)?;
            let select_goal = |segment_id: &str,
                               requested: Option<String>,
                               option_name: &str|
             -> Result<&huntctl::timeline::Goal, Box<dyn Error>> {
                let available = timeline
                    .goals
                    .values()
                    .filter(|goal| {
                        goal.segment == segment_id
                            || timeline
                                .proofs
                                .iter()
                                .any(|proof| proof.segment == segment_id && proof.goal == goal.id)
                    })
                    .collect::<Vec<_>>();
                if let Some(id) = requested {
                    let goal = timeline
                        .goals
                        .get(&id)
                        .ok_or_else(|| format!("unknown route goal {id:?}"))?;
                    if !available.iter().any(|candidate| candidate.id == goal.id) {
                        return Err(format!(
                            "segment {segment_id:?} neither defines nor proves goal {id:?}"
                        )
                        .into());
                    }
                    return Ok(goal);
                }
                if available.len() != 1 {
                    return Err(format!(
                        "segment {segment_id:?} defines or proves {} goals; select one with {option_name}",
                        available.len()
                    )
                    .into());
                }
                Ok(available[0])
            };
            let source_goal = select_goal(
                parent_segment,
                option(search_args, "--source-goal"),
                "--source-goal GOAL",
            )?;
            let target_goal =
                select_goal(&segment_name, option(search_args, "--goal"), "--goal GOAL")?;

            let mut progress_goals = Vec::new();
            let mut progress_goal_ids = std::collections::BTreeSet::new();
            for id in repeated_option(search_args, "--progress-goal") {
                if id == source_goal.id || id == target_goal.id {
                    return Err(format!(
                        "route progress goal {id:?} duplicates the selected source or target goal"
                    )
                    .into());
                }
                if !progress_goal_ids.insert(id.clone()) {
                    return Err(format!("duplicate route progress goal {id:?}").into());
                }
                let goal = timeline
                    .goals
                    .get(&id)
                    .ok_or_else(|| format!("unknown route progress goal {id:?}"))?;
                let available = goal.segment == segment_name
                    || timeline
                        .proofs
                        .iter()
                        .any(|proof| proof.segment == segment_name && proof.goal == goal.id);
                if !available {
                    return Err(format!(
                        "segment {segment_name:?} neither defines nor proves progress goal {id:?}"
                    )
                    .into());
                }
                progress_goals.push(goal);
            }

            let mut combined_program: Option<milestone_dsl::MilestoneProgram> = None;
            let mut names = std::collections::BTreeSet::new();
            for goal in std::iter::once(source_goal)
                .chain(progress_goals.iter().copied())
                .chain(std::iter::once(target_goal))
            {
                let relative = timeline
                    .goal_predicate_source(&goal.id)
                    .ok_or_else(|| format!("route goal {:?} has no predicate source", goal.id))?;
                let mut program =
                    milestone_dsl::parse(&fs::read_to_string(artifact_root.join(relative))?)?;
                program
                    .definitions
                    .retain(|definition| definition.name == goal.predicate);
                if program.definitions.len() != 1 {
                    return Err(format!(
                        "route goal {:?} predicate source does not define {:?}",
                        goal.id, goal.predicate
                    )
                    .into());
                }
                if let Some(combined) = &mut combined_program {
                    if combined.version != program.version {
                        return Err("route goal predicate sources use incompatible versions".into());
                    }
                    for definition in program.definitions {
                        if !names.insert(definition.name.clone()) {
                            return Err(format!(
                                "route goals select duplicate predicate {:?}",
                                definition.name
                            )
                            .into());
                        }
                        combined.definitions.push(definition);
                    }
                } else {
                    names.insert(program.definitions[0].name.clone());
                    combined_program = Some(program);
                }
            }
            let compiled = milestone_dsl::compile(
                &combined_program.expect("source and target goals always provide predicates"),
            )?;
            let program_path = objective_root.join("milestones.dmsp");
            fs::write(&program_path, &compiled.bytes)?;

            let summary = run_anchored_search(&AnchoredSearchRunConfig {
                search: SearchRunConfig {
                    segment: segment.profile,
                    seed_candidate: Some(seed_candidate),
                    game: game.clone(),
                    dvd: dvd.clone(),
                    output_root: output,
                    working_directory,
                    game_args_prefix: Vec::new(),
                    generations,
                    population_size: size,
                    elite_count,
                    workers,
                    repetitions,
                    timeout,
                    rng_seed,
                    harness: execution.harness,
                },
                objective: AnchoredObjectiveConfig {
                    segment: segment.profile,
                    prefix_tape: prefix_path,
                    milestone_program: program_path,
                    game,
                    dvd,
                    source_milestone: source_goal.predicate.clone(),
                    source_boundary_fingerprint: source_fingerprint,
                    goal_milestone: target_goal.predicate.clone(),
                },
            })?;
            println!("{}", serde_json::to_string_pretty(&summary)?);
            Ok(())
        }
        Some("golf-route-inputs") => {
            let search_args = &args[1..];
            let timeline_path = required_path(search_args, "--timeline")?;
            let timeline =
                huntctl::timeline::Timeline::parse(&fs::read_to_string(&timeline_path)?)?;
            let artifact_root = timeline_path
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."));
            timeline.validate_artifacts(Some(artifact_root))?;
            let segment_name = option(search_args, "--segment")
                .ok_or("missing required --segment TIMELINE_SEGMENT")?;
            let segment = timeline
                .segments
                .get(&segment_name)
                .ok_or_else(|| format!("unknown timeline segment {segment_name:?}"))?;
            let parent_segment = segment.parent.as_ref().ok_or(
                "route input golf requires a child segment with an explicit parent predicate",
            )?;
            let anchor_segment =
                option(search_args, "--anchor-segment").unwrap_or_else(|| parent_segment.clone());
            if !timeline.segments.contains_key(&anchor_segment) {
                return Err(format!("unknown route input-golf anchor {anchor_segment:?}").into());
            }
            let prefix = huntctl::route_workbench::materialize_segment_chain(
                &timeline,
                artifact_root,
                &anchor_segment,
            )?;
            let through_goal = huntctl::route_workbench::materialize_segment_chain(
                &timeline,
                artifact_root,
                &segment.id,
            )?;
            if through_goal.steps.len() <= prefix.steps.len()
                || through_goal.steps.last().map(|step| step.segment.as_str())
                    != Some(segment_name.as_str())
                || through_goal.steps[..prefix.steps.len()]
                    .iter()
                    .map(|step| step.segment.as_str())
                    .ne(prefix.steps.iter().map(|step| step.segment.as_str()))
            {
                return Err(format!(
                    "segment {segment_name:?} is not a structural descendant of input-golf anchor {anchor_segment:?}"
                )
                .into());
            }
            if through_goal.steps[prefix.steps.len()..]
                .iter()
                .any(|step| timeline.segments[&step.segment].profile != segment.profile)
            {
                return Err(
                    "route input golf cannot cross a segment-profile boundary after its anchor"
                        .into(),
                );
            }
            let suffix = InputTape {
                boot: prefix.tape.boot.clone(),
                tick_rate_numerator: through_goal.tape.tick_rate_numerator,
                tick_rate_denominator: through_goal.tape.tick_rate_denominator,
                frames: through_goal.tape.frames[prefix.tape.frames.len()..].to_vec(),
            };
            let candidate = Candidate::from_absolute_tape(segment.profile, &suffix)?;

            let select_goal = |segment_id: &str,
                               requested: Option<String>,
                               option_name: &str|
             -> Result<&huntctl::timeline::Goal, Box<dyn Error>> {
                let available = timeline
                    .goals
                    .values()
                    .filter(|goal| {
                        goal.segment == segment_id
                            || timeline
                                .proofs
                                .iter()
                                .any(|proof| proof.segment == segment_id && proof.goal == goal.id)
                    })
                    .collect::<Vec<_>>();
                if let Some(id) = requested {
                    let goal = timeline
                        .goals
                        .get(&id)
                        .ok_or_else(|| format!("unknown route goal {id:?}"))?;
                    if !available.iter().any(|candidate| candidate.id == goal.id) {
                        return Err(format!(
                            "segment {segment_id:?} neither defines nor proves goal {id:?}"
                        )
                        .into());
                    }
                    return Ok(goal);
                }
                if available.len() != 1 {
                    return Err(format!(
                        "segment {segment_id:?} defines or proves {} goals; select one with {option_name}",
                        available.len()
                    )
                    .into());
                }
                Ok(available[0])
            };
            let source_goal = select_goal(
                &anchor_segment,
                option(search_args, "--source-goal"),
                "--source-goal GOAL",
            )?;
            let target_goal =
                select_goal(&segment_name, option(search_args, "--goal"), "--goal GOAL")?;
            let mut combined_program: Option<milestone_dsl::MilestoneProgram> = None;
            let mut names = std::collections::BTreeSet::new();
            for goal in [source_goal, target_goal] {
                let relative = timeline
                    .goal_predicate_source(&goal.id)
                    .ok_or_else(|| format!("route goal {:?} has no predicate source", goal.id))?;
                let mut program =
                    milestone_dsl::parse(&fs::read_to_string(artifact_root.join(relative))?)?;
                program
                    .definitions
                    .retain(|definition| definition.name == goal.predicate);
                if program.definitions.len() != 1 {
                    return Err(format!(
                        "route goal {:?} predicate source does not define {:?}",
                        goal.id, goal.predicate
                    )
                    .into());
                }
                if let Some(combined) = &mut combined_program {
                    if combined.version != program.version {
                        return Err("route goal predicate sources use incompatible versions".into());
                    }
                    for definition in program.definitions {
                        if names.insert(definition.name.clone()) {
                            combined.definitions.push(definition);
                        }
                    }
                } else {
                    names.insert(program.definitions[0].name.clone());
                    combined_program = Some(program);
                }
            }

            let output = required_path(search_args, "--output")?;
            let output_name = output
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or("route input-golf output requires a UTF-8 final path component")?;
            let objective_root = output.with_file_name(format!("{output_name}.objective"));
            if output.exists() || objective_root.exists() {
                return Err("route input-golf output and objective paths must both be new".into());
            }
            fs::create_dir_all(&objective_root)?;
            let prefix_path = objective_root.join("prefix.tape");
            fs::write(&prefix_path, prefix.tape.encode()?)?;
            let compiled = milestone_dsl::compile(
                &combined_program.expect("source and target goals always provide predicates"),
            )?;
            let program_path = objective_root.join("milestones.dmsp");
            fs::write(&program_path, &compiled.bytes)?;
            let execution = search_execution_config(search_args)?;
            let summary = golf_anchored_inputs(&AnchoredInputGolfConfig {
                candidate,
                objective: AnchoredObjectiveConfig {
                    segment: segment.profile,
                    prefix_tape: prefix_path,
                    milestone_program: program_path,
                    game: execution.game,
                    dvd: execution.dvd,
                    source_milestone: source_goal.predicate.clone(),
                    source_boundary_fingerprint: timeline.segments[&anchor_segment]
                        .end_fingerprint
                        .clone(),
                    goal_milestone: target_goal.predicate.clone(),
                },
                output_root: output,
                working_directory: execution.working_directory,
                game_args_prefix: execution.game_args_prefix,
                workers: usize_option(search_args, "--workers", 4)?,
                repetitions: u32_option(search_args, "--repetitions", 3)?,
                candidate_budget: usize_option(search_args, "--candidate-budget", 256)?,
                timeout: execution.timeout,
                harness: execution.harness,
            })?;
            println!("{}", serde_json::to_string_pretty(&summary)?);
            Ok(())
        }
        Some("run") => {
            let search_args = &args[1..];
            let segment: SegmentProfile = option(search_args, "--segment")
                .ok_or("missing required --segment ID")?
                .parse()?;
            let seed_candidate = if let Some(path) = option(search_args, "--candidate") {
                let candidate: Candidate = serde_json::from_slice(&fs::read(path)?)?;
                candidate.validate()?;
                if candidate.segment != segment {
                    return Err("candidate segment does not match --segment".into());
                }
                Some(candidate)
            } else {
                None
            };
            let output = required_path(search_args, "--output")?;
            let size = usize_option(search_args, "--size", 16)?;
            let execution = search_execution_config(search_args)?;
            let summary = run_search(&SearchRunConfig {
                segment,
                seed_candidate,
                game: execution.game,
                dvd: execution.dvd,
                output_root: output,
                working_directory: execution.working_directory,
                game_args_prefix: execution.game_args_prefix,
                generations: u32_option(search_args, "--generations", 2)?,
                population_size: size,
                elite_count: usize_option(search_args, "--elites", (size / 4).max(1))?,
                workers: usize_option(search_args, "--workers", 4)?,
                repetitions: u32_option(search_args, "--repetitions", 3)?,
                timeout: execution.timeout,
                rng_seed: u64_option(search_args, "--rng-seed", 1)?,
                harness: execution.harness,
            })?;
            println!("{}", serde_json::to_string_pretty(&summary)?);
            Ok(())
        }
        Some("beam") => {
            let search_args = &args[1..];
            let seed_candidate: Candidate =
                serde_json::from_slice(&fs::read(required_path(search_args, "--candidate")?)?)?;
            seed_candidate.validate()?;
            let options: Vec<huntctl::search::MacroAction> =
                serde_json::from_slice(&fs::read(required_path(search_args, "--options")?)?)?;
            let q_priors: Option<QBeamPriorTable> = option(search_args, "--q-priors")
                .map(|path| {
                    fs::read(path)
                        .map_err(Box::<dyn Error>::from)
                        .and_then(|bytes| {
                            serde_json::from_slice(&bytes).map_err(Box::<dyn Error>::from)
                        })
                })
                .transpose()?;
            let execution = search_execution_config(search_args)?;
            let summary = run_beam_search(&BeamSearchConfig {
                segment: seed_candidate.segment,
                seed_candidate,
                options,
                q_priors,
                game: execution.game,
                dvd: execution.dvd,
                output_root: required_path(search_args, "--output")?,
                working_directory: execution.working_directory,
                game_args_prefix: execution.game_args_prefix,
                beam_width: usize_option(search_args, "--beam-width", 8)?,
                maximum_depth: u32_option(search_args, "--maximum-depth", 8)?,
                candidate_budget: usize_option(search_args, "--candidate-budget", 1_000)?,
                workers: usize_option(search_args, "--workers", 4)?,
                repetitions: u32_option(search_args, "--repetitions", 3)?,
                timeout: execution.timeout,
                harness: execution.harness,
            })?;
            println!("{}", serde_json::to_string_pretty(&summary)?);
            Ok(())
        }
        Some("continuous") => {
            let search_args = &args[1..];
            let seed_candidate: Candidate =
                serde_json::from_slice(&fs::read(required_path(search_args, "--candidate")?)?)?;
            seed_candidate.validate()?;
            let axes: ContinuousAxes =
                serde_json::from_slice(&fs::read(required_path(search_args, "--axes")?)?)?;
            let method: ContinuousMethod = option(search_args, "--method")
                .ok_or("missing required --method cem|cma-es")?
                .parse()?;
            let population_size = usize_option(search_args, "--population", 32)?;
            let execution = search_execution_config(search_args)?;
            let summary = run_continuous_search(&ContinuousSearchRunConfig {
                method,
                seed_candidate,
                axes,
                game: execution.game,
                dvd: execution.dvd,
                output_root: required_path(search_args, "--output")?,
                working_directory: execution.working_directory,
                game_args_prefix: execution.game_args_prefix,
                generations: u32_option(search_args, "--generations", 10)?,
                population_size,
                elite_count: usize_option(search_args, "--elites", (population_size / 4).max(1))?,
                initial_sigma: option(search_args, "--initial-sigma")
                    .map(|value| value.parse())
                    .transpose()?
                    .unwrap_or(0.25),
                candidate_budget: usize_option(search_args, "--candidate-budget", 10_000)?,
                rng_seed: u64_option(search_args, "--rng-seed", 1)?,
                workers: usize_option(search_args, "--workers", 4)?,
                repetitions: u32_option(search_args, "--repetitions", 3)?,
                timeout: execution.timeout,
                harness: execution.harness,
            })?;
            println!("{}", serde_json::to_string_pretty(&summary)?);
            Ok(())
        }
        Some("bayesian") => {
            let search_args = &args[1..];
            let seed_candidate: Candidate =
                serde_json::from_slice(&fs::read(required_path(search_args, "--candidate")?)?)?;
            seed_candidate.validate()?;
            let axes: ContinuousAxes =
                serde_json::from_slice(&fs::read(required_path(search_args, "--axes")?)?)?;
            let parse_f64 = |name: &str, default: f64| -> Result<f64, Box<dyn Error>> {
                Ok(option(search_args, name)
                    .map(|value| value.parse())
                    .transpose()?
                    .unwrap_or(default))
            };
            let execution = search_execution_config(search_args)?;
            let summary = run_bayesian_search(&BayesianSearchRunConfig {
                seed_candidate,
                axes,
                game: execution.game,
                dvd: execution.dvd,
                output_root: required_path(search_args, "--output")?,
                working_directory: execution.working_directory,
                game_args_prefix: execution.game_args_prefix,
                generations: u32_option(search_args, "--generations", 20)?,
                batch_size: usize_option(search_args, "--batch-size", 4)?,
                initial_samples: usize_option(search_args, "--initial-samples", 8)?,
                acquisition_pool: usize_option(search_args, "--acquisition-pool", 2_048)?,
                length_scale: parse_f64("--length-scale", 0.2)?,
                observation_noise: parse_f64("--observation-noise", 1.0e-6)?,
                exploration: parse_f64("--exploration", 0.01)?,
                candidate_budget: usize_option(search_args, "--candidate-budget", 80)?,
                rng_seed: u64_option(search_args, "--rng-seed", 1)?,
                workers: usize_option(search_args, "--workers", 4)?,
                repetitions: u32_option(search_args, "--repetitions", 3)?,
                timeout: execution.timeout,
                harness: execution.harness,
            })?;
            println!("{}", serde_json::to_string_pretty(&summary)?);
            Ok(())
        }
        Some("tournament") => {
            let search_args = &args[1..];
            let definition_path = required_path(search_args, "--definition")?;
            let definition: TournamentDefinition =
                serde_json::from_slice(&fs::read(&definition_path)?)?;
            let definition_directory = fs::canonicalize(
                definition_path
                    .parent()
                    .ok_or("tournament definition has no parent directory")?,
            )?;
            let execution = search_execution_config(search_args)?;
            let anchored = if let Some(prefix) = option(search_args, "--anchored-prefix") {
                Some(AnchoredObjectiveConfig {
                    segment: option(search_args, "--segment")
                        .ok_or("anchored tournament requires --segment ID")?
                        .parse()?,
                    prefix_tape: PathBuf::from(prefix),
                    milestone_program: required_path(search_args, "--milestones")?,
                    game: execution.game.clone(),
                    dvd: execution.dvd.clone(),
                    source_milestone: option(search_args, "--source-milestone")
                        .ok_or("anchored tournament requires --source-milestone NAME")?,
                    source_boundary_fingerprint: option(
                        search_args,
                        "--source-boundary-fingerprint",
                    )
                    .ok_or("anchored tournament requires --source-boundary-fingerprint VALUE")?,
                    goal_milestone: option(search_args, "--goal-milestone")
                        .ok_or("anchored tournament requires --goal-milestone NAME")?,
                })
            } else {
                None
            };
            let summary = run_proposer_tournament(&ProposerTournamentConfig {
                definition,
                definition_directory,
                game: execution.game,
                dvd: execution.dvd,
                output_root: required_path(search_args, "--output")?,
                working_directory: execution.working_directory,
                game_args_prefix: execution.game_args_prefix,
                workers: usize_option(search_args, "--workers", 4)?,
                repetitions: u32_option(search_args, "--repetitions", 3)?,
                timeout: execution.timeout,
                harness: execution.harness,
                anchored,
            })?;
            println!("{}", serde_json::to_string_pretty(&summary)?);
            Ok(())
        }
        Some("prepare-tournament-lane") => {
            let search_args = &args[1..];
            let candidate_path = required_path(search_args, "--candidate")?;
            let candidate: Candidate = serde_json::from_slice(&fs::read(&candidate_path)?)?;
            candidate.validate()?;
            let envelope_path = required_path(search_args, "--proposal-envelopes")?;
            let envelope_value: Value = serde_json::from_slice(&fs::read(&envelope_path)?)?;
            let source_set = if envelope_value.get("schema").and_then(Value::as_str)
                == Some("dusklight-candidate-envelope-set/v1")
            {
                serde_json::from_value::<CandidateEnvelopeSet>(envelope_value)?
            } else {
                let envelopes = serde_json::from_value::<Vec<CandidateEnvelope>>(
                    envelope_value
                        .get("envelopes")
                        .cloned()
                        .ok_or("proposal artifact has no envelopes array")?,
                )?;
                CandidateEnvelopeSet::build(envelopes)?
            };
            source_set.validate()?;
            let candidate_sha256 = candidate.id()?.parse()?;
            let matches = source_set
                .envelopes
                .iter()
                .filter(|envelope| envelope.candidate_sha256 == candidate_sha256)
                .cloned()
                .collect::<Vec<_>>();
            if matches.len() != 1 {
                return Err(format!(
                    "proposal artifact must contain exactly one envelope for candidate {candidate_sha256}, found {}",
                    matches.len()
                )
                .into());
            }
            let envelope_set = CandidateEnvelopeSet::build(matches)?;
            let envelope = &envelope_set.envelopes[0];
            let output = required_path(search_args, "--output")?;
            if output.is_file()
                || output
                    .read_dir()
                    .ok()
                    .is_some_and(|mut entries| entries.next().is_some())
            {
                return Err(format!(
                    "prepared tournament lane output must be new or empty: {}",
                    output.display()
                )
                .into());
            }
            let manifest = write_explicit_population_with_seed(
                &output,
                candidate.segment,
                candidate.ancestry.generation,
                envelope.seed,
                vec![candidate],
            )?;
            fs::write(
                output.join("proposal-envelopes.json"),
                serde_json::to_vec_pretty(&envelope_set)?,
            )?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": "dusklight-prepared-tournament-lane/v1",
                    "manifest": output.join("manifest.json"),
                    "proposal_envelopes": output.join("proposal-envelopes.json"),
                    "candidate_id": manifest.members[0].candidate_id,
                    "proposer": envelope.proposer,
                    "objective": envelope.objective,
                    "action_schema": envelope.action_schema,
                    "charged_candidate_ticks_per_repetition": manifest.members[0].frame_count,
                }))?
            );
            Ok(())
        }
        Some("minimize-route") => {
            let search_args = &args[1..];
            let candidate: Candidate =
                serde_json::from_slice(&fs::read(required_path(search_args, "--candidate")?)?)?;
            candidate.validate()?;
            let execution = search_execution_config(search_args)?;
            let objective = AnchoredObjectiveConfig {
                segment: option(search_args, "--segment")
                    .ok_or("route minimization requires --segment ID")?
                    .parse()?,
                prefix_tape: required_path(search_args, "--anchored-prefix")?,
                milestone_program: required_path(search_args, "--milestones")?,
                game: execution.game,
                dvd: execution.dvd,
                source_milestone: option(search_args, "--source-milestone")
                    .ok_or("route minimization requires --source-milestone NAME")?,
                source_boundary_fingerprint: option(search_args, "--source-boundary-fingerprint")
                    .ok_or(
                    "route minimization requires --source-boundary-fingerprint VALUE",
                )?,
                goal_milestone: option(search_args, "--goal-milestone")
                    .ok_or("route minimization requires --goal-milestone NAME")?,
            };
            let summary = minimize_anchored_route(&AnchoredRouteMinimizeConfig {
                candidate,
                objective,
                output_root: required_path(search_args, "--output")?,
                working_directory: execution.working_directory,
                game_args_prefix: execution.game_args_prefix,
                workers: usize_option(search_args, "--workers", 4)?,
                repetitions: u32_option(search_args, "--repetitions", 3)?,
                candidate_budget: usize_option(search_args, "--candidate-budget", 256)?,
                resume: flag(search_args, "--resume"),
                timeout: execution.timeout,
                harness: execution.harness,
            })?;
            println!("{}", serde_json::to_string_pretty(&summary)?);
            Ok(())
        }
        Some("minimize-boot") => {
            let search_args = &args[1..];
            let candidate: Candidate =
                serde_json::from_slice(&fs::read(required_path(search_args, "--candidate")?)?)?;
            candidate.validate()?;
            let execution = search_execution_config(search_args)?;
            let summary = minimize_boot(&BootMinimizeConfig {
                candidate,
                game: execution.game,
                dvd: execution.dvd,
                output_root: required_path(search_args, "--output")?,
                working_directory: execution.working_directory,
                game_args_prefix: execution.game_args_prefix,
                workers: usize_option(search_args, "--workers", 4)?,
                repetitions: u32_option(search_args, "--repetitions", 3)?,
                timeout: execution.timeout,
                harness: execution.harness,
            })?;
            println!("{}", serde_json::to_string_pretty(&summary)?);
            Ok(())
        }
        Some("golf-inputs") => {
            let search_args = &args[1..];
            let candidate: Candidate =
                serde_json::from_slice(&fs::read(required_path(search_args, "--candidate")?)?)?;
            candidate.validate()?;
            let execution = search_execution_config(search_args)?;
            let objective = AnchoredObjectiveConfig {
                segment: option(search_args, "--segment")
                    .ok_or("input golf requires --segment ID")?
                    .parse()?,
                prefix_tape: required_path(search_args, "--anchored-prefix")?,
                milestone_program: required_path(search_args, "--milestones")?,
                game: execution.game,
                dvd: execution.dvd,
                source_milestone: option(search_args, "--source-milestone")
                    .ok_or("input golf requires --source-milestone NAME")?,
                source_boundary_fingerprint: option(search_args, "--source-boundary-fingerprint")
                    .ok_or(
                    "input golf requires --source-boundary-fingerprint VALUE",
                )?,
                goal_milestone: option(search_args, "--goal-milestone")
                    .ok_or("input golf requires --goal-milestone NAME")?,
            };
            let summary = golf_anchored_inputs(&AnchoredInputGolfConfig {
                candidate,
                objective,
                output_root: required_path(search_args, "--output")?,
                working_directory: execution.working_directory,
                game_args_prefix: execution.game_args_prefix,
                workers: usize_option(search_args, "--workers", 4)?,
                repetitions: u32_option(search_args, "--repetitions", 3)?,
                candidate_budget: usize_option(search_args, "--candidate-budget", 256)?,
                timeout: execution.timeout,
                harness: execution.harness,
            })?;
            println!("{}", serde_json::to_string_pretty(&summary)?);
            Ok(())
        }
        Some("golf-boot") => {
            let search_args = &args[1..];
            let candidate: Candidate =
                serde_json::from_slice(&fs::read(required_path(search_args, "--candidate")?)?)?;
            candidate.validate()?;
            let execution = search_execution_config(search_args)?;
            let summary = golf_boot(&BootGolfConfig {
                candidate,
                game: execution.game,
                dvd: execution.dvd,
                output_root: required_path(search_args, "--output")?,
                working_directory: execution.working_directory,
                game_args_prefix: execution.game_args_prefix,
                workers: usize_option(search_args, "--workers", 4)?,
                repetitions: u32_option(search_args, "--repetitions", 3)?,
                resume: flag(search_args, "--resume"),
                timeout: execution.timeout,
                harness: execution.harness,
            })?;
            println!("{}", serde_json::to_string_pretty(&summary)?);
            Ok(())
        }
        Some("golf-option") => {
            let search_args = &args[1..];
            let plan: RollOptionPlan =
                serde_json::from_slice(&fs::read(required_path(search_args, "--plan")?)?)?;
            let execution: OptionExecution =
                serde_json::from_slice(&fs::read(required_path(search_args, "--execution")?)?)?;
            let tape_path = required_path(search_args, "--tape")?;
            let tape = InputTape::decode(&fs::read(&tape_path)?)?.tape;
            let cancellation_tick = option(search_args, "--cancellation-tick")
                .map(|value| value.parse::<u32>())
                .transpose()?;
            let condition_index = option(search_args, "--condition-index")
                .map(|value| value.parse::<u32>())
                .transpose()?;
            let cancellation = match (cancellation_tick, condition_index) {
                (Some(tick), Some(condition_index)) => Some(RollCancellationHit {
                    tick,
                    condition_index,
                }),
                (None, None) => None,
                _ => {
                    return Err(
                        "--cancellation-tick and --condition-index must be supplied together"
                            .into(),
                    );
                }
            };
            let steps = RollGolfSteps {
                heading_degrees: u16::try_from(u32_option(search_args, "--heading-step", 1)?)?,
                magnitude: u8::try_from(u32_option(search_args, "--magnitude-step", 1)?)?,
                duration_ticks: u32_option(search_args, "--duration-step", 1)?,
                phase_ticks: u32_option(search_args, "--phase-step", 1)?,
                button_ticks: u32_option(search_args, "--button-step", 1)?,
                cancellation_ticks: u32_option(search_args, "--cancellation-step", 1)?,
            };
            let proposals = golf_roll_option(&plan, cancellation, &execution, &tape, steps)?;
            let output = required_path(search_args, "--output")?;
            if let Some(parent) = output.parent() {
                fs::create_dir_all(parent)?;
            }
            let manifest = json!({
                "schema": "dusklight-option-relative-golf-manifest/v1",
                "seed_option_id": execution.option_id,
                "seed_tape": tape_path,
                "steps": steps,
                "proposal_count": proposals.len(),
                "proposals": proposals,
            });
            fs::write(&output, serde_json::to_vec_pretty(&manifest)?)?;
            println!("{}", serde_json::to_string_pretty(&manifest)?);
            Ok(())
        }
        Some("golf-path") => {
            let search_args = &args[1..];
            let plan: MotionPathPlan =
                serde_json::from_slice(&fs::read(required_path(search_args, "--plan")?)?)?;
            let execution: OptionExecution =
                serde_json::from_slice(&fs::read(required_path(search_args, "--execution")?)?)?;
            let tape_path = required_path(search_args, "--tape")?;
            let tape = InputTape::decode(&fs::read(&tape_path)?)?.tape;
            let cancellation_tick = option(search_args, "--cancellation-tick")
                .map(|value| value.parse::<u32>())
                .transpose()?;
            let condition_index = option(search_args, "--condition-index")
                .map(|value| value.parse::<u32>())
                .transpose()?;
            let cancellation = match (cancellation_tick, condition_index) {
                (Some(tick), Some(condition_index)) => Some(PathCancellationHit {
                    tick,
                    condition_index,
                }),
                (None, None) => None,
                _ => {
                    return Err(
                        "--cancellation-tick and --condition-index must be supplied together"
                            .into(),
                    );
                }
            };
            let steps = MotionPathGolfSteps {
                point_units: u16::try_from(u32_option(search_args, "--point-step", 1)?)?,
                duration_ticks: u32_option(search_args, "--duration-step", 1)?,
                phase_units: u32_option(search_args, "--phase-step", 1)?,
                cancellation_ticks: u32_option(search_args, "--cancellation-step", 1)?,
            };
            let proposals = golf_motion_path(&plan, cancellation, &execution, &tape, steps)?;
            let output = required_path(search_args, "--output")?;
            if output.exists() {
                return Err(
                    format!("path-golf output already exists: {}", output.display()).into(),
                );
            }
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            let manifest = json!({
                "schema": "dusklight-motion-path-relative-golf-manifest/v1",
                "seed_option_id": execution.option_id,
                "seed_tape": tape_path,
                "steps": steps,
                "proposal_count": proposals.len(),
                "proposals": proposals,
            });
            fs::write(&output, serde_json::to_vec_pretty(&manifest)?)?;
            println!("{}", serde_json::to_string_pretty(&manifest)?);
            Ok(())
        }
        Some("import-tape") => {
            let search_args = &args[1..];
            let segment: SegmentProfile = option(search_args, "--segment")
                .ok_or("missing required --segment ID")?
                .parse()?;
            let tape_path = required_path(search_args, "--tape")?;
            let output = required_path(search_args, "--output")?;
            let tape = InputTape::decode(&fs::read(&tape_path)?)?.tape;
            let candidate = Candidate::from_absolute_tape(segment, &tape)?;
            if let Some(parent) = output.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&output, serde_json::to_vec_pretty(&candidate)?)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "candidate_id": candidate.id()?,
                    "candidate": output,
                    "source_tape": tape_path,
                    "frames": candidate.frame_count(),
                    "lossless": candidate.compile()? == tape,
                }))?
            );
            Ok(())
        }
        Some("seed") => {
            let search_args = &args[1..];
            let segment: SegmentProfile = option(search_args, "--segment")
                .ok_or("missing required --segment ID")?
                .parse()?;
            let output = required_path(search_args, "--output")?;
            let size = usize_option(search_args, "--size", 16)?;
            let rng_seed = u64_option(search_args, "--rng-seed", 1)?;
            let candidate = if let Some(path) = option(search_args, "--candidate") {
                let candidate: Candidate = serde_json::from_slice(&fs::read(path)?)?;
                if candidate.segment != segment {
                    return Err("candidate segment does not match --segment".into());
                }
                candidate
            } else {
                Candidate::baseline(segment)
            };
            let manifest = write_seed_population(&output, candidate, size, rng_seed)?;
            println!("{}", serde_json::to_string_pretty(&manifest)?);
            Ok(())
        }
        Some("evolve") => {
            let search_args = &args[1..];
            let population = required_path(search_args, "--population")?;
            let results: SearchResults =
                serde_json::from_slice(&fs::read(required_path(search_args, "--results")?)?)?;
            let output = required_path(search_args, "--output")?;
            let size = usize_option(search_args, "--size", 16)?;
            let elites = usize_option(search_args, "--elites", (size / 4).max(1))?;
            let rng_seed = u64_option(search_args, "--rng-seed", 1)?;
            let manifest = evolve_population(
                &population,
                &results,
                &output,
                EvolutionConfig {
                    population_size: size,
                    elite_count: elites,
                    rng_seed,
                },
            )?;
            println!("{}", serde_json::to_string_pretty(&manifest)?);
            Ok(())
        }
        Some("rank") => {
            let search_args = &args[1..];
            let manifest: PopulationManifest =
                serde_json::from_slice(&fs::read(required_path(search_args, "--population")?)?)?;
            let results: SearchResults =
                serde_json::from_slice(&fs::read(required_path(search_args, "--results")?)?)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&rank_population(&manifest, &results)?)?
            );
            Ok(())
        }
        Some("collect") => {
            let search_args = &args[1..];
            let manifest: PopulationManifest =
                serde_json::from_slice(&fs::read(required_path(search_args, "--population")?)?)?;
            let inputs = repeated_option(search_args, "--input");
            if inputs.is_empty() {
                return Err("search collect requires at least one --input FILE".into());
            }
            let artifacts = inputs
                .iter()
                .map(|path| serde_json::from_slice(&fs::read(path)?).map_err(Into::into))
                .collect::<Result<Vec<EvaluationArtifact>, Box<dyn Error>>>()?;
            let results = collect_results(&manifest, artifacts)?;
            let output = required_path(search_args, "--output")?;
            fs::write(&output, serde_json::to_vec_pretty(&results)?)?;
            println!("{}", serde_json::to_string_pretty(&results)?);
            Ok(())
        }
        Some("inspect") if args.len() == 2 => {
            let candidate: Candidate = serde_json::from_slice(&fs::read(&args[1])?)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "candidate_id": candidate.id()?,
                    "segment": candidate.segment,
                    "target": candidate.segment.target(),
                    "target_depth": candidate.segment.target_depth(),
                    "action_count": candidate.actions.len(),
                    "frame_count": candidate.frame_count(),
                    "ancestry": candidate.ancestry,
                }))?
            );
            Ok(())
        }
        Some("mock-evaluate") => {
            let search_args = &args[1..];
            let population_path = required_path(search_args, "--population")?;
            let output = required_path(search_args, "--output")?;
            let attempts = u32::try_from(usize_option(search_args, "--attempts", 3)?)?;
            if attempts == 0 {
                return Err("--attempts must be greater than zero".into());
            }
            let manifest: PopulationManifest = serde_json::from_slice(&fs::read(population_path)?)?;
            let candidates = manifest
                .members
                .iter()
                .map(|member| {
                    (
                        member.candidate_id.clone(),
                        CandidateResult {
                            goal_reached: Some(true),
                            milestone_depth: manifest.segment.target_depth(),
                            attempts,
                            successes: attempts,
                            first_hit_ticks: vec![member.frame_count; attempts as usize],
                            risk_events: None,
                            boundary_compatibility: huntctl::search::BoundaryCompatibility::Unknown,
                        },
                    )
                })
                .collect();
            let results = SearchResults {
                schema: RESULTS_SCHEMA.into(),
                segment: manifest.segment,
                boot: manifest.boot,
                candidates,
            };
            fs::write(&output, serde_json::to_vec_pretty(&results)?)?;
            println!("{}", serde_json::to_string_pretty(&results)?);
            Ok(())
        }
        _ => usage_error(),
    }
}
