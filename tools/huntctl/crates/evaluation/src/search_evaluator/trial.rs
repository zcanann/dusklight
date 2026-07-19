use super::*;

#[derive(Clone, Debug)]
pub(super) struct Trial {
    pub(super) candidate_id: String,
    pub(super) ancestry: crate::search::Ancestry,
    pub(super) rng_seed: u64,
    pub(super) attempt: u32,
    pub(super) tape: PathBuf,
    pub(super) logical_tick_budget: u64,
    pub(super) boot: TapeBoot,
    pub(super) suffix_tape: Option<PathBuf>,
    pub(super) root: PathBuf,
    pub(super) state: PathBuf,
    pub(super) milestones: PathBuf,
    pub(super) gameplay_trace: Option<PathBuf>,
    pub(super) stdout: PathBuf,
    pub(super) stderr: PathBuf,
}

pub(super) fn build_trials(
    manifest: &PopulationManifest,
    population_root: &Path,
    output_root: &Path,
    repetitions: u32,
) -> Result<Vec<Trial>, EvaluateError> {
    let mut trials = Vec::with_capacity(manifest.members.len() * repetitions as usize);
    for member in &manifest.members {
        if member.candidate_id.is_empty()
            || !member
                .candidate_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(EvaluateError::InvalidManifest(format!(
                "candidate ID {:?} is unsafe",
                member.candidate_id
            )));
        }
        let tape = fs::canonicalize(population_root.join(&member.tape_file))?;
        if !tape.starts_with(population_root) {
            return Err(EvaluateError::InvalidManifest(format!(
                "candidate {} tape escapes the population directory",
                member.candidate_id
            )));
        }
        let candidate_path = fs::canonicalize(population_root.join(&member.candidate_file))?;
        if !candidate_path.starts_with(population_root) {
            return Err(EvaluateError::InvalidManifest(format!(
                "candidate {} source escapes the population directory",
                member.candidate_id
            )));
        }
        let candidate: Candidate = serde_json::from_slice(&fs::read(&candidate_path)?)?;
        candidate.validate()?;
        let tape_bytes = fs::read(&tape)?;
        let decoded = InputTape::decode(&tape_bytes)?;
        let compiled = candidate.compile()?;
        if candidate.segment != manifest.segment
            || candidate.boot != manifest.boot
            || candidate.id()? != member.candidate_id
            || candidate.ancestry != member.ancestry
            || compiled.frames.len() as u64 != member.frame_count
            || member.input_complexity != Some(tape_input_complexity(&compiled))
            || compiled.encode()? != tape_bytes
        {
            return Err(EvaluateError::InvalidManifest(format!(
                "candidate {} ID, ancestry, frame count, complexity, source, and tape are not one content identity",
                member.candidate_id
            )));
        }
        let expected_boot = match manifest.segment {
            SegmentProfile::BootToFsp103 => TapeBoot::Process,
            SegmentProfile::Fsp103ToFsp104 => TapeBoot::Stage {
                stage: "F_SP103".into(),
                room: 1,
                point: 1,
                layer: 3,
                save_slot: None,
                fixture: None,
            },
            SegmentProfile::LinkControlToTunnelCrawlStart => {
                return Err(EvaluateError::InvalidManifest(
                    "anchored movement candidates require an anchored objective".into(),
                ));
            }
        };
        if manifest.boot != expected_boot || decoded.tape.boot != manifest.boot {
            return Err(EvaluateError::InvalidManifest(format!(
                "candidate {} boot origin {:?} and manifest origin {:?} do not match direct profile origin {:?}",
                member.candidate_id, decoded.tape.boot, manifest.boot, expected_boot
            )));
        }
        for attempt in 1..=repetitions {
            let root = output_root
                .join("candidates")
                .join(&member.candidate_id)
                .join(format!("attempt-{attempt:03}"));
            trials.push(Trial {
                candidate_id: member.candidate_id.clone(),
                ancestry: candidate.ancestry.clone(),
                rng_seed: manifest.rng_seed,
                attempt,
                tape: tape.clone(),
                logical_tick_budget: member.frame_count,
                boot: decoded.tape.boot.clone(),
                suffix_tape: None,
                state: root.join("state"),
                milestones: root.join("milestones.json"),
                gameplay_trace: (attempt == 1).then(|| root.join("gameplay.trace")),
                stdout: root.join("stdout.txt"),
                stderr: root.join("stderr.txt"),
                root,
            });
        }
    }
    Ok(trials)
}

