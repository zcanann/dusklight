use super::*;

/// Evaluate several proposer populations under the same declared cap. Every
/// selected candidate enters one deduplicated native population, so no proposer
/// can bypass the evaluator or spend simulator time twice on a shared tape.
pub fn run_proposer_tournament(
    config: &ProposerTournamentConfig,
) -> Result<ProposerTournamentSummary, EvaluateError> {
    let definition = &config.definition;
    if config.harness.is_some() && config.anchored.is_some() {
        return Err(EvaluateError::InvalidConfig(
            "tournament execution cannot combine a harness request with an anchored objective"
                .into(),
        ));
    }
    if definition.schema != "dusklight-proposer-tournament-definition/v2"
        || definition.budget_per_proposer == 0
        || definition.proposers.len() < 2
        || definition.proposers.len() > 16
        || config.workers == 0
        || config.repetitions < 2
        || config.timeout.is_zero()
        || !config.definition_directory.is_dir()
        || !config.game.is_file()
        || !config.dvd.is_file()
        || !config.working_directory.is_dir()
        || directory_is_nonempty(&config.output_root)?
    {
        return Err(EvaluateError::InvalidConfig(
            "tournaments require a bounded v2 definition with authenticated proposal envelopes, 2..=16 proposers, native inputs, at least two repetitions, and a new output root"
                .into(),
        ));
    }
    if !definition
        .proposers
        .iter()
        .any(|proposer| proposer.kind == TournamentProposerKind::IncumbentMutation)
        || !definition
            .proposers
            .iter()
            .any(|proposer| proposer.kind == TournamentProposerKind::BlindExploration)
    {
        return Err(EvaluateError::InvalidConfig(
            "a fair tournament must retain incumbent_mutation and blind_exploration proposers"
                .into(),
        ));
    }
    let episode_slots = match definition.budget_unit {
        TournamentBudgetUnit::Episodes => {
            let repetitions = u64::from(config.repetitions);
            if !definition.budget_per_proposer.is_multiple_of(repetitions) {
                return Err(EvaluateError::InvalidConfig(
                    "episode budget must be an exact multiple of repetitions".into(),
                ));
            }
            Some(
                usize::try_from(definition.budget_per_proposer / repetitions).map_err(|_| {
                    EvaluateError::InvalidConfig("episode budget is too large".into())
                })?,
            )
        }
        TournamentBudgetUnit::CandidateTicks => None,
    };
    if episode_slots == Some(0) {
        return Err(EvaluateError::InvalidConfig(
            "episode budget cannot select zero candidates".into(),
        ));
    }

    struct SelectedProposer {
        name: String,
        kind: TournamentProposerKind,
        proposer: ProposerIdentity,
        proposal_envelope_set_sha256: ArtifactDigest,
        candidate_ids: Vec<String>,
        candidate_ticks: u64,
    }

    let mut names = HashSet::new();
    let mut segment = None;
    let mut boot = None;
    let mut objective = None;
    let mut action_schema = None;
    let mut selected = Vec::new();
    let mut union = BTreeMap::<String, Candidate>::new();
    for proposer in &definition.proposers {
        if proposer.name.is_empty()
            || proposer.name.len() > 64
            || !proposer
                .name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            || !names.insert(proposer.name.clone())
        {
            return Err(EvaluateError::InvalidConfig(
                "proposer names must be unique 1..=64 byte identifiers".into(),
            ));
        }
        let population_path = if proposer.population.is_absolute() {
            proposer.population.clone()
        } else {
            config.definition_directory.join(&proposer.population)
        };
        let population_path = fs::canonicalize(population_path)?;
        let manifest: PopulationManifest = serde_json::from_slice(&fs::read(&population_path)?)?;
        validate_manifest(&manifest, &population_path)?;
        let proposal_envelopes_path = if proposer.proposal_envelopes.is_absolute() {
            proposer.proposal_envelopes.clone()
        } else {
            config
                .definition_directory
                .join(&proposer.proposal_envelopes)
        };
        let proposal_envelopes_path = fs::canonicalize(proposal_envelopes_path)?;
        let envelope_set: CandidateEnvelopeSet =
            serde_json::from_slice(&fs::read(&proposal_envelopes_path)?)?;
        envelope_set.validate().map_err(|error| {
            EvaluateError::InvalidManifest(format!(
                "proposer {:?} has an invalid candidate-envelope set: {error}",
                proposer.name
            ))
        })?;
        let proposer_identity = envelope_set.envelopes[0].proposer.clone();
        if envelope_set
            .envelopes
            .iter()
            .any(|envelope| envelope.proposer != proposer_identity)
        {
            return Err(EvaluateError::InvalidManifest(format!(
                "tournament proposer {:?} mixes authenticated proposer identities",
                proposer.name
            )));
        }
        let proposer_objective = envelope_set.envelopes[0].objective.clone();
        let proposer_action_schema = envelope_set.envelopes[0].action_schema.clone();
        if proposer_identity.kind != proposer.kind.envelope_kind() {
            return Err(EvaluateError::InvalidManifest(format!(
                "proposer {:?} declares {:?} but its authenticated envelope kind is {:?}",
                proposer.name, proposer.kind, proposer_identity.kind
            )));
        }
        if objective
            .as_ref()
            .is_some_and(|value| value != &proposer_objective)
            || action_schema
                .as_ref()
                .is_some_and(|value| value != &proposer_action_schema)
        {
            return Err(EvaluateError::InvalidManifest(
                "tournament proposal envelopes must share one exact objective and action schema"
                    .into(),
            ));
        }
        objective.get_or_insert_with(|| proposer_objective.clone());
        action_schema.get_or_insert_with(|| proposer_action_schema.clone());
        let envelopes_by_candidate = envelope_set
            .envelopes
            .iter()
            .map(|envelope| (envelope.candidate_sha256, envelope))
            .collect::<BTreeMap<_, _>>();
        if segment.is_some_and(|value| value != manifest.segment)
            || boot.as_ref().is_some_and(|value| *value != manifest.boot)
        {
            return Err(EvaluateError::InvalidManifest(
                "tournament populations must share one segment and boot origin".into(),
            ));
        }
        segment = Some(manifest.segment);
        boot.get_or_insert_with(|| manifest.boot.clone());
        let population_root = canonical_parent(&population_path)?;
        let mut ids = Vec::new();
        let mut candidate_ticks = 0_u64;
        let mut manifest_candidate_digests = BTreeSet::new();
        for member in &manifest.members {
            let candidate_path = fs::canonicalize(population_root.join(&member.candidate_file))?;
            if !candidate_path.starts_with(&population_root) {
                return Err(EvaluateError::InvalidManifest(
                    "tournament candidate escapes its population root".into(),
                ));
            }
            let candidate: Candidate = serde_json::from_slice(&fs::read(candidate_path)?)?;
            candidate.validate()?;
            let id = candidate.id()?;
            let candidate_sha256 = id.parse().map_err(|error| {
                EvaluateError::InvalidManifest(format!(
                    "proposer {:?} has an invalid candidate digest: {error}",
                    proposer.name
                ))
            })?;
            manifest_candidate_digests.insert(candidate_sha256);
            let envelope = envelopes_by_candidate
                .get(&candidate_sha256)
                .ok_or_else(|| {
                    EvaluateError::InvalidManifest(format!(
                        "proposer {:?} candidate {id} has no authenticated proposal envelope",
                        proposer.name
                    ))
                })?;
            let parent_candidate_sha256 = candidate
                .ancestry
                .parent_id
                .as_deref()
                .map(str::parse)
                .transpose()
                .map_err(|error| {
                    EvaluateError::InvalidManifest(format!(
                        "proposer {:?} has an invalid parent candidate digest: {error}",
                        proposer.name
                    ))
                })?;
            if id != member.candidate_id
                || candidate.segment != manifest.segment
                || candidate.boot != manifest.boot
                || candidate.frame_count() != member.frame_count
                || envelope.generation != candidate.ancestry.generation
                || envelope.parent_candidate_sha256 != parent_candidate_sha256
                || envelope.seed != manifest.rng_seed
            {
                return Err(EvaluateError::InvalidManifest(format!(
                    "proposer {:?} contains a candidate, manifest, lineage, or seed identity mismatch",
                    proposer.name
                )));
            }
            let cost = member
                .frame_count
                .checked_mul(u64::from(config.repetitions))
                .ok_or_else(|| {
                    EvaluateError::InvalidConfig("candidate-tick cost overflowed".into())
                })?;
            let accept = match episode_slots {
                Some(slots) => ids.len() < slots,
                None => candidate_ticks
                    .checked_add(cost)
                    .is_some_and(|total| total <= definition.budget_per_proposer),
            };
            if !accept {
                continue;
            }
            candidate_ticks += cost;
            ids.push(id.clone());
            union.entry(id).or_insert(candidate);
        }
        if manifest_candidate_digests != envelopes_by_candidate.keys().copied().collect() {
            return Err(EvaluateError::InvalidManifest(format!(
                "proposer {:?} candidate-envelope set does not exactly cover its population",
                proposer.name
            )));
        }
        if ids.is_empty() || episode_slots.is_some_and(|slots| ids.len() != slots) {
            return Err(EvaluateError::InvalidConfig(format!(
                "proposer {:?} cannot fill its declared budget with valid candidates",
                proposer.name
            )));
        }
        selected.push(SelectedProposer {
            name: proposer.name.clone(),
            kind: proposer.kind,
            proposer: proposer_identity,
            proposal_envelope_set_sha256: envelope_set.content_sha256,
            candidate_ids: ids,
            candidate_ticks,
        });
    }
    if union.len() > 10_000 {
        return Err(EvaluateError::InvalidConfig(
            "tournament union exceeds 10,000 physical candidates".into(),
        ));
    }
    let segment = segment.ok_or_else(|| EvaluateError::InvalidConfig("empty tournament".into()))?;
    let boot = boot.ok_or_else(|| EvaluateError::InvalidConfig("empty tournament".into()))?;
    let objective = objective.ok_or_else(|| {
        EvaluateError::InvalidManifest("tournament objective identity is missing".into())
    })?;
    let action_schema = action_schema.ok_or_else(|| {
        EvaluateError::InvalidManifest("tournament action schema identity is missing".into())
    })?;
    if let Some(harness) = &config.harness {
        let request = &harness.request_template;
        let expected_objective = NamedDigest::new(
            request.objective.goal.clone(),
            request.objective.program_sha256,
        );
        let expected_action_schema = NamedDigest::new(
            request.action_schema.id.clone(),
            request.action_schema.sha256,
        );
        if objective != expected_objective || action_schema != expected_action_schema {
            return Err(EvaluateError::InvalidManifest(
                "tournament proposal envelopes do not match the authenticated run objective and action schema"
                    .into(),
            ));
        }
    }
    let prepared_anchored = config
        .anchored
        .as_ref()
        .map(|anchored| prepare_anchored_objective(anchored, PathBuf::new()))
        .transpose()?;
    if let Some(prepared) = &prepared_anchored {
        let identity = &prepared.identity;
        let expected_objective = NamedDigest::new(
            identity.goal_milestone.clone(),
            identity.digest.parse().map_err(|error| {
                EvaluateError::InvalidResult(format!("invalid anchored objective digest: {error}"))
            })?,
        );
        let expected_action_schema =
            NamedDigest::new("movement-action/v2", movement_action_schema_digest_v2());
        if objective != expected_objective || action_schema != expected_action_schema {
            return Err(EvaluateError::InvalidManifest(
                "tournament proposal envelopes do not match the anchored objective and action schema"
                    .into(),
            ));
        }
    }
    fs::create_dir_all(&config.output_root)?;
    let population_root = config.output_root.join("population");
    let candidates_by_id = union.clone();
    let manifest =
        write_explicit_population(&population_root, segment, 0, union.into_values().collect())?;
    if manifest.boot != boot {
        return Err(EvaluateError::InvalidManifest(
            "deduplicated tournament changed the boot origin".into(),
        ));
    }
    let results_path = config.output_root.join("results.json");
    let started = Instant::now();
    let evaluation = EvaluateConfig {
        population_path: population_root.join("manifest.json"),
        game: config.game.clone(),
        dvd: config.dvd.clone(),
        output_root: config.output_root.join("evaluations"),
        results_path: results_path.clone(),
        working_directory: config.working_directory.clone(),
        game_args_prefix: config.game_args_prefix.clone(),
        workers: config.workers,
        repetitions: config.repetitions,
        timeout: config.timeout,
        harness: config.harness.clone(),
    };
    let (report, results) = if let Some(objective) = &config.anchored {
        let (report, results) = evaluate_anchored_population_internal(
            &AnchoredEvaluateConfig {
                evaluation,
                objective: objective.clone(),
            },
            prepared_anchored.as_ref(),
        )?;
        (report, results.results)
    } else {
        let report = evaluate_population(&evaluation)?;
        let results: SearchResults = serde_json::from_slice(&fs::read(&results_path)?)?;
        (report, results)
    };
    if let Some(harness) = &config.harness {
        validate_tournament_attempt_compatibility(&report, harness)?;
    }
    let evaluation_wall_millis = started.elapsed().as_millis();
    let leaderboard = rank_population(&manifest, &results)?;
    write_json(&config.output_root.join("leaderboard.json"), &leaderboard)?;
    let scores = leaderboard
        .iter()
        .map(|row| (row.candidate_id.as_str(), row.score))
        .collect::<BTreeMap<_, _>>();
    let incumbent_score = selected
        .iter()
        .filter(|proposer| proposer.kind == TournamentProposerKind::IncumbentMutation)
        .flat_map(|proposer| &proposer.candidate_ids)
        .filter_map(|id| scores.get(id.as_str()).copied())
        .max()
        .ok_or(EvaluateError::EmptyLeaderboard)?;
    let incidence = selected
        .iter()
        .flat_map(|proposer| &proposer.candidate_ids)
        .fold(BTreeMap::<String, usize>::new(), |mut counts, id| {
            *counts.entry(id.clone()).or_default() += 1;
            counts
        });
    let frame_counts = manifest
        .members
        .iter()
        .map(|member| (member.candidate_id.as_str(), member.frame_count))
        .collect::<BTreeMap<_, _>>();
    let finalists_root = config.output_root.join("finalists");
    fs::create_dir_all(&finalists_root)?;
    let mut rows = Vec::new();
    for proposer in selected {
        let best = leaderboard
            .iter()
            .filter(|row| proposer.candidate_ids.contains(&row.candidate_id))
            .max_by(|left, right| left.score.cmp(&right.score))
            .ok_or(EvaluateError::EmptyLeaderboard)?;
        let predicate_hits = proposer
            .candidate_ids
            .iter()
            .filter(|id| results.candidates[*id].goal_reached == Some(true))
            .count();
        let misses = proposer.candidate_ids.len() - predicate_hits;
        let boundaries = report
            .attempts
            .iter()
            .filter(|attempt| proposer.candidate_ids.contains(&attempt.candidate_id))
            .flat_map(|attempt| attempt.boundary_fingerprints.values())
            .map(|fingerprint| fingerprint.digest.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let improvements_over_incumbent = proposer
            .candidate_ids
            .iter()
            .filter(|id| scores[id.as_str()] > incumbent_score)
            .count();
        let frame_wins = proposer
            .candidate_ids
            .iter()
            .map(|id| scores[id.as_str()])
            .filter(|score| {
                score.goal_feasible
                    && incumbent_score.goal_feasible
                    && score.milestone_depth >= incumbent_score.milestone_depth
                    && score.median_first_hit_tick < incumbent_score.median_first_hit_tick
            })
            .count();
        let observed_simulator_ticks = report
            .attempts
            .iter()
            .filter(|attempt| proposer.candidate_ids.contains(&attempt.candidate_id))
            .map(|attempt| {
                observed_attempt_ticks(attempt, frame_counts[attempt.candidate_id.as_str()])
            })
            .sum();
        let (replay_verdict, best_proved_tape, best_proved_tape_sha256) =
            if best.score.goal_feasible {
                let tape_bytes = if config.anchored.is_some() {
                    let attempt = report
                        .attempts
                        .iter()
                        .find(|attempt| {
                            attempt.candidate_id == best.candidate_id && attempt.goal_reached
                        })
                        .ok_or_else(|| {
                            EvaluateError::InvalidResult(format!(
                                "best anchored candidate {} has no proved replay attempt",
                                best.candidate_id
                            ))
                        })?;
                    fs::read(&attempt.tape)?
                } else {
                    let candidate = candidates_by_id.get(&best.candidate_id).ok_or_else(|| {
                        EvaluateError::InvalidResult(format!(
                            "best candidate {} is absent from the evaluated population",
                            best.candidate_id
                        ))
                    })?;
                    candidate.compile()?.encode()?
                };
                let tape_sha256 = ArtifactDigest(Sha256::digest(&tape_bytes).into());
                let tape_path = finalists_root.join(format!("{tape_sha256}.tape"));
                if !tape_path.exists() {
                    fs::write(&tape_path, &tape_bytes)?;
                }
                (
                    ProposerReplayVerdict::Proved,
                    Some(tape_path),
                    Some(tape_sha256),
                )
            } else {
                (ProposerReplayVerdict::ObjectiveMiss, None, None)
            };
        rows.push(ProposerTournamentRow {
            name: proposer.name,
            kind: proposer.kind,
            proposer: proposer.proposer,
            proposal_envelope_set_sha256: proposer.proposal_envelope_set_sha256,
            selected_candidates: proposer.candidate_ids.len(),
            charged_episodes: proposer.candidate_ids.len() as u64 * u64::from(config.repetitions),
            charged_candidate_ticks: proposer.candidate_ticks,
            observed_simulator_ticks,
            shared_duplicate_proposals: proposer
                .candidate_ids
                .iter()
                .filter(|id| incidence[id.as_str()] > 1)
                .count(),
            improvements_over_incumbent,
            misses,
            crashes: 0,
            predicate_hits,
            predicate_hit_rate: predicate_hits as f64 / proposer.candidate_ids.len() as f64,
            frame_wins,
            boundary_diversity: boundaries.len(),
            boundary_fingerprints: boundaries,
            cold_replay_pass_rate: predicate_hits as f64 / proposer.candidate_ids.len() as f64,
            replay_verdict,
            best_candidate_id: best.candidate_id.clone(),
            best_score: best.score,
            best_proved_tape,
            best_proved_tape_sha256,
        });
    }
    rows.sort_by(|left, right| left.name.cmp(&right.name));
    let champion = leaderboard.first().ok_or(EvaluateError::EmptyLeaderboard)?;
    let physical_candidate_ticks = manifest
        .members
        .iter()
        .map(|member| member.frame_count * u64::from(config.repetitions))
        .sum();
    let physical_simulator_ticks = report
        .attempts
        .iter()
        .map(|attempt| observed_attempt_ticks(attempt, frame_counts[attempt.candidate_id.as_str()]))
        .sum();
    let summary = ProposerTournamentSummary {
        schema: "dusklight-proposer-tournament/v3",
        segment,
        boot,
        objective,
        action_schema,
        budget_unit: definition.budget_unit,
        budget_per_proposer: definition.budget_per_proposer,
        repetitions: config.repetitions,
        physical_candidates: manifest.members.len(),
        physical_episodes: manifest.members.len() * config.repetitions as usize,
        physical_candidate_ticks,
        physical_simulator_ticks,
        evaluation_wall_millis,
        incumbent_score,
        rows,
        champion_id: champion.candidate_id.clone(),
        champion_score: champion.score,
        output_root: config.output_root.clone(),
    };
    write_json(
        &config.output_root.join("tournament.summary.json"),
        &summary,
    )?;
    Ok(summary)
}

fn observed_attempt_ticks(attempt: &AttemptEvidence, candidate_ticks: u64) -> u64 {
    observed_simulator_ticks(
        attempt.goal_reached,
        attempt.first_hit_tick,
        candidate_ticks,
    )
}

fn observed_simulator_ticks(
    goal_reached: bool,
    first_hit_tick: Option<u64>,
    candidate_ticks: u64,
) -> u64 {
    if goal_reached {
        first_hit_tick
            .map(|tick| tick.saturating_add(1))
            .unwrap_or(candidate_ticks)
    } else {
        // A source or intermediate milestone hit is ranking progress, not an
        // early-stop objective hit. A miss consumes the candidate's full
        // logical simulator budget.
        candidate_ticks
    }
}

#[cfg(test)]
mod accounting_tests {
    use super::observed_simulator_ticks;

    #[test]
    fn intermediate_progress_does_not_undercharge_an_objective_miss() {
        assert_eq!(observed_simulator_ticks(false, Some(0), 144), 144);
        assert_eq!(observed_simulator_ticks(true, Some(138), 144), 139);
        assert_eq!(observed_simulator_ticks(true, None, 144), 144);
    }
}

fn validate_tournament_attempt_compatibility(
    report: &EvaluationReport,
    harness: &HarnessEvaluateConfig,
) -> Result<(), EvaluateError> {
    for attempt in &report.attempts {
        let request_path = attempt.harness_request.as_ref().ok_or_else(|| {
            EvaluateError::InvalidResult(format!(
                "tournament attempt {} omitted its authenticated request",
                attempt.candidate_id
            ))
        })?;
        let result_path = attempt.harness_result.as_ref().ok_or_else(|| {
            EvaluateError::InvalidResult(format!(
                "tournament attempt {} omitted its authenticated result",
                attempt.candidate_id
            ))
        })?;
        let request: HarnessRunRequest = serde_json::from_slice(&fs::read(request_path)?)?;
        let result: HarnessRunResult = serde_json::from_slice(&fs::read(result_path)?)?;
        ensure_compatible(
            CompatibilityMode::CrossBuildComparison,
            &harness.request_template.identity,
            &request.identity,
        )
        .map_err(|error| {
            EvaluateError::InvalidResult(format!(
                "tournament attempt {} is comparison-incompatible: {error}",
                attempt.candidate_id
            ))
        })?;
        result
            .validate_files(&request, &attempt.artifact_root)
            .map_err(|error| {
                EvaluateError::InvalidResult(format!(
                    "tournament attempt {} is replay-incompatible: {error}",
                    attempt.candidate_id
                ))
            })?;
    }
    Ok(())
}

pub(super) fn tape_intervention(
    parent: &InputTape,
    child: &InputTape,
) -> Option<InterventionRange> {
    if parent.boot != child.boot
        || parent.tick_rate_numerator != child.tick_rate_numerator
        || parent.tick_rate_denominator != child.tick_rate_denominator
    {
        return None;
    }
    let shared = parent.frames.len().min(child.frames.len());
    let start = (0..shared)
        .find(|index| parent.frames[*index] != child.frames[*index])
        .or_else(|| (parent.frames.len() != child.frames.len()).then_some(shared))?;
    let mut parent_end = parent.frames.len();
    let mut child_end = child.frames.len();
    while parent_end > start
        && child_end > start
        && parent.frames[parent_end - 1] == child.frames[child_end - 1]
    {
        parent_end -= 1;
        child_end -= 1;
    }
    Some(InterventionRange {
        start_frame: start as u64,
        end_frame_exclusive: child_end as u64,
        parent_end_frame_exclusive: parent_end as u64,
    })
}

pub(super) fn required_native_facts_supported(attempts: &[AttemptEvidence]) -> bool {
    if attempts
        .iter()
        .any(|attempt| attempt.harness_terminal.is_some())
    {
        return native_terminals_support_required_facts(
            attempts.iter().map(|attempt| attempt.harness_terminal),
        );
    }

    // Anchored route evaluation predates HarnessRunResult, but its first
    // repetition extracts the same authenticated observation view into a
    // sealed transition corpus. That successful extraction is direct evidence
    // that the required native facts and channel ABI were available. Later
    // repetitions intentionally omit traces and must not negate it.
    !attempts.is_empty()
        && attempts
            .iter()
            .all(|attempt| attempt.infrastructure_error.is_none())
        && attempts
            .iter()
            .any(|attempt| attempt.transition_corpus.is_some())
}

pub(super) fn native_terminals_support_required_facts(
    terminals: impl IntoIterator<Item = Option<HarnessTerminalReason>>,
) -> bool {
    let mut observed = false;
    for terminal in terminals {
        observed = true;
        if matches!(
            terminal,
            None | Some(
                HarnessTerminalReason::Unsupported | HarnessTerminalReason::CapabilityMismatch
            )
        ) {
            return false;
        }
    }
    observed
}

pub(super) fn learned_holdout_scores_adequate(
    rows: impl IntoIterator<Item = (bool, LexicographicScore)>,
) -> bool {
    let mut best_learned: Option<LexicographicScore> = None;
    let mut best_baseline: Option<LexicographicScore> = None;
    for (learned, score) in rows {
        let best = if learned {
            &mut best_learned
        } else {
            &mut best_baseline
        };
        if best.as_ref().is_none_or(|incumbent| score > *incumbent) {
            *best = Some(score);
        }
    }
    matches!((best_learned, best_baseline), (Some(learned), Some(baseline)) if learned >= baseline)
}

pub(super) fn learned_proposal_held_out_performance(
    manifest: &PopulationManifest,
    leaderboard: &[LeaderboardEntry],
) -> bool {
    let member_by_id = manifest
        .members
        .iter()
        .map(|member| (member.candidate_id.as_str(), member))
        .collect::<BTreeMap<_, _>>();
    learned_holdout_scores_adequate(leaderboard.iter().filter_map(|row| {
        let member = member_by_id.get(row.candidate_id.as_str())?;
        let learned = member.ancestry.mutation.as_deref().is_some_and(|mutation| {
            mutation.starts_with("q_guided") || mutation.starts_with("q_disagreement_heuristic")
        });
        Some((learned, row.score))
    }))
}
