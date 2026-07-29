use super::*;

pub(super) fn command_optimization(command: &str, args: &[String]) -> Result<(), Box<dyn Error>> {
    match command {
        "golf-route-inputs" => {
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
            let mut execution = search_execution_config(search_args)?;
            bind_route_origin_card_fixture(&timeline, &prefix.tape.boot, &mut execution)?;

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
        "run" => {
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
        "beam" => {
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
        "continuous" => {
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
            let objective = option(search_args, "--anchored-prefix")
                .map(|prefix| -> Result<AnchoredObjectiveConfig, Box<dyn Error>> {
                    Ok(AnchoredObjectiveConfig {
                        segment: seed_candidate.segment,
                        prefix_tape: PathBuf::from(prefix),
                        milestone_program: required_path(search_args, "--milestones")?,
                        game: execution.game.clone(),
                        dvd: execution.dvd.clone(),
                        source_milestone: option(search_args, "--source-milestone")
                            .ok_or(
                                "anchored continuous search requires --source-milestone NAME",
                            )?,
                        source_boundary_fingerprint: option(
                            search_args,
                            "--source-boundary-fingerprint",
                        )
                        .ok_or(
                            "anchored continuous search requires --source-boundary-fingerprint VALUE",
                        )?,
                        goal_milestone: option(search_args, "--goal-milestone")
                            .ok_or("anchored continuous search requires --goal-milestone NAME")?,
                    })
                })
                .transpose()?;
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
                objective,
            })?;
            println!("{}", serde_json::to_string_pretty(&summary)?);
            Ok(())
        }
        "bayesian" => {
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
        "tournament" => {
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
        "prepare-tournament-lane" => {
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
        "minimize-route" => {
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
        "minimize-boot" => {
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
        "golf-inputs" => {
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
        "golf-boot" => {
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
        "golf-option" => {
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
        "golf-path" => {
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
        _ => usage_error(),
    }
}