pub(super) fn build_anchored_trials(
    manifest: &PopulationManifest,
    population_root: &Path,
    output_root: &Path,
    repetitions: u32,
    objective: &PreparedAnchoredObjective,
) -> Result<Vec<Trial>, EvaluateError> {
    let mut trials = Vec::with_capacity(manifest.members.len() * repetitions as usize);
    for member in &manifest.members {
        if member.candidate_id.is_empty()
            || !member
                .candidate_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(EvaluateError::InvalidManifest(format!(
                "candidate ID {:?} is unsafe",
                member.candidate_id
            )));
        }
        let suffix_path = fs::canonicalize(population_root.join(&member.tape_file))?;
        if !suffix_path.starts_with(population_root) {
            return Err(EvaluateError::InvalidManifest(format!(
                "candidate {} tape escapes the population directory",
                member.candidate_id
            )));
        }
        let candidate_path = fs::canonicalize(population_root.join(&member.candidate_file))?;
        if !candidate_path.starts_with(population_root) {
            return Err(EvaluateError::InvalidManifest(format!(
                "candidate {} source escapes the population directory",
                member.candidate_id
            )));
        }
        let candidate: Candidate = serde_json::from_slice(&fs::read(&candidate_path)?)?;
        candidate.validate()?;
        let suffix_bytes = fs::read(&suffix_path)?;
        let suffix = InputTape::decode(&suffix_bytes)?.tape;
        let compiled = candidate.compile()?;
        if candidate.segment != manifest.segment
            || candidate.boot != manifest.boot
            || candidate.id()? != member.candidate_id
            || candidate.ancestry != member.ancestry
            || compiled.frames.len() as u64 != member.frame_count
            || compiled.encode()? != suffix_bytes
        {
            return Err(EvaluateError::InvalidManifest(format!(
                "candidate {} ID, ancestry, frame count, source, and tape are not one content identity",
                member.candidate_id
            )));
        }
        let chained = concatenate(vec![
            ChainSegment::all(objective.prefix.clone()),
            ChainSegment::all(suffix),
        ])
        .map_err(|error| EvaluateError::InvalidManifest(error.to_string()))?;
        let logical_tick_budget = u64::try_from(chained.tape.frames.len()).map_err(|_| {
            EvaluateError::InvalidManifest(format!(
                "candidate {} chained tape length does not fit u64",
                member.candidate_id
            ))
        })?;
        for attempt in 1..=repetitions {
            let root = output_root
                .join("candidates")
                .join(&member.candidate_id)
                .join(format!("attempt-{attempt:03}"));
            fs::create_dir_all(&root)?;
            let full_tape = root.join("full.tape");
            fs::write(&full_tape, chained.tape.encode()?)?;
            trials.push(Trial {
                candidate_id: member.candidate_id.clone(),
                ancestry: candidate.ancestry.clone(),
                rng_seed: manifest.rng_seed,
                attempt,
                tape: full_tape,
                logical_tick_budget,
                boot: chained.tape.boot.clone(),
                suffix_tape: Some(suffix_path.clone()),
                state: root.join("state"),
                milestones: root.join("milestones.json"),
                gameplay_trace: (attempt == 1).then(|| root.join("gameplay.trace")),
                stdout: root.join("stdout.txt"),
                stderr: root.join("stderr.txt"),
                root,
            });
        }
    }
    Ok(trials)
}

