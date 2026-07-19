//! Search, evaluation, optimizer, and tournament command adapters.

use crate::*;

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
            let source_path = artifact_root.join(
                timeline
                    .predicate_program
                    .as_ref()
                    .ok_or("route search requires predicate_program")?,
            );
            let compiled = milestone_dsl::compile_source(&fs::read_to_string(&source_path)?)?;
            let program_path = objective_root.join("milestones.dmsp");
            fs::write(&program_path, &compiled.bytes)?;

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
