use super::*;

pub(super) fn command_execution(command: &str, args: &[String]) -> Result<(), Box<dyn Error>> {
    match command {
        "evaluate" => {
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
        "run-route" => {
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
            let mut execution = search_execution_config(search_args)?;
            bind_route_origin_card_fixture(&timeline, &prefix.tape.boot, &mut execution)?;
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
                    game_args_prefix: execution.game_args_prefix,
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
        _ => usage_error(),
    }
}