pub(super) fn run_trial(
    config: &EvaluateConfig,
    segment: SegmentProfile,
    trial: &Trial,
    worker_id: &str,
    global_cancel: &AtomicBool,
    anchored: Option<&PreparedAnchoredObjective>,
) -> AttemptEvidence {
    let started = Instant::now();
    let mut evidence = AttemptEvidence {
        schema: ATTEMPT_SCHEMA,
        candidate_id: trial.candidate_id.clone(),
        ancestry: trial.ancestry.clone(),
        attempt: trial.attempt,
        worker_id: worker_id.into(),
        segment,
        boot: trial.boot.clone(),
        tape: trial.tape.clone(),
        realized_tape: None,
        suffix_tape: trial.suffix_tape.clone(),
        artifact_root: trial.root.clone(),
        harness_request: None,
        harness_request_sha256: None,
        harness_result: None,
        harness_result_sha256: None,
        harness_terminal: None,
        state_root: trial.state.clone(),
        milestone_result: trial.milestones.clone(),
        gameplay_trace: None,
        gameplay_trace_blob: None,
        gameplay_trace_error: None,
        transition_corpus: None,
        transition_evidence: None,
        episode_manifest: None,
        immutable_episode: None,
        dataset_source: None,
        transition_count: None,
        transition_corpus_error: None,
        stdout: trial.stdout.clone(),
        stderr: trial.stderr.clone(),
        elapsed_millis: 0,
        exit_code: None,
        timed_out: false,
        cancelled: false,
        infrastructure_error: None,
        outcome: EpisodeOutcome {
            class: EpisodeOutcomeClass::Failed,
            reason: "trial has not completed".into(),
        },
        crash_artifacts: Vec::new(),
        milestone_depth: 0,
        deepest_milestone: "none".into(),
        first_hit_tick: None,
        goal_reached: false,
        milestone_observations: BTreeMap::new(),
        boundary_fingerprints: BTreeMap::new(),
        value_projections: BTreeMap::new(),
    };
    let mut run = || -> Result<TrialScore, EvaluateError> {
        if let Some(harness) = &config.harness {
            return run_harness_trial(
                harness,
                segment,
                trial,
                global_cancel,
                anchored,
                &mut evidence,
            );
        }
        fs::create_dir_all(&trial.state)?;
        let stdout = File::create(&trial.stdout)?;
        let stderr = File::create(&trial.stderr)?;
        let mut command = Command::new(&config.game);
        command
            .current_dir(&config.working_directory)
            .args(&config.game_args_prefix)
            .arg("--dvd")
            .arg(&config.dvd);
        let (milestone_list, goal) = if let Some(objective) = anchored {
            command
                .arg("--milestone-program")
                .arg(&objective.runtime_program);
            (
                format!(
                    "{},{}",
                    objective.identity.source_milestone, objective.identity.goal_milestone
                ),
                objective.identity.goal_milestone.clone(),
            )
        } else {
            match segment {
                SegmentProfile::BootToFsp103 => (
                    "gameplay-ready-f-sp103".into(),
                    "gameplay-ready-f-sp103".into(),
                ),
                SegmentProfile::Fsp103ToFsp104 => (
                    "gameplay-ready-f-sp103,exit-f-sp103-to-f-sp104,entered-f-sp104".into(),
                    "entered-f-sp104".into(),
                ),
                SegmentProfile::LinkControlToTunnelCrawlStart => unreachable!(
                    "anchored profiles are evaluated through evaluate_anchored_population"
                ),
            }
        };
        command
            .arg("--input-tape")
            .arg(&trial.tape)
            .arg("--input-tape-end")
            .arg("hold")
            .arg("--automation-tick-budget")
            .arg(trial.logical_tick_budget.to_string())
            .arg("--automation-data-root")
            .arg(&trial.state)
            .arg("--milestones")
            .arg(&milestone_list)
            .arg("--milestone-goal")
            .arg(&goal)
            .arg("--milestone-result")
            .arg(&trial.milestones);
        if let Some(gameplay_trace) = &trial.gameplay_trace {
            command
                .arg("--gameplay-trace")
                .arg(gameplay_trace)
                .arg("--gameplay-trace-channels")
                .arg("all");
        }
        command
            .arg("--cvar")
            .arg("game.instantSaves=true")
            .arg("--cvar")
            .arg("backend.cardFileType=1")
            .arg("--cvar")
            .arg("backend.wasPresetChosen=true")
            .arg("--cvar")
            .arg("game.enableMenuPointer=false")
            .arg("--headless")
            .arg("--fixed-step")
            .arg("--exit-after-tape")
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));
        hide_window(&mut command);
        let mut child = command.spawn().map_err(EvaluateError::Launch)?;
        let status = loop {
            if global_cancel.load(Ordering::Acquire) {
                evidence.cancelled = true;
                let _ = child.kill();
                let _ = child.wait();
                return Err(EvaluateError::Cancelled);
            }
            if started.elapsed() >= config.timeout {
                evidence.timed_out = true;
                let _ = child.kill();
                let _ = child.wait();
                return Err(EvaluateError::Timeout(config.timeout));
            }
            match child.try_wait()? {
                Some(status) => {
                    evidence.exit_code = status.code();
                    break status;
                }
                None => thread::sleep(Duration::from_millis(10)),
            }
        };
        let score = if let Some(objective) = anchored {
            parse_anchored_milestones(&trial.milestones, objective, &trial.boot)
        } else {
            parse_native_milestones(&trial.milestones, segment, &trial.boot)
        }?;
        validate_native_exit(status, score.goal_reached)?;
        Ok(score)
    };
    match run() {
        Ok(score) => {
            evidence.milestone_depth = score.depth;
            evidence.deepest_milestone = score.deepest;
            evidence.first_hit_tick = score.score_tick;
            evidence.goal_reached = score.goal_reached;
            evidence.milestone_observations = score.milestone_observations;
            evidence.boundary_fingerprints = score.boundary_fingerprints;
            evidence.value_projections = score.value_projections;
        }
        Err(error) => evidence.infrastructure_error = Some(error.to_string()),
    }
    let gameplay_trace = evidence
        .gameplay_trace
        .clone()
        .or_else(|| trial.gameplay_trace.clone());
    evidence.gameplay_trace = None;
    if let Some(path) = gameplay_trace {
        match fs::read(&path)
            .map_err(|error| error.to_string())
            .and_then(|bytes| crate::trace::decode(&bytes).map_err(|error| error.to_string()))
            .and_then(|trace| {
                let expected_boot = evidence
                    .realized_tape
                    .as_ref()
                    .map(|path| {
                        fs::read(path)
                            .map_err(|error| error.to_string())
                            .and_then(|bytes| {
                                InputTape::decode(&bytes)
                                    .map(|decoded| decoded.tape.boot)
                                    .map_err(|error| error.to_string())
                            })
                    })
                    .transpose()?
                    .unwrap_or_else(|| trial.boot.clone());
                if trace.boot != expected_boot {
                    Err(format!(
                        "gameplay trace boot origin {:?} does not match realized tape origin {:?}",
                        trace.boot, expected_boot
                    ))
                } else if trace.capacity_exhausted {
                    Err("gameplay trace capacity was exhausted".into())
                } else if trace.records.is_empty() {
                    Err("gameplay trace contains no records".into())
                } else {
                    Ok(())
                }
            }) {
            Ok(()) => evidence.gameplay_trace = Some(path),
            Err(error) => evidence.gameplay_trace_error = Some(error),
        }
    }
    evidence.outcome = classify_attempt_outcome(&evidence);
    if trial.attempt == 1
        && evidence.infrastructure_error.is_none()
        && evidence.gameplay_trace.is_some()
        && let Some(objective) = anchored
    {
        match extract_trial_transition_corpus(trial, &evidence, objective) {
            Ok((
                path,
                evidence_path,
                episode_manifest,
                immutable_episode,
                dataset_source,
                count,
            )) => {
                evidence.transition_corpus = Some(path);
                evidence.transition_evidence = Some(evidence_path);
                evidence.episode_manifest = Some(episode_manifest);
                evidence.immutable_episode = immutable_episode;
                evidence.dataset_source = Some(dataset_source);
                evidence.transition_count = Some(count);
            }
            Err(error) => evidence.transition_corpus_error = Some(error),
        }
    }
    evidence.elapsed_millis = started.elapsed().as_millis();
    evidence
}

