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

#[derive(Clone, Debug, Eq, PartialEq)]
struct RouteReductionTarget {
    first_hit_tick: u64,
    goal_sim_tick: u64,
    goal_tape_frame: u64,
    goal_boundary_fingerprint: BoundaryFingerprint,
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
    if config.candidate.segment != config.objective.segment
        || !is_anchored_profile(config.candidate.segment)
        || !config.working_directory.is_dir()
        || config.workers == 0
        || config.repetitions < 2
        || config.candidate_budget == 0
        || config.timeout.is_zero()
        || directory_is_nonempty(&config.output_root)?
    {
        return Err(EvaluateError::InvalidConfig(
            "anchored route minimization requires a matching movement candidate, at least two repetitions, positive bounded execution limits, and a new output root"
                .into(),
        ));
    }
    let prepared = prepare_anchored_objective(&config.objective, PathBuf::new())?;
    let objective = prepared.identity.clone();
    fs::create_dir_all(&config.output_root)?;

    let source_id = config.candidate.id()?;
    let source_tape = config.candidate.compile()?;
    let source_frames = config.candidate.frame_count();
    let source_actions = config.candidate.actions.len();
    let source_input_complexity = tape_input_complexity(&source_tape);
    let (source_candidates, source_report) = evaluate_route_batch(
        config,
        &prepared,
        vec![config.candidate.clone()],
        &config.output_root.join("source"),
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

    let mut current = source;
    let mut history = Vec::new();
    let mut proposal_evaluations = 0_usize;
    let mut accepted_reductions = 0_usize;
    let mut round = 1_u32;

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
    if required_suffix_frames < current.candidate.frame_count() && proposal_evaluations < config.candidate_budget {
        let mut trimmed_tape = current.tape.clone();
        trimmed_tape.frames.truncate(required_suffix_frames as usize);
        let mut trimmed = Candidate::from_absolute_tape(current.candidate.segment, &trimmed_tape)?;
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
        let (evaluated, _) = evaluate_route_batch(
            config,
            &prepared,
            vec![trimmed],
            &config.output_root.join("rounds").join(format!("{round:04}")),
            round,
        )?;
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

    let mut granularity = 2_usize;
    while current.candidate.actions.len() > 1 && proposal_evaluations < config.candidate_budget {
        let partitions = granularity.min(current.candidate.actions.len());
        let mut proposals = action_partition_removals(&current.candidate, partitions, round)?;
        proposals.truncate(config.candidate_budget - proposal_evaluations);
        if proposals.is_empty() {
            break;
        }
        let evaluated_count = proposals.len();
        let (evaluated, _) = evaluate_route_batch(
            config,
            &prepared,
            proposals,
            &config.output_root.join("rounds").join(format!("{round:04}")),
            round,
        )?;
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
        } else if partitions == current.candidate.actions.len() {
            history.push(round_record_with_id(
                round,
                "remove_action_partitions",
                evaluated_count,
                &current,
                accepted_id,
            )?);
            round += 1;
            break;
        } else {
            granularity = (partitions * 2).min(current.candidate.actions.len());
        }
        history.push(round_record_with_id(
            round,
            "remove_action_partitions",
            evaluated_count,
            &current,
            accepted_id,
        )?);
        round += 1;
    }

    while proposal_evaluations < config.candidate_budget {
        let mut proposals = duration_reductions(&current.candidate, round)?;
        proposals.truncate(config.candidate_budget - proposal_evaluations);
        if proposals.is_empty() {
            break;
        }
        let evaluated_count = proposals.len();
        let (evaluated, _) = evaluate_route_batch(
            config,
            &prepared,
            proposals,
            &config.output_root.join("rounds").join(format!("{round:04}")),
            round,
        )?;
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
        history.push(round_record_with_id(
            round,
            "reduce_action_duration",
            evaluated_count,
            &current,
            accepted_id,
        )?);
        round += 1;
        if no_acceptance {
            break;
        }
    }

    let (final_candidates, final_report) = evaluate_route_batch(
        config,
        &prepared,
        vec![current.candidate.clone()],
        &config.output_root.join("final-proof"),
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
        evaluated_candidates: proposal_evaluations + 2,
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

fn evaluate_route_batch(
    config: &AnchoredRouteMinimizeConfig,
    prepared: &PreparedAnchoredObjective,
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
    let (report, results) = evaluate_anchored_population_internal(&AnchoredEvaluateConfig {
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
            harness: None,
        },
        objective: config.objective.clone(),
    }, Some(prepared))?;
    if results.objective != prepared.identity {
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
        .map(|candidate| reduction_quality(&candidate.candidate).map(|quality| (quality, candidate)))
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

    #[test]
    fn partitions_and_duration_reductions_are_bounded_and_deterministic() {
        let source = candidate();
        source.validate().unwrap();
        let partitions = action_partition_removals(&source, 2, 1).unwrap();
        assert_eq!(partitions.len(), 2);
        assert!(partitions.iter().all(|candidate| candidate.actions.len() < 3));
        let reductions = duration_reductions(&source, 2).unwrap();
        assert_eq!(reductions.len(), 3);
        assert!(reductions
            .iter()
            .all(|candidate| candidate.frame_count() == source.frame_count() - 1));
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
}
