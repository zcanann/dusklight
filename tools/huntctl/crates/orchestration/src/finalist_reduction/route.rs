//! Exact anchored-route reduction through repeated clean-boot evaluation.

use super::*;

#[derive(Clone)]
struct ProvenRouteCandidate {
    candidate: Candidate,
    tape: InputTape,
    first_hit_tick: u64,
    goal_sim_tick: u64,
    goal_tape_frame: u64,
    goal_boundary_fingerprint: BoundaryFingerprint,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct RouteReductionTarget {
    first_hit_tick: u64,
    goal_sim_tick: u64,
    goal_tape_frame: u64,
    goal_boundary_fingerprint: BoundaryFingerprint,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "phase")]
enum RouteMinimizePhase {
    Trim,
    Actions { granularity: usize },
    Durations,
    FinalProof,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RouteMinimizeCheckpoint {
    schema: String,
    objective: AnchoredObjectiveIdentity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    harness_request_sha256: Option<ArtifactDigest>,
    source_candidate_id: String,
    candidate_budget: usize,
    target: RouteReductionTarget,
    retained_candidate: Candidate,
    history: Vec<AnchoredRouteMinimizeRound>,
    proposal_evaluations: usize,
    accepted_reductions: usize,
    next_round: u32,
    phase: RouteMinimizePhase,
}

impl RouteReductionTarget {
    fn accepts(&self, candidate: &ProvenRouteCandidate) -> bool {
        candidate.first_hit_tick == self.first_hit_tick
            && candidate.goal_sim_tick == self.goal_sim_tick
            && candidate.goal_tape_frame == self.goal_tape_frame
            && candidate.goal_boundary_fingerprint == self.goal_boundary_fingerprint
    }
}

pub fn minimize_anchored_route(
    config: &AnchoredRouteMinimizeConfig,
) -> Result<AnchoredRouteMinimizeSummary, EvaluateError> {
    config.candidate.validate()?;
    let output_nonempty = directory_is_nonempty(&config.output_root)?;
    if config.candidate.segment != config.objective.segment
        || !is_anchored_profile(config.candidate.segment)
        || !config.working_directory.is_dir()
        || config.workers == 0
        || config.repetitions < 2
        || config.candidate_budget == 0
        || config.timeout.is_zero()
        || (!config.resume && output_nonempty)
        || (config.resume && !output_nonempty)
        || (config.resume && config.output_root.join("minimize.summary.json").exists())
    {
        return Err(EvaluateError::InvalidConfig(
            "anchored route minimization requires a matching movement candidate, at least two repetitions, positive bounded execution limits, and either a new output root or --resume with an incomplete checkpointed root"
                .into(),
        ));
    }
    let prepared = prepare_anchored_evaluator(&config.objective)?;
    let objective = prepared.identity().clone();
    validate_anchored_harness(config.harness.as_ref(), &objective)?;
    fs::create_dir_all(&config.output_root)?;

    let source_id = config.candidate.id()?;
    let source_tape = config.candidate.compile()?;
    let source_frames = config.candidate.frame_count();
    let source_actions = config.candidate.actions.len();
    let source_input_complexity = tape_input_complexity(&source_tape);
    let source_root = fresh_evidence_root(&config.output_root, "source", config.resume)?;
    let (source_candidates, source_report) = evaluate_route_batch(
        config,
        &prepared,
        vec![config.candidate.clone()],
        &source_root,
        0,
    )?;
    let source = source_candidates.into_iter().next().ok_or_else(|| {
        EvaluateError::InvalidResult(
            "the anchored minimization source did not prove the exact authored goal in every repetition"
                .into(),
        )
    })?;
    let target = RouteReductionTarget {
        first_hit_tick: source.first_hit_tick,
        goal_sim_tick: source.goal_sim_tick,
        goal_tape_frame: source.goal_tape_frame,
        goal_boundary_fingerprint: source.goal_boundary_fingerprint.clone(),
    };

    let (
        mut current,
        mut history,
        mut proposal_evaluations,
        mut accepted_reductions,
        mut round,
        mut phase,
        proof_evaluations,
    ) = if config.resume {
        let checkpoint = read_checkpoint(config, &objective, &source_id, &target)?;
        let retained_root = fresh_evidence_root(&config.output_root, "resume-retained", true)?;
        let (proved, _) = evaluate_route_batch(
            config,
            &prepared,
            vec![checkpoint.retained_candidate.clone()],
            &retained_root,
            checkpoint.next_round,
        )?;
        let current = proved
            .into_iter()
            .find(|candidate| target.accepts(candidate))
            .ok_or_else(|| {
                EvaluateError::InvalidResult(
                    "checkpointed retained route failed exact resume proof".into(),
                )
            })?;
        (
            current,
            checkpoint.history,
            checkpoint.proposal_evaluations,
            checkpoint.accepted_reductions,
            checkpoint.next_round,
            checkpoint.phase,
            3_usize,
        )
    } else {
        (
            source,
            Vec::new(),
            0_usize,
            0_usize,
            1_u32,
            RouteMinimizePhase::Trim,
            2_usize,
        )
    };
    write_checkpoint(
        config,
        &objective,
        &source_id,
        &target,
        &current,
        &history,
        proposal_evaluations,
        accepted_reductions,
        round,
        &phase,
    )?;

    let prefix = InputTape::decode(&fs::read(&config.objective.prefix_tape)?)?.tape;
    let required_suffix_frames = target
        .goal_tape_frame
        .checked_add(1)
        .and_then(|end| end.checked_sub(prefix.frames.len() as u64))
        .ok_or_else(|| {
            EvaluateError::InvalidResult(
                "anchored goal tape frame does not lie after the immutable prefix".into(),
            )
        })?;
    if phase == RouteMinimizePhase::Trim {
        if required_suffix_frames < current.candidate.frame_count()
            && proposal_evaluations < config.candidate_budget
        {
            let mut trimmed_tape = current.tape.clone();
            trimmed_tape
                .frames
                .truncate(required_suffix_frames as usize);
            let mut trimmed =
                Candidate::from_absolute_tape(current.candidate.segment, &trimmed_tape)?;
            trimmed.ancestry = Ancestry {
                generation: round,
                parent_id: Some(current.candidate.id()?),
                mutation: Some("trim after exact anchored goal tape frame".into()),
                intervention: Some(InterventionRange {
                    start_frame: required_suffix_frames,
                    end_frame_exclusive: required_suffix_frames,
                    parent_end_frame_exclusive: current.candidate.frame_count(),
                }),
            };
            let round_root = fresh_round_root(&config.output_root, round)?;
            let (evaluated, _) =
                evaluate_route_batch(config, &prepared, vec![trimmed], &round_root, round)?;
            proposal_evaluations += 1;
            let accepted = best_reduction(&current, evaluated, &target)?;
            let was_accepted = accepted.is_some();
            if let Some(candidate) = accepted {
                current = candidate;
                accepted_reductions += 1;
            }
            history.push(round_record(
                round,
                "trim_after_goal",
                1,
                &current,
                was_accepted,
            )?);
            round += 1;
        }
        phase = RouteMinimizePhase::Actions { granularity: 2 };
        write_checkpoint(
            config,
            &objective,
            &source_id,
            &target,
            &current,
            &history,
            proposal_evaluations,
            accepted_reductions,
            round,
            &phase,
        )?;
    }

    let mut granularity = match phase {
        RouteMinimizePhase::Actions { granularity } => granularity,
        _ => 2,
    };
    while matches!(phase, RouteMinimizePhase::Actions { .. })
        && current.candidate.actions.len() > 1
        && proposal_evaluations < config.candidate_budget
    {
        let partitions = granularity.min(current.candidate.actions.len());
        let mut proposals = action_partition_removals(&current.candidate, partitions, round)?;
        proposals.truncate(config.candidate_budget - proposal_evaluations);
        if proposals.is_empty() {
            phase = RouteMinimizePhase::Durations;
            write_checkpoint(
                config,
                &objective,
                &source_id,
                &target,
                &current,
                &history,
                proposal_evaluations,
                accepted_reductions,
                round,
                &phase,
            )?;
            break;
        }
        let evaluated_count = proposals.len();
        let round_root = fresh_round_root(&config.output_root, round)?;
        let (evaluated, _) =
            evaluate_route_batch(config, &prepared, proposals, &round_root, round)?;
        proposal_evaluations += evaluated_count;
        let accepted = best_reduction(&current, evaluated, &target)?;
        let accepted_id = accepted
            .as_ref()
            .map(|candidate| candidate.candidate.id())
            .transpose()?;
        if let Some(candidate) = accepted {
            current = candidate;
            accepted_reductions += 1;
            granularity = 2;
            phase = RouteMinimizePhase::Actions { granularity };
        } else if partitions == current.candidate.actions.len() {
            phase = RouteMinimizePhase::Durations;
        } else {
            granularity = (partitions * 2).min(current.candidate.actions.len());
            phase = RouteMinimizePhase::Actions { granularity };
        }
        history.push(round_record_with_id(
            round,
            "remove_action_partitions",
            evaluated_count,
            &current,
            accepted_id,
        )?);
        round += 1;
        write_checkpoint(
            config,
            &objective,
            &source_id,
            &target,
            &current,
            &history,
            proposal_evaluations,
            accepted_reductions,
            round,
            &phase,
        )?;
    }

    if matches!(phase, RouteMinimizePhase::Actions { .. }) {
        phase = RouteMinimizePhase::Durations;
        write_checkpoint(
            config,
            &objective,
            &source_id,
            &target,
            &current,
            &history,
            proposal_evaluations,
            accepted_reductions,
            round,
            &phase,
        )?;
    }

    while phase == RouteMinimizePhase::Durations && proposal_evaluations < config.candidate_budget {
        let mut proposals = duration_reductions(&current.candidate, round)?;
        proposals.truncate(config.candidate_budget - proposal_evaluations);
        if proposals.is_empty() {
            phase = RouteMinimizePhase::FinalProof;
            write_checkpoint(
                config,
                &objective,
                &source_id,
                &target,
                &current,
                &history,
                proposal_evaluations,
                accepted_reductions,
                round,
                &phase,
            )?;
            break;
        }
        let evaluated_count = proposals.len();
        let round_root = fresh_round_root(&config.output_root, round)?;
        let (evaluated, _) =
            evaluate_route_batch(config, &prepared, proposals, &round_root, round)?;
        proposal_evaluations += evaluated_count;
        let accepted = best_reduction(&current, evaluated, &target)?;
        let accepted_id = accepted
            .as_ref()
            .map(|candidate| candidate.candidate.id())
            .transpose()?;
        if let Some(candidate) = accepted {
            current = candidate;
            accepted_reductions += 1;
        }
        let no_acceptance = accepted_id.is_none();
        if no_acceptance {
            phase = RouteMinimizePhase::FinalProof;
        }
        history.push(round_record_with_id(
            round,
            "reduce_action_duration",
            evaluated_count,
            &current,
            accepted_id,
        )?);
        round += 1;
        write_checkpoint(
            config,
            &objective,
            &source_id,
            &target,
            &current,
            &history,
            proposal_evaluations,
            accepted_reductions,
            round,
            &phase,
        )?;
        if no_acceptance {
            break;
        }
    }

    if phase == RouteMinimizePhase::Durations {
        phase = RouteMinimizePhase::FinalProof;
        write_checkpoint(
            config,
            &objective,
            &source_id,
            &target,
            &current,
            &history,
            proposal_evaluations,
            accepted_reductions,
            round,
            &phase,
        )?;
    }

    let final_root = fresh_evidence_root(&config.output_root, "final-proof", config.resume)?;
    let (final_candidates, final_report) = evaluate_route_batch(
        config,
        &prepared,
        vec![current.candidate.clone()],
        &final_root,
        round,
    )?;
    let minimized = final_candidates
        .into_iter()
        .find(|candidate| target.accepts(candidate))
        .ok_or_else(|| {
            EvaluateError::InvalidResult(
                "the minimized anchored route failed its independent exact final proof".into(),
            )
        })?;

    let candidate_path = config.output_root.join("minimized.candidate.json");
    let suffix_tape_path = config.output_root.join("minimized.suffix.tape");
    let realized_tape_path = config.output_root.join("minimized.tape");
    let history_path = config.output_root.join("reduction-history.json");
    fs::write(
        &candidate_path,
        serde_json::to_vec_pretty(&minimized.candidate)?,
    )?;
    fs::write(&suffix_tape_path, minimized.tape.encode()?)?;
    let realized = concatenate(vec![
        ChainSegment::all(prefix),
        ChainSegment::all(minimized.tape.clone()),
    ])
    .map_err(|error| EvaluateError::InvalidResult(error.to_string()))?
    .tape;
    fs::write(&realized_tape_path, realized.encode()?)?;
    write_json(&history_path, &history)?;

    let minimized_input_complexity = tape_input_complexity(&minimized.tape);
    let summary = AnchoredRouteMinimizeSummary {
        schema: "dusklight-anchored-route-minimization/v1",
        objective,
        source_candidate_id: source_id,
        minimized_candidate_id: minimized.candidate.id()?,
        source_frames,
        minimized_frames: minimized.candidate.frame_count(),
        source_actions,
        minimized_actions: minimized.candidate.actions.len(),
        source_input_complexity,
        minimized_input_complexity,
        goal_first_hit_tick: target.first_hit_tick,
        goal_sim_tick: target.goal_sim_tick,
        goal_tape_frame: target.goal_tape_frame,
        goal_boundary_fingerprint: target.goal_boundary_fingerprint.digest,
        evaluated_candidates: proposal_evaluations + proof_evaluations,
        accepted_reductions,
        candidate: candidate_path,
        suffix_tape: suffix_tape_path,
        realized_tape: realized_tape_path,
        source_proof: source_report.results,
        final_proof: final_report.results,
        reduction_history: history_path,
        output_root: config.output_root.clone(),
    };
    write_json(&config.output_root.join("minimize.summary.json"), &summary)?;
    Ok(summary)
}

fn phase_order(phase: &RouteMinimizePhase) -> u8 {
    match phase {
        RouteMinimizePhase::Trim => 0,
        RouteMinimizePhase::Actions { .. } => 1,
        RouteMinimizePhase::Durations => 2,
        RouteMinimizePhase::FinalProof => 3,
    }
}

#[allow(clippy::too_many_arguments)]
fn write_checkpoint(
    config: &AnchoredRouteMinimizeConfig,
    objective: &AnchoredObjectiveIdentity,
    source_candidate_id: &str,
    target: &RouteReductionTarget,
    retained: &ProvenRouteCandidate,
    history: &[AnchoredRouteMinimizeRound],
    proposal_evaluations: usize,
    accepted_reductions: usize,
    next_round: u32,
    phase: &RouteMinimizePhase,
) -> Result<(), EvaluateError> {
    let checkpoint = RouteMinimizeCheckpoint {
        schema: "dusklight-anchored-route-minimization-checkpoint/v2".into(),
        objective: objective.clone(),
        harness_request_sha256: config
            .harness
            .as_ref()
            .map(|harness| harness.request_template.content_sha256),
        source_candidate_id: source_candidate_id.into(),
        candidate_budget: config.candidate_budget,
        target: target.clone(),
        retained_candidate: retained.candidate.clone(),
        history: history.to_vec(),
        proposal_evaluations,
        accepted_reductions,
        next_round,
        phase: phase.clone(),
    };
    validate_checkpoint(config, objective, source_candidate_id, target, &checkpoint)?;
    let directory = config.output_root.join("checkpoints");
    fs::create_dir_all(&directory)?;
    let name = format!(
        "{:08}-{}-{:08}-{:08}.json",
        history.len(),
        phase_order(phase),
        proposal_evaluations,
        next_round,
    );
    let destination = directory.join(&name);
    let bytes = serde_json::to_vec_pretty(&checkpoint)?;
    if destination.exists() {
        if fs::read(&destination)? != bytes {
            return Err(EvaluateError::InvalidResult(format!(
                "checkpoint destination changed: {}",
                destination.display()
            )));
        }
        return Ok(());
    }
    let temporary = directory.join(format!(".{name}.{}.tmp", std::process::id()));
    fs::write(&temporary, &bytes)?;
    fs::rename(&temporary, &destination)?;
    Ok(())
}

fn read_checkpoint(
    config: &AnchoredRouteMinimizeConfig,
    objective: &AnchoredObjectiveIdentity,
    source_candidate_id: &str,
    target: &RouteReductionTarget,
) -> Result<RouteMinimizeCheckpoint, EvaluateError> {
    let directory = config.output_root.join("checkpoints");
    let mut paths = fs::read_dir(&directory)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()?;
    paths.retain(|path| {
        path.extension()
            .is_some_and(|extension| extension == "json")
    });
    paths.sort();
    let path = paths.pop().ok_or_else(|| {
        EvaluateError::InvalidConfig(format!(
            "resume root has no completed checkpoint: {}",
            directory.display()
        ))
    })?;
    let checkpoint: RouteMinimizeCheckpoint = serde_json::from_slice(&fs::read(&path)?)?;
    validate_checkpoint(config, objective, source_candidate_id, target, &checkpoint)?;
    Ok(checkpoint)
}

fn validate_checkpoint(
    config: &AnchoredRouteMinimizeConfig,
    objective: &AnchoredObjectiveIdentity,
    source_candidate_id: &str,
    target: &RouteReductionTarget,
    checkpoint: &RouteMinimizeCheckpoint,
) -> Result<(), EvaluateError> {
    checkpoint.retained_candidate.validate()?;
    let expected_next_round = checkpoint
        .history
        .last()
        .map_or(1, |record| record.round.saturating_add(1));
    let evaluated = checkpoint
        .history
        .iter()
        .try_fold(0_usize, |sum, record| {
            sum.checked_add(record.evaluated_candidates)
        })
        .ok_or_else(|| EvaluateError::InvalidResult("checkpoint budget overflowed".into()))?;
    let accepted = checkpoint
        .history
        .iter()
        .filter(|record| record.accepted_candidate_id.is_some())
        .count();
    let rounds_are_contiguous = checkpoint
        .history
        .iter()
        .enumerate()
        .all(|(index, record)| record.round == index as u32 + 1);
    let retained_tape = checkpoint.retained_candidate.compile()?;
    let retained_id = checkpoint.retained_candidate.id()?;
    let expected_retained_id = checkpoint
        .history
        .iter()
        .rev()
        .find_map(|record| record.accepted_candidate_id.as_deref())
        .unwrap_or(source_candidate_id);
    let retained_matches = checkpoint.history.last().is_none_or(|record| {
        record.retained_frames == checkpoint.retained_candidate.frame_count()
            && record.retained_actions == checkpoint.retained_candidate.actions.len()
            && record.retained_input_complexity == tape_input_complexity(&retained_tape)
    }) && retained_id == expected_retained_id;
    let history_phases_are_ordered = checkpoint.history.iter().try_fold(
        (false, false),
        |(saw_actions, saw_durations), record| match record.operation.as_str() {
            "trim_after_goal" if record.round == 1 && !saw_actions && !saw_durations => {
                Some((false, false))
            }
            "remove_action_partitions" if !saw_durations => Some((true, false)),
            "reduce_action_duration" => Some((saw_actions, true)),
            _ => None,
        },
    );
    let phase_is_valid = match checkpoint.phase {
        RouteMinimizePhase::Trim => checkpoint.history.is_empty() && evaluated == 0,
        RouteMinimizePhase::Actions { granularity } => {
            granularity >= 2
                && granularity <= checkpoint.retained_candidate.actions.len().max(2)
                && history_phases_are_ordered.is_some_and(|(_, saw_durations)| !saw_durations)
        }
        RouteMinimizePhase::Durations | RouteMinimizePhase::FinalProof => {
            history_phases_are_ordered.is_some()
        }
    };
    let expected_harness_request = config
        .harness
        .as_ref()
        .map(|harness| harness.request_template.content_sha256);
    if !checkpoint_harness_is_valid(
        &checkpoint.schema,
        checkpoint.harness_request_sha256,
        expected_harness_request,
    ) || &checkpoint.objective != objective
        || checkpoint.source_candidate_id != source_candidate_id
        || checkpoint.candidate_budget != config.candidate_budget
        || &checkpoint.target != target
        || checkpoint.retained_candidate.segment != config.objective.segment
        || checkpoint.proposal_evaluations != evaluated
        || checkpoint.proposal_evaluations > config.candidate_budget
        || checkpoint.accepted_reductions != accepted
        || checkpoint.next_round != expected_next_round
        || !rounds_are_contiguous
        || !retained_matches
        || !phase_is_valid
    {
        return Err(EvaluateError::InvalidResult(
            "route minimization checkpoint is stale, inconsistent, or tampered".into(),
        ));
    }
    Ok(())
}

fn checkpoint_harness_is_valid(
    schema: &str,
    checkpoint_request: Option<ArtifactDigest>,
    expected_request: Option<ArtifactDigest>,
) -> bool {
    match schema {
        "dusklight-anchored-route-minimization-checkpoint/v2" => {
            checkpoint_request == expected_request
        }
        "dusklight-anchored-route-minimization-checkpoint/v1" => {
            checkpoint_request.is_none() && expected_request.is_none()
        }
        _ => false,
    }
}

fn validate_anchored_harness(
    harness: Option<&HarnessEvaluateConfig>,
    objective: &AnchoredObjectiveIdentity,
) -> Result<(), EvaluateError> {
    let Some(harness) = harness else {
        return Ok(());
    };
    if harness.request_template.objective.goal != objective.goal_milestone
        || harness
            .request_template
            .objective
            .program_sha256
            .to_string()
            != objective.milestone_program_sha256
    {
        return Err(EvaluateError::InvalidConfig(
            "anchored route minimization run request must bind the exact goal and milestone program"
                .into(),
        ));
    }
    Ok(())
}

fn fresh_evidence_root(
    output_root: &Path,
    label: &str,
    resume: bool,
) -> Result<PathBuf, EvaluateError> {
    let base = output_root.join(label);
    if !base.exists() {
        return Ok(base);
    }
    if !resume {
        return Err(EvaluateError::InvalidConfig(format!(
            "evidence root already exists: {}",
            base.display()
        )));
    }
    for attempt in 1..=10_000_u32 {
        let candidate = output_root.join(format!("{label}-resume-{attempt:04}"));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(EvaluateError::InvalidConfig(
        "too many resume evidence roots".into(),
    ))
}

fn fresh_round_root(output_root: &Path, round: u32) -> Result<PathBuf, EvaluateError> {
    let rounds = output_root.join("rounds");
    let base = rounds.join(format!("{round:04}"));
    if !base.exists() {
        return Ok(base);
    }
    for attempt in 1..=10_000_u32 {
        let candidate = rounds.join(format!("{round:04}-resume-{attempt:04}"));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(EvaluateError::InvalidConfig(
        "too many resumed round evidence roots".into(),
    ))
}

fn evaluate_route_batch(
    config: &AnchoredRouteMinimizeConfig,
    prepared: &PreparedAnchoredEvaluator,
    candidates: Vec<Candidate>,
    root: &Path,
    generation: u32,
) -> Result<(Vec<ProvenRouteCandidate>, EvaluationReport), EvaluateError> {
    let population_root = root.join("population");
    let manifest = write_explicit_population(
        &population_root,
        config.objective.segment,
        generation,
        candidates.clone(),
    )?;
    let (report, results) = evaluate_prepared_anchored_population(
        &AnchoredEvaluateConfig {
            evaluation: EvaluateConfig {
                population_path: population_root.join("manifest.json"),
                game: config.objective.game.clone(),
                dvd: config.objective.dvd.clone(),
                output_root: root.join("evidence"),
                results_path: root.join("results.json"),
                working_directory: config.working_directory.clone(),
                game_args_prefix: config.game_args_prefix.clone(),
                workers: config.workers,
                repetitions: config.repetitions,
                timeout: config.timeout,
                harness: config.harness.clone(),
            },
            objective: config.objective.clone(),
        },
        prepared,
    )?;
    if &results.objective != prepared.identity() {
        return Err(EvaluateError::InvalidResult(
            "anchored route minimization changed objective identity".into(),
        ));
    }
    let mut proven = Vec::new();
    for candidate in candidates {
        let id = candidate.id()?;
        let attempts = report
            .attempts
            .iter()
            .filter(|attempt| attempt.candidate_id == id)
            .collect::<Vec<_>>();
        if attempts.len() != config.repetitions as usize
            || !attempts.iter().all(|attempt| {
                attempt.goal_reached
                    && attempt.infrastructure_error.is_none()
                    && attempt.first_hit_tick.is_some()
            })
        {
            continue;
        }
        let reference = attempts[0];
        let observation = reference
            .milestone_observations
            .get(&config.objective.goal_milestone)
            .ok_or_else(|| {
                EvaluateError::InvalidResult(format!(
                    "successful anchored candidate {id} has no goal observation"
                ))
            })?;
        let fingerprint = reference
            .boundary_fingerprints
            .get(&config.objective.goal_milestone)
            .ok_or_else(|| {
                EvaluateError::InvalidResult(format!(
                    "successful anchored candidate {id} has no goal boundary fingerprint"
                ))
            })?
            .clone();
        if attempts.iter().any(|attempt| {
            attempt.first_hit_tick != reference.first_hit_tick
                || attempt
                    .milestone_observations
                    .get(&config.objective.goal_milestone)
                    != Some(observation)
                || attempt
                    .boundary_fingerprints
                    .get(&config.objective.goal_milestone)
                    != Some(&fingerprint)
        }) {
            return Err(EvaluateError::InvalidResult(format!(
                "anchored route candidate {id} disagreed across repetitions"
            )));
        }
        proven.push(ProvenRouteCandidate {
            tape: candidate.compile()?,
            candidate,
            first_hit_tick: reference.first_hit_tick.expect("checked above"),
            goal_sim_tick: observation.sim_tick,
            goal_tape_frame: observation.tape_frame,
            goal_boundary_fingerprint: fingerprint,
        });
    }
    debug_assert_eq!(
        manifest.members.len(),
        report.planned_attempts / config.repetitions as usize
    );
    Ok((proven, report))
}

fn best_reduction(
    current: &ProvenRouteCandidate,
    candidates: Vec<ProvenRouteCandidate>,
    target: &RouteReductionTarget,
) -> Result<Option<ProvenRouteCandidate>, EvaluateError> {
    let current_quality = reduction_quality(&current.candidate)?;
    let mut eligible = candidates
        .into_iter()
        .filter(|candidate| target.accepts(candidate))
        .map(|candidate| {
            reduction_quality(&candidate.candidate).map(|quality| (quality, candidate))
        })
        .collect::<Result<Vec<_>, _>>()?;
    eligible.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(eligible
        .into_iter()
        .find(|(quality, _)| quality < &current_quality)
        .map(|(_, candidate)| candidate))
}

fn reduction_quality(candidate: &Candidate) -> Result<(u64, u64, usize, String), EvaluateError> {
    let tape = candidate.compile()?;
    Ok((
        candidate.frame_count(),
        tape_input_complexity(&tape),
        candidate.actions.len(),
        candidate.id()?,
    ))
}

fn action_partition_removals(
    source: &Candidate,
    partitions: usize,
    generation: u32,
) -> Result<Vec<Candidate>, EvaluateError> {
    let mut candidates = Vec::new();
    let mut ids = BTreeSet::new();
    for partition in 0..partitions {
        let start = source.actions.len() * partition / partitions;
        let end = source.actions.len() * (partition + 1) / partitions;
        if start == end || end - start == source.actions.len() {
            continue;
        }
        let mut candidate = source.clone();
        candidate.actions.drain(start..end);
        candidate.ancestry = Ancestry {
            generation,
            parent_id: Some(source.id()?),
            mutation: Some(format!("minimize remove actions[{start}..{end}]")),
            intervention: None,
        };
        candidate.validate()?;
        let id = candidate.id()?;
        if ids.insert(id) {
            candidates.push(candidate);
        }
    }
    Ok(candidates)
}

fn duration_reductions(
    source: &Candidate,
    generation: u32,
) -> Result<Vec<Candidate>, EvaluateError> {
    let mut candidates = Vec::new();
    let mut ids = BTreeSet::new();
    for index in 0..source.actions.len() {
        for action in shorter_actions(&source.actions[index]) {
            let mut candidate = source.clone();
            candidate.actions[index] = action;
            candidate.ancestry = Ancestry {
                generation,
                parent_id: Some(source.id()?),
                mutation: Some(format!("minimize action[{index}] duration -1")),
                intervention: None,
            };
            candidate.validate()?;
            let id = candidate.id()?;
            if ids.insert(id) {
                candidates.push(candidate);
            }
        }
    }
    Ok(candidates)
}

fn shorter_actions(action: &MacroAction) -> Vec<MacroAction> {
    let mut actions = Vec::new();
    match action {
        MacroAction::Move {
            angle_degrees,
            magnitude,
            frames,
        } if *frames > 1 => actions.push(MacroAction::Move {
            angle_degrees: *angle_degrees,
            magnitude: *magnitude,
            frames: frames - 1,
        }),
        MacroAction::Roll {
            angle_degrees,
            magnitude,
            button_frame,
            recovery_frames,
            spacing,
        } if *recovery_frames > 1 && *button_frame + 1 < *recovery_frames => {
            actions.push(MacroAction::Roll {
                angle_degrees: *angle_degrees,
                magnitude: *magnitude,
                button_frame: *button_frame,
                recovery_frames: recovery_frames - 1,
                spacing: *spacing,
            });
        }
        MacroAction::Neutral { frames } if *frames > 1 => {
            actions.push(MacroAction::Neutral { frames: frames - 1 });
        }
        MacroAction::Press {
            buttons,
            hold_frames,
            neutral_frames,
        } => {
            if *hold_frames > 1 {
                actions.push(MacroAction::Press {
                    buttons: buttons.clone(),
                    hold_frames: hold_frames - 1,
                    neutral_frames: *neutral_frames,
                });
            }
            if *neutral_frames > 0 {
                actions.push(MacroAction::Press {
                    buttons: buttons.clone(),
                    hold_frames: *hold_frames,
                    neutral_frames: neutral_frames - 1,
                });
            }
        }
        MacroAction::PadRun { pad, frames } if *frames > 1 => {
            actions.push(MacroAction::PadRun {
                pad: *pad,
                frames: frames - 1,
            });
        }
        _ => {}
    }
    actions
}

fn round_record(
    round: u32,
    operation: &str,
    evaluated: usize,
    retained: &ProvenRouteCandidate,
    accepted: bool,
) -> Result<AnchoredRouteMinimizeRound, EvaluateError> {
    round_record_with_id(
        round,
        operation,
        evaluated,
        retained,
        accepted.then(|| retained.candidate.id()).transpose()?,
    )
}

fn round_record_with_id(
    round: u32,
    operation: &str,
    evaluated: usize,
    retained: &ProvenRouteCandidate,
    accepted_candidate_id: Option<String>,
) -> Result<AnchoredRouteMinimizeRound, EvaluateError> {
    Ok(AnchoredRouteMinimizeRound {
        round,
        operation: operation.into(),
        evaluated_candidates: evaluated,
        accepted_candidate_id,
        retained_frames: retained.candidate.frame_count(),
        retained_actions: retained.candidate.actions.len(),
        retained_input_complexity: tape_input_complexity(&retained.tape),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::SearchPadState;

    fn candidate() -> Candidate {
        Candidate {
            schema: "dusklight-search-candidate/v2".into(),
            segment: SegmentProfile::Fsp103ToFsp104,
            boot: TapeBoot::Process,
            actions: vec![
                MacroAction::PadRun {
                    pad: SearchPadState {
                        buttons: 0,
                        stick_x: 127,
                        stick_y: 0,
                        substick_x: 0,
                        substick_y: 0,
                        trigger_left: 0,
                        trigger_right: 0,
                        analog_a: 0,
                        analog_b: 0,
                        connected: true,
                        error: 0,
                    },
                    frames: 4,
                },
                MacroAction::Neutral { frames: 3 },
                MacroAction::Move {
                    angle_degrees: 90,
                    magnitude: 127,
                    frames: 2,
                },
            ],
            ancestry: Ancestry::default(),
        }
    }

    fn objective() -> AnchoredObjectiveIdentity {
        AnchoredObjectiveIdentity {
            schema: "dusklight-anchored-search-objective/v2".into(),
            segment: SegmentProfile::Fsp103ToFsp104,
            digest: "a".repeat(64),
            prefix_sha256: "b".repeat(64),
            prefix_frames: 440,
            milestone_program_sha256: "c".repeat(64),
            game_sha256: "d".repeat(64),
            dvd_sha256: "e".repeat(64),
            source_milestone: "source".into(),
            source_definition_sha256: "f".repeat(64),
            source_boundary_fingerprint: "1".repeat(32),
            source_tape_frame: 439,
            source_boundary_index: 440,
            goal_milestone: "goal".into(),
            goal_definition_sha256: "2".repeat(64),
        }
    }

    fn config(candidate: Candidate) -> AnchoredRouteMinimizeConfig {
        AnchoredRouteMinimizeConfig {
            candidate,
            objective: AnchoredObjectiveConfig {
                segment: SegmentProfile::Fsp103ToFsp104,
                prefix_tape: "prefix.tape".into(),
                milestone_program: "objective.dmsp".into(),
                game: "game".into(),
                dvd: "dvd".into(),
                source_milestone: "source".into(),
                source_boundary_fingerprint: "1".repeat(32),
                goal_milestone: "goal".into(),
            },
            output_root: "output".into(),
            working_directory: ".".into(),
            game_args_prefix: Vec::new(),
            workers: 1,
            repetitions: 2,
            candidate_budget: 10,
            resume: true,
            timeout: std::time::Duration::from_secs(1),
            harness: None,
        }
    }

    #[test]
    fn partitions_and_duration_reductions_are_bounded_and_deterministic() {
        let source = candidate();
        source.validate().unwrap();
        let partitions = action_partition_removals(&source, 2, 1).unwrap();
        assert_eq!(partitions.len(), 2);
        assert!(
            partitions
                .iter()
                .all(|candidate| candidate.actions.len() < 3)
        );
        let reductions = duration_reductions(&source, 2).unwrap();
        assert_eq!(reductions.len(), 3);
        assert!(
            reductions
                .iter()
                .all(|candidate| candidate.frame_count() == source.frame_count() - 1)
        );
    }

    #[test]
    fn exact_target_rejects_tick_or_terminal_state_drift() {
        let boundary = BoundaryFingerprint {
            schema: "dusklight.milestone-boundary/v4".into(),
            algorithm: "xxh3-128".into(),
            canonical_encoding: "little-endian-fixed-v4".into(),
            digest: "11111111111111111111111111111111".into(),
        };
        let proven = ProvenRouteCandidate {
            tape: candidate().compile().unwrap(),
            candidate: candidate(),
            first_hit_tick: 10,
            goal_sim_tick: 450,
            goal_tape_frame: 450,
            goal_boundary_fingerprint: boundary.clone(),
        };
        let target = RouteReductionTarget {
            first_hit_tick: 10,
            goal_sim_tick: 450,
            goal_tape_frame: 450,
            goal_boundary_fingerprint: boundary,
        };
        assert!(target.accepts(&proven));
        let mut drifted = proven.clone();
        drifted.first_hit_tick += 1;
        assert!(!target.accepts(&drifted));
        drifted = proven;
        drifted.goal_boundary_fingerprint.digest = "22222222222222222222222222222222".into();
        assert!(!target.accepts(&drifted));
    }

    #[test]
    fn resume_checkpoint_rejects_budget_history_and_target_drift() {
        let retained = candidate();
        let retained_tape = retained.compile().unwrap();
        let objective = objective();
        let target = RouteReductionTarget {
            first_hit_tick: 10,
            goal_sim_tick: 575,
            goal_tape_frame: 575,
            goal_boundary_fingerprint: BoundaryFingerprint {
                schema: "dusklight.milestone-boundary/v4".into(),
                algorithm: "xxh3-128".into(),
                canonical_encoding: "little-endian-fixed-v4".into(),
                digest: "3".repeat(32),
            },
        };
        let source_id = retained.id().unwrap();
        let checkpoint = RouteMinimizeCheckpoint {
            schema: "dusklight-anchored-route-minimization-checkpoint/v1".into(),
            objective: objective.clone(),
            harness_request_sha256: None,
            source_candidate_id: source_id.clone(),
            candidate_budget: 10,
            target: target.clone(),
            retained_candidate: retained.clone(),
            history: vec![AnchoredRouteMinimizeRound {
                round: 1,
                operation: "trim_after_goal".into(),
                evaluated_candidates: 1,
                accepted_candidate_id: Some(source_id.clone()),
                retained_frames: retained.frame_count(),
                retained_actions: retained.actions.len(),
                retained_input_complexity: tape_input_complexity(&retained_tape),
            }],
            proposal_evaluations: 1,
            accepted_reductions: 1,
            next_round: 2,
            phase: RouteMinimizePhase::Actions { granularity: 2 },
        };
        let config = config(retained);
        validate_checkpoint(&config, &objective, &source_id, &target, &checkpoint).unwrap();

        let mut tampered = checkpoint.clone();
        tampered.proposal_evaluations = 2;
        assert!(validate_checkpoint(&config, &objective, &source_id, &target, &tampered).is_err());
        tampered = checkpoint.clone();
        tampered.target.goal_sim_tick += 1;
        assert!(validate_checkpoint(&config, &objective, &source_id, &target, &tampered).is_err());
        tampered = checkpoint.clone();
        let MacroAction::Move { angle_degrees, .. } = &mut tampered.retained_candidate.actions[2]
        else {
            panic!("fixture action changed")
        };
        *angle_degrees += 1;
        assert!(validate_checkpoint(&config, &objective, &source_id, &target, &tampered).is_err());
        tampered = checkpoint.clone();
        tampered.history[0].operation = "reduce_action_duration".into();
        assert!(validate_checkpoint(&config, &objective, &source_id, &target, &tampered).is_err());
        let mut changed_budget = config;
        changed_budget.candidate_budget = 11;
        assert!(
            validate_checkpoint(
                &changed_budget,
                &objective,
                &source_id,
                &target,
                &checkpoint,
            )
            .is_err()
        );
    }

    #[test]
    fn checkpoint_authority_rejects_legacy_or_changed_run_requests() {
        let request = ArtifactDigest([1; 32]);
        let changed = ArtifactDigest([2; 32]);
        assert!(checkpoint_harness_is_valid(
            "dusklight-anchored-route-minimization-checkpoint/v1",
            None,
            None,
        ));
        assert!(!checkpoint_harness_is_valid(
            "dusklight-anchored-route-minimization-checkpoint/v1",
            None,
            Some(request),
        ));
        assert!(checkpoint_harness_is_valid(
            "dusklight-anchored-route-minimization-checkpoint/v2",
            Some(request),
            Some(request),
        ));
        assert!(!checkpoint_harness_is_valid(
            "dusklight-anchored-route-minimization-checkpoint/v2",
            Some(request),
            Some(changed),
        ));
    }
}