fn run_harness_trial(
    harness: &HarnessEvaluateConfig,
    segment: SegmentProfile,
    trial: &Trial,
    global_cancel: &AtomicBool,
    anchored: Option<&PreparedAnchoredObjective>,
    evidence: &mut AttemptEvidence,
) -> Result<TrialScore, EvaluateError> {
    if global_cancel.load(Ordering::Acquire) {
        evidence.cancelled = true;
        return Err(EvaluateError::Cancelled);
    }
    let repository_root = fs::canonicalize(&harness.repository_root)?;
    fs::create_dir_all(&trial.root)?;
    let trial_root = fs::canonicalize(&trial.root)?;
    let artifact_root = trial_root.join("harness");
    let destination = artifact_root
        .strip_prefix(&repository_root)
        .map_err(|_| {
            EvaluateError::InvalidConfig(format!(
                "search output must be beneath the harness repository root: {}",
                artifact_root.display()
            ))
        })?
        .to_str()
        .ok_or_else(|| EvaluateError::InvalidConfig("search output path is not UTF-8".into()))?
        .replace(std::path::MAIN_SEPARATOR, "/");
    let request = derive_candidate_request(
        &harness.request_template,
        &repository_root,
        &trial.tape,
        &destination,
        trial.rng_seed,
    )?;
    if let Some(objective) = anchored
        && (request.objective.goal != objective.identity.goal_milestone
            || request.objective.program_sha256.to_string()
                != objective.identity.milestone_program_sha256)
    {
        return Err(EvaluateError::InvalidConfig(
            "anchored search request template does not bind the prepared objective".into(),
        ));
    }

    let request_path = trial_root.join("request.json");
    write_json(&request_path, &request)?;
    let result = execute_request(&request, &repository_root, trial.attempt)
        .map_err(|error| EvaluateError::NativeResult(error.to_string()))?;
    let result_path = artifact_root.join("result.json");
    evidence.artifact_root = artifact_root.clone();
    evidence.harness_request = Some(request_path);
    evidence.harness_request_sha256 = Some(request.content_sha256);
    evidence.harness_result = Some(result_path);
    evidence.harness_result_sha256 = Some(result.content_sha256);
    evidence.harness_terminal = Some(result.terminal);
    evidence.state_root = artifact_root.join("state");
    evidence.milestone_result =
        harness_artifact_path(&artifact_root, result.artifacts.objective_result.as_ref())
            .unwrap_or_else(|| artifact_root.join("objective.json"));
    evidence.gameplay_trace =
        harness_artifact_path(&artifact_root, result.artifacts.gameplay_trace.as_ref());
    evidence.realized_tape =
        harness_artifact_path(&artifact_root, result.artifacts.realized_input.as_ref());
    evidence.stdout = harness_artifact_path(&artifact_root, result.artifacts.stdout.as_ref())
        .unwrap_or_else(|| artifact_root.join("stdout.txt"));
    evidence.stderr = harness_artifact_path(&artifact_root, result.artifacts.stderr.as_ref())
        .unwrap_or_else(|| artifact_root.join("stderr.txt"));
    evidence.elapsed_millis = u128::from(result.timing.host_elapsed_millis);
    evidence.exit_code = match result.terminal {
        HarnessTerminalReason::Reached => Some(0),
        HarnessTerminalReason::Exhausted | HarnessTerminalReason::TargetLost => {
            Some(NATIVE_GOAL_MISS_EXIT_CODE)
        }
        _ => None,
    };
    evidence.timed_out = result.terminal == HarnessTerminalReason::HostTimeout;
    evidence.cancelled = result.terminal == HarnessTerminalReason::Cancelled;

    let score_boot = evidence
        .realized_tape
        .as_ref()
        .map(|path| -> Result<TapeBoot, EvaluateError> {
            Ok(InputTape::decode(&fs::read(path)?)?.tape.boot)
        })
        .transpose()?
        .unwrap_or_else(|| trial.boot.clone());

    match result.terminal {
        HarnessTerminalReason::Reached | HarnessTerminalReason::Exhausted => score_harness_result(
            &result,
            &request,
            &evidence.milestone_result,
            &score_boot,
            segment,
            anchored,
        ),
        _ => Ok(empty_harness_score()),
    }
}

fn harness_artifact_path(
    artifact_root: &Path,
    reference: Option<&ArtifactReference>,
) -> Option<PathBuf> {
    reference.map(|reference| artifact_root.join(&reference.path))
}

pub(super) fn classify_attempt_outcome(evidence: &AttemptEvidence) -> EpisodeOutcome {
    if let Some(terminal) = evidence.harness_terminal {
        let class = match terminal {
            HarnessTerminalReason::Reached => EpisodeOutcomeClass::Successful,
            HarnessTerminalReason::Exhausted
            | HarnessTerminalReason::Impossible
            | HarnessTerminalReason::TargetLost
            | HarnessTerminalReason::Rejected => EpisodeOutcomeClass::Failed,
            HarnessTerminalReason::Unsupported | HarnessTerminalReason::CapabilityMismatch => {
                EpisodeOutcomeClass::Unsupported
            }
            HarnessTerminalReason::HostTimeout | HarnessTerminalReason::Hung => {
                EpisodeOutcomeClass::TimedOut
            }
            HarnessTerminalReason::WorkerCrashed | HarnessTerminalReason::GameCrashed => {
                EpisodeOutcomeClass::Crashed
            }
            HarnessTerminalReason::IdentityMismatch
            | HarnessTerminalReason::ProtocolFailure
            | HarnessTerminalReason::Nondeterministic => EpisodeOutcomeClass::Desynced,
            HarnessTerminalReason::Cancelled => EpisodeOutcomeClass::Failed,
        };
        return EpisodeOutcome {
            class,
            reason: format!("core harness terminal: {}", terminal.name()),
        };
    }
    classify_outcome(
        evidence.timed_out,
        evidence.cancelled,
        evidence.gameplay_trace_error.as_deref(),
        evidence.infrastructure_error.as_deref(),
        evidence.goal_reached,
    )
}

pub(super) fn classify_outcome(
    timed_out: bool,
    cancelled: bool,
    gameplay_trace_error: Option<&str>,
    infrastructure_error: Option<&str>,
    goal_reached: bool,
) -> EpisodeOutcome {
    if timed_out {
        return EpisodeOutcome {
            class: EpisodeOutcomeClass::TimedOut,
            reason: "evaluation timeout expired".into(),
        };
    }
    if gameplay_trace_error.is_some_and(|reason| reason.contains("capacity was exhausted")) {
        return EpisodeOutcome {
            class: EpisodeOutcomeClass::Truncated,
            reason: gameplay_trace_error.unwrap().into(),
        };
    }
    if let Some(reason) = infrastructure_error {
        let class = if reason.starts_with("could not launch Dusklight") {
            EpisodeOutcomeClass::Unsupported
        } else if reason.contains("worker exit") {
            EpisodeOutcomeClass::Crashed
        } else if reason.starts_with("invalid native milestone result")
            || reason.starts_with("invalid search result")
        {
            EpisodeOutcomeClass::Desynced
        } else if cancelled {
            EpisodeOutcomeClass::Failed
        } else {
            EpisodeOutcomeClass::Crashed
        };
        return EpisodeOutcome {
            class,
            reason: reason.into(),
        };
    }
    if goal_reached {
        EpisodeOutcome {
            class: EpisodeOutcomeClass::Successful,
            reason: "objective reached".into(),
        }
    } else {
        EpisodeOutcome {
            class: EpisodeOutcomeClass::Failed,
            reason: "objective not reached".into(),
        }
    }
}

type ExtractedTrialTransitionCorpus = (PathBuf, PathBuf, PathBuf, Option<PathBuf>, PathBuf, u64);

fn extract_trial_transition_corpus(
    trial: &Trial,
    evidence: &AttemptEvidence,
    objective: &PreparedAnchoredObjective,
) -> Result<ExtractedTrialTransitionCorpus, String> {
    let trace_path = evidence
        .gameplay_trace
        .as_ref()
        .ok_or_else(|| "validated gameplay trace is missing".to_string())?;
    let trace_bytes = fs::read(trace_path).map_err(|error| error.to_string())?;
    let decoded = crate::trace::decode(&trace_bytes).map_err(|error| error.to_string())?;
    let start_tape_frame = objective
        .identity
        .source_tape_frame
        .checked_add(1)
        .ok_or_else(|| "learning range start overflows".to_string())?;
    let end_tape_frame = if evidence.goal_reached {
        evidence
            .milestone_observations
            .get(&objective.identity.goal_milestone)
            .map(|observation| observation.tape_frame)
            .ok_or_else(|| "goal hit lacks a tape-frame observation".to_string())?
    } else {
        decoded
            .records
            .last()
            .and_then(|record| record.tape_frame)
            .ok_or_else(|| "goal miss trace lacks a final tape frame".to_string())?
    };
    if end_tape_frame < start_tape_frame {
        return Err(format!(
            "learning range {start_tape_frame}..={end_tape_frame} is empty"
        ));
    }
    let episode_tape_path = evidence.realized_tape.as_ref().unwrap_or(&trial.tape);
    let tape_bytes = fs::read(episode_tape_path).map_err(|error| error.to_string())?;
    let decoded_tape = InputTape::decode(&tape_bytes)
        .map_err(|error| error.to_string())?
        .tape;
    let run_request = evidence
        .harness_request
        .as_ref()
        .map(|path| {
            serde_json::from_slice::<HarnessRunRequest>(
                &fs::read(path).map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())
        })
        .transpose()?;
    if let Some(request) = &run_request {
        request
            .validate()
            .map_err(|error| format!("episode harness request is invalid: {error}"))?;
    }
    let start_reference = learning_boundary_reference(
        &objective.identity.digest,
        &objective.identity.source_milestone,
        &objective.identity.source_boundary_fingerprint,
    );
    let terminal_reference = if evidence.goal_reached {
        let boundary = evidence
            .boundary_fingerprints
            .get(&objective.identity.goal_milestone)
            .ok_or_else(|| "goal hit lacks a terminal boundary fingerprint".to_string())?;
        Some(learning_boundary_reference(
            &objective.identity.digest,
            &objective.identity.goal_milestone,
            &boundary.digest,
        ))
    } else {
        None
    };
    let episode_digest = learning_episode_digest(
        &objective.identity.digest,
        &trial.candidate_id,
        &trace_bytes,
    );
    let corpus = extract_exploratory_v2_from_bytes(
        &trace_bytes,
        &tape_bytes,
        ExploratoryExtractConfig {
            episode_digest,
            start_tape_frame,
            end_tape_frame,
            start_reference: Some(start_reference),
            terminal_reference,
            end_is_terminal: evidence.goal_reached,
        },
    )
    .map_err(|error| error.to_string())?;
    let count = u64::try_from(corpus.transitions.len())
        .map_err(|_| "transition count does not fit u64".to_string())?;
    let path = trial.root.join("transitions.dtcz");
    let transition_evidence = TransitionEvidenceBundle::build(TransitionEvidenceBuild {
        corpus: &corpus,
        trace: &decoded,
        tape: &decoded_tape,
        trace_sha256: ArtifactDigest(Sha256::digest(&trace_bytes).into()),
        tape_sha256: ArtifactDigest(Sha256::digest(&tape_bytes).into()),
        start_tape_frame,
        end_tape_frame,
        terminal_reason: evidence
            .goal_reached
            .then_some(TerminalReasonEvidence::ObjectiveReached),
    })
    .map_err(|error| error.to_string())?;
    let transition_evidence_bytes =
        serde_json::to_vec_pretty(&transition_evidence).map_err(|error| error.to_string())?;
    let intervention_offset = objective.prefix.frames.len() as u64;
    let intervention = trial
        .ancestry
        .intervention
        .as_ref()
        .map(|value| EpisodeIntervention {
            start_frame: intervention_offset.saturating_add(value.start_frame),
            end_frame_exclusive: intervention_offset.saturating_add(value.end_frame_exclusive),
            parent_end_frame_exclusive: intervention_offset
                .saturating_add(value.parent_end_frame_exclusive),
            description: trial
                .ancestry
                .mutation
                .clone()
                .unwrap_or_else(|| "candidate intervention".into()),
        });
    let producer_kind = if let Some(mutation) = trial.ancestry.mutation.as_deref() {
        if mutation.starts_with("q_") {
            EpisodeProducerKind::FittedQ
        } else if mutation.starts_with("structured_counterfactual") {
            EpisodeProducerKind::StructuredCounterfactual
        } else if mutation.starts_with("archive_novelty") {
            EpisodeProducerKind::ArchiveNovelty
        } else if mutation.starts_with("blind_") {
            EpisodeProducerKind::BlindCoverage
        } else if mutation.starts_with("systematic_probe") {
            EpisodeProducerKind::SystematicProbe
        } else if mutation.starts_with("random_probe") {
            EpisodeProducerKind::RandomProbe
        } else if mutation.starts_with("latin_hypercube") {
            EpisodeProducerKind::LatinHypercube
        } else if trial.ancestry.generation == 0 && trial.ancestry.parent_id.is_none() {
            EpisodeProducerKind::Seed
        } else {
            EpisodeProducerKind::Evolution
        }
    } else if trial.ancestry.generation == 0 && trial.ancestry.parent_id.is_none() {
        EpisodeProducerKind::Seed
    } else {
        EpisodeProducerKind::Evolution
    };
    let context = EpisodeContext {
        schema: EPISODE_CONTEXT_SCHEMA_V1.into(),
        run_identity: run_request.as_ref().map(|request| request.identity.clone()),
        run_build: run_request.as_ref().map_or_else(
            || {
                Ok::<_, String>(RunBuildIdentity {
                    executable_sha256: objective
                        .identity
                        .game_sha256
                        .parse()
                        .map_err(|error| format!("invalid objective game digest: {error}"))?,
                    dusklight_commit: None,
                    aurora_commit: None,
                    target: Some(format!(
                        "{}-{}",
                        std::env::consts::ARCH,
                        std::env::consts::OS
                    )),
                    profile: None,
                    feature_digest: None,
                })
            },
            |request| {
                Ok(RunBuildIdentity {
                    executable_sha256: request.executable.sha256,
                    dusklight_commit: Some(request.build.dusklight_commit.clone()),
                    aurora_commit: Some(request.build.aurora_commit.clone()),
                    target: Some(request.build.target.clone()),
                    profile: Some(request.build.profile.clone()),
                    feature_digest: Some(request.build.feature_digest),
                })
            },
        )?,
        objective: EpisodeObjectiveIdentity {
            id: format!(
                "{}:{}",
                objective.identity.segment.as_str(),
                objective.identity.goal_milestone
            ),
            digest: objective
                .identity
                .digest
                .parse()
                .map_err(|error| format!("invalid objective digest: {error}"))?,
        },
        producer: EpisodeProducerIdentity {
            kind: producer_kind,
            name: "huntctl".into(),
            version: env!("CARGO_PKG_VERSION").into(),
        },
        seed: EpisodeSeed::Deterministic {
            value: trial.rng_seed,
        },
        worker_id: evidence.worker_id.clone(),
        lineage: EpisodeLineage {
            candidate_id: Some(trial.candidate_id.clone()),
            parent_candidate_id: trial.ancestry.parent_id.clone(),
            generation: trial.ancestry.generation,
            intervention,
        },
        outcome: evidence.outcome.clone(),
    };
    let transition_evidence_sha256 =
        ArtifactDigest(Sha256::digest(&transition_evidence_bytes).into());
    let episode_manifest = EpisodeManifest::build(EpisodeManifestBuild {
        context: &context,
        boot: &decoded_tape.boot,
        corpus: &corpus,
        query_view_id: "movement-state/v1",
        tape_sha256: ArtifactDigest(Sha256::digest(&tape_bytes).into()),
        trace_sha256: ArtifactDigest(Sha256::digest(&trace_bytes).into()),
        transition_evidence_sha256,
    })
    .map_err(|error| error.to_string())?;
    let evidence_path = trial.root.join("transitions.dtcz.evidence.json");
    let episode_manifest_path = trial.root.join("episode.json");
    let immutable_episode_path = trial.root.join("immutable-episode.json");
    let dataset_source_path = trial.root.join("dataset-source.json");
    corpus
        .write_zstd_file(&path, 3)
        .map_err(|error| error.to_string())?;
    fs::write(&evidence_path, transition_evidence_bytes).map_err(|error| error.to_string())?;
    fs::write(
        &episode_manifest_path,
        serde_json::to_vec_pretty(&episode_manifest).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let immutable_episode_path = if let Some(terminal) = evidence.harness_terminal {
        let result_path = evidence
            .harness_result
            .as_ref()
            .ok_or_else(|| "harness episode is missing its sealed result path".to_string())?;
        let result: HarnessRunResult =
            serde_json::from_slice(&fs::read(result_path).map_err(|error| error.to_string())?)
                .map_err(|error| error.to_string())?;
        if result.terminal != terminal
            || Some(result.content_sha256) != evidence.harness_result_sha256
        {
            return Err("harness episode terminal or result identity changed".into());
        }
        let immutable_episode = ImmutableEpisodeArtifact::build(ImmutableEpisodeBuild {
            manifest: &episode_manifest,
            corpus: &corpus,
            evidence: &transition_evidence,
            transition_evidence_sha256,
            terminal,
            terminal_detail: &result.detail.message,
        })
        .map_err(|error| error.to_string())?;
        fs::write(
            &immutable_episode_path,
            serde_json::to_vec_pretty(&immutable_episode).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        Some(immutable_episode_path)
    } else {
        None
    };
    fs::write(
        &dataset_source_path,
        serde_json::to_vec_pretty(&DatasetSourceDescriptor {
            schema: DATASET_SOURCE_SCHEMA_V1.into(),
            source_id: episode_manifest.episode_sha256.to_string(),
            episode_manifest: fs::canonicalize(&episode_manifest_path)
                .map_err(|error| error.to_string())?,
            transition_corpus: fs::canonicalize(&path).map_err(|error| error.to_string())?,
            absolute_tape: fs::canonicalize(episode_tape_path)
                .map_err(|error| error.to_string())?,
            transition_evidence: fs::canonicalize(&evidence_path)
                .map_err(|error| error.to_string())?,
            gameplay_trace: fs::canonicalize(trace_path).map_err(|error| error.to_string())?,
            route_family: episode_manifest.objective.id.clone(),
            screenshot_sha256: Vec::new(),
            checkpoint_sha256: Vec::new(),
        })
        .map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    Ok((
        path,
        evidence_path,
        episode_manifest_path,
        immutable_episode_path,
        dataset_source_path,
        count,
    ))
}

fn learning_episode_digest(
    objective_digest: &str,
    candidate_id: &str,
    trace_bytes: &[u8],
) -> ArtifactDigest {
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight.search-learning-episode/v1\0");
    for bytes in [
        objective_digest.as_bytes(),
        candidate_id.as_bytes(),
        trace_bytes,
    ] {
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
    }
    ArtifactDigest(hasher.finalize().into())
}

pub(super) fn write_episode_ledger(
    output_root: &Path,
    attempts: &[AttemptEvidence],
) -> Result<Option<PathBuf>, EvaluateError> {
    let mut ledger = EpisodeLedger::new();
    let mut candidate_inputs = BTreeMap::new();
    for attempt in attempts {
        let (Some(manifest_path), Some(corpus_path)) =
            (&attempt.episode_manifest, &attempt.transition_corpus)
        else {
            continue;
        };
        let manifest: EpisodeManifest = serde_json::from_slice(&fs::read(manifest_path)?)?;
        let corpus = TransitionCorpus::read_zstd_file(corpus_path)
            .map_err(|error| EvaluateError::InvalidResult(error.to_string()))?;
        manifest
            .validate(&corpus)
            .map_err(|error| EvaluateError::InvalidResult(error.to_string()))?;
        if let Some(previous) =
            candidate_inputs.insert(attempt.candidate_id.clone(), manifest.input_identity_sha256)
            && previous != manifest.input_identity_sha256
        {
            return Err(EvaluateError::InvalidResult(format!(
                "candidate {} produced conflicting episode input identities",
                attempt.candidate_id
            )));
        }
        ledger.ingest_episode(&manifest, manifest_path.clone());
    }
    if ledger.groups.is_empty() {
        return Ok(None);
    }
    for attempt in attempts {
        let Some(input_identity) = candidate_inputs.get(&attempt.candidate_id).copied() else {
            continue;
        };
        let proof_path = attempt.artifact_root.join("attempt.json");
        let proof_bytes = fs::read(&proof_path)?;
        ledger
            .ingest_proof(
                input_identity,
                ArtifactDigest(Sha256::digest(&proof_bytes).into()),
                proof_path,
                attempt.worker_id.clone(),
                attempt.attempt,
                attempt.outcome.clone(),
            )
            .map_err(|error| EvaluateError::InvalidResult(error.to_string()))?;
    }
    ledger
        .validate()
        .map_err(|error| EvaluateError::InvalidResult(error.to_string()))?;
    let path = output_root.join("episodes.json");
    write_json(&path, &ledger)?;
    Ok(Some(path))
}

pub(super) fn address_attempt_artifacts(
    output_root: &Path,
    attempts: &mut [AttemptEvidence],
) -> Result<(), EvaluateError> {
    let store = ContentStore::initialize(output_root.join("content"))
        .map_err(|error| EvaluateError::InvalidResult(error.to_string()))?;
    for attempt in attempts {
        if let Some(path) = &attempt.gameplay_trace {
            attempt.gameplay_trace_blob = Some(
                store
                    .put_file(path, ContentKind::GameplayTrace)
                    .map_err(|error| EvaluateError::InvalidResult(error.to_string()))?,
            );
        }
        if matches!(
            attempt.outcome.class,
            EpisodeOutcomeClass::Crashed
                | EpisodeOutcomeClass::TimedOut
                | EpisodeOutcomeClass::Desynced
                | EpisodeOutcomeClass::Unsupported
                | EpisodeOutcomeClass::Truncated
        ) {
            let mut paths = vec![
                attempt.stdout.clone(),
                attempt.stderr.clone(),
                attempt.milestone_result.clone(),
            ];
            if attempt.gameplay_trace.is_none() {
                paths.push(attempt.artifact_root.join("gameplay.trace"));
            }
            for path in paths {
                if fs::metadata(&path)
                    .is_ok_and(|metadata| metadata.is_file() && metadata.len() > 0)
                {
                    let blob = store
                        .put_file(&path, ContentKind::CrashArtifact)
                        .map_err(|error| EvaluateError::InvalidResult(error.to_string()))?;
                    if !attempt
                        .crash_artifacts
                        .iter()
                        .any(|existing| existing.sha256 == blob.sha256)
                    {
                        attempt.crash_artifacts.push(blob);
                    }
                }
            }
        }
        write_json(&attempt.artifact_root.join("attempt.json"), attempt)?;
    }
    Ok(())
}

pub(super) fn semantic_novelty_descriptor(
    evidence: &AttemptEvidence,
) -> Result<Option<SemanticNoveltyDescriptor>, EvaluateError> {
    evidence
        .gameplay_trace
        .as_ref()
        .map(|path| {
            let bytes = fs::read(path)?;
            let trace = crate::trace::decode(&bytes)
                .map_err(|error| EvaluateError::InvalidResult(error.to_string()))?;
            let boundaries = evidence
                .boundary_fingerprints
                .iter()
                .map(|(name, value)| BoundaryFingerprintFact {
                    name: name.clone(),
                    schema: value.schema.clone(),
                    algorithm: value.algorithm.clone(),
                    canonical_encoding: value.canonical_encoding.clone(),
                    digest: value.digest.clone(),
                })
                .collect();
            SemanticNoveltyDescriptor::from_trace(&trace, boundaries)
                .map_err(|error| EvaluateError::InvalidResult(error.to_string()))
        })
        .transpose()
}

pub(super) fn archive_behavior_context(
    evidence: &AttemptEvidence,
    descriptor: &SemanticNoveltyDescriptor,
) -> BehaviorContext {
    let axes = descriptor.axis_identities();
    let mut context = archive_behavior_context_from_evidence(
        &evidence.boundary_fingerprints,
        &evidence.value_projections,
        axes.contacts,
    );
    context.procedure_sequence_identity = axes.procedure_sequence;
    context.event_sequence_identity = axes.event_sequence;
    context.state_transition_identity = axes.state_transitions;
    context.actor_relationship_identity = axes.actor_relationships;
    context.flag_state_identity = axes.flags;
    context.kinematic_extrema_identity = axes.kinematic_extrema;
    context
}

pub(super) fn archive_behavior_context_from_evidence(
    boundaries: &BTreeMap<String, BoundaryFingerprint>,
    projections: &BTreeMap<String, BTreeMap<String, ValueProjectionEvidence>>,
    contact_behavior_identity: Option<String>,
) -> BehaviorContext {
    let mut rng = Vec::new();
    let mut actors = Vec::new();
    let mut downstream_boundaries = Vec::new();
    let mut downstream = Vec::new();
    for (milestone, milestone_projections) in projections {
        for (name, projection) in milestone_projections {
            let Some(fingerprint) = projection.value_fingerprint.as_ref() else {
                continue;
            };
            if !projection.available {
                continue;
            }
            let encoded = serde_json::to_vec(&(
                milestone,
                name,
                &projection.identity,
                &fingerprint.schema,
                &fingerprint.algorithm,
                &fingerprint.canonical_encoding,
                &fingerprint.digest,
            ))
            .expect("validated value-projection identity is serializable");
            if projection
                .values
                .iter()
                .any(|value| value.get("kind").and_then(serde_json::Value::as_str) == Some("rng"))
            {
                rng.push(encoded.clone());
            }
            if projection.values.iter().any(|value| {
                value.get("kind").and_then(serde_json::Value::as_str) == Some("actor_population")
            }) {
                actors.push(encoded.clone());
            }
            downstream.push(encoded);
        }
    }
    for (milestone, fingerprint) in boundaries {
        let encoded = serde_json::to_vec(&(
            milestone,
            &fingerprint.schema,
            &fingerprint.algorithm,
            &fingerprint.canonical_encoding,
            &fingerprint.digest,
        ))
        .expect("validated boundary identity is serializable");
        downstream_boundaries.push(encoded.clone());
        downstream.push(encoded);
    }
    BehaviorContext {
        procedure_sequence_identity: None,
        event_sequence_identity: None,
        state_transition_identity: None,
        actor_relationship_identity: None,
        flag_state_identity: None,
        kinematic_extrema_identity: None,
        objective_rng_identity: archive_axis_identity(b"rng/v1", &rng),
        actor_population_identity: archive_axis_identity(b"actors/v1", &actors),
        contact_behavior_identity,
        boundary_state_identity: archive_axis_identity(b"boundaries/v1", &downstream_boundaries),
        downstream_state_identity: archive_axis_identity(b"downstream/v1", &downstream),
    }
}

fn archive_axis_identity(domain: &[u8], entries: &[Vec<u8>]) -> Option<String> {
    if entries.is_empty() {
        return None;
    }
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight-behavior-archive-axis/v1\0");
    hasher.update((domain.len() as u64).to_le_bytes());
    hasher.update(domain);
    for entry in entries {
        hasher.update((entry.len() as u64).to_le_bytes());
        hasher.update(entry);
    }
    Some(format!("{:x}", hasher.finalize()))
}

fn learning_boundary_reference(
    objective_digest: &str,
    milestone: &str,
    boundary_fingerprint: &str,
) -> StateReference {
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight.search-learning-boundary/v1\0");
    for value in [objective_digest, milestone, boundary_fingerprint] {
        hasher.update((value.len() as u64).to_le_bytes());
        hasher.update(value.as_bytes());
    }
    StateReference {
        kind: StateReferenceKind::Boundary,
        digest: ArtifactDigest(hasher.finalize().into()),
    }
}
