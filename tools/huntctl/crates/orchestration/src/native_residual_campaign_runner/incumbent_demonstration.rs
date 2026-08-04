//! Incumbent demonstration admission and replay-corpus validation.

use super::*;

fn validate_incumbent_demonstration_artifacts(
    root: &Path,
    optimization: &OptimizationRequest,
    execution: &NativeResidualExecutionBinding,
    demonstration: &NativeIncumbentDemonstration,
) -> Result<(), NativeResidualCampaignRunnerError> {
    demonstration
        .validate(optimization, execution)
        .map_err(native_error)?;
    let batch: NativeSuffixBatch = serde_json::from_slice(
        &read_artifact(root, &demonstration.attempt.batch_request).map_err(native_error)?,
    )
    .map_err(native_error)?;
    let result_path = root.join(&demonstration.attempt.batch_result.path);
    if artifact_reference(root, &result_path).map_err(native_error)?
        != demonstration.attempt.batch_result
    {
        return Err(native_message(
            "incumbent demonstration result artifact digest differs",
        ));
    }
    let terminal = NativeTerminalBinding {
        goal: optimization.terminal_predicate.goal.clone(),
        program_sha256: optimization.terminal_predicate.program_sha256,
        definition_sha256: optimization.terminal_predicate.definition_sha256,
    };
    let validated =
        validate_native_suffix_artifacts(&batch, &result_path, &terminal).map_err(native_error)?;
    let candidate = validated
        .candidates
        .first()
        .filter(|candidate| {
            validated.candidates.len() == 1
                && candidate.id == demonstration.attempt.wire_candidate_id
        })
        .ok_or_else(|| native_message("incumbent demonstration lacks one exact candidate"))?;
    let episode_reference =
        artifact_reference(root, Path::new(&validated.episode_shard_path)).map_err(native_error)?;
    if episode_reference != demonstration.attempt.episode_shard
        || native_attempt(
            1,
            optimization.execution.deterministic_seeds[0],
            candidate,
            demonstration.attempt.batch_request.clone(),
            demonstration.attempt.batch_result.clone(),
            episode_reference,
            &validated,
        ) != demonstration.attempt
    {
        return Err(native_message(
            "incumbent demonstration differs from its validated native result",
        ));
    }
    let corpus = load_corpus(root, &demonstration.replay.artifact).map_err(native_error)?;
    demonstration
        .replay
        .validate_corpus(&corpus)
        .map_err(native_error)?;
    validate_residual_corpus_scope(optimization, &corpus).map_err(native_error)?;
    if corpus.entries.len() != 1
        || corpus.entries[0].role
            != dusklight_learning::native_replay_corpus::ReplayExperienceRole::Demonstration
        || corpus.entries[0].shard_sha256 != demonstration.attempt.episode_shard.sha256
    {
        return Err(native_message(
            "incumbent demonstration replay differs from its exact native episode",
        ));
    }
    Ok(())
}

pub(super) fn load_incumbent_demonstration(
    root: &Path,
    optimization: &OptimizationRequest,
    execution: &NativeResidualExecutionBinding,
    reference: &dusklight_harness_contracts::objective_suite::ArtifactReference,
) -> Result<NativeIncumbentDemonstration, NativeResidualCampaignRunnerError> {
    let demonstration: NativeIncumbentDemonstration =
        serde_json::from_slice(&read_artifact(root, reference).map_err(native_error)?)
            .map_err(native_error)?;
    validate_incumbent_demonstration_artifacts(root, optimization, execution, &demonstration)?;
    Ok(demonstration)
}

pub(super) fn write_uncommitted_native_request(
    batch_root: &Path,
    request_path: &Path,
    bytes: &[u8],
) -> Result<(), NativeResidualCampaignRunnerError> {
    if !request_path.exists() || fs::read(request_path).map_err(native_error)? == bytes {
        return write_exact_or_new(request_path, bytes).map_err(native_error);
    }
    let entries = fs::read_dir(batch_root)
        .map_err(native_error)?
        .map(|entry| entry.map(|entry| entry.path()).map_err(native_error))
        .collect::<Result<Vec<_>, _>>()?;
    if entries.len() != 1 || entries[0] != request_path {
        return Err(native_message(format!(
            "existing native request differs after its attempt acquired artifacts: {}",
            request_path.display()
        )));
    }

    // A request with no result, episode, or other sibling artifact has not been
    // executed or admitted. It is safe to discard after an interrupted startup
    // changed the derived wire schema; the exact replacement remains governed
    // by write_exact_or_new on every subsequent resume.
    fs::remove_file(request_path).map_err(native_error)?;
    write_exact_or_new(request_path, bytes).map_err(native_error)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn ensure_incumbent_demonstration(
    root: &Path,
    campaign: &Path,
    config: &NativeResidualCampaignRunConfig<'_>,
    parent: &InputTape,
    resume: &OptimizationResumeState,
    pool: &mut WorkerPool<'_>,
) -> Result<
    (OptimizationResumeState, NativeIncumbentDemonstration),
    NativeResidualCampaignRunnerError,
> {
    if let Some(reference) = &resume.demonstration {
        return Ok((
            resume.clone(),
            load_incumbent_demonstration(root, config.optimization, config.execution, reference)?,
        ));
    }
    ensure_not_cancelled(config)?;
    let profile = segment_profile(root, config.optimization)?;
    let batch =
        incumbent_demonstration_batch(config.optimization, config.execution, profile, parent)?;
    let batch_root = campaign.join("demonstration").join("native");
    fs::create_dir_all(&batch_root).map_err(native_error)?;
    let request_path = batch_root.join("request.json");
    write_uncommitted_native_request(
        &batch_root,
        &request_path,
        &pretty_json(&batch).map_err(native_error)?,
    )?;
    let (result_path, adopted) = select_result_path(&batch_root, &batch, &pool.terminal)?;
    let output = if let Some(validated) = adopted {
        BatchOutput {
            lane: 0,
            request_path,
            result_path,
            validated,
        }
    } else {
        ensure_not_cancelled(config)?;
        pool.run_jobs(vec![BatchJob {
            lane: 0,
            request_path,
            result_path,
            batch,
        }])?
        .pop()
        .ok_or_else(|| native_message("incumbent demonstration produced no native result"))?
    };
    let candidate = output
        .validated
        .candidates
        .first()
        .filter(|candidate| {
            output.validated.candidates.len() == 1 && candidate.id == "incumbent-demonstration"
        })
        .ok_or_else(|| native_message("incumbent demonstration lacks one exact result"))?;
    let incumbent = config
        .optimization
        .incumbent
        .as_ref()
        .ok_or_else(|| native_message("native residual campaign requires an incumbent"))?;
    if candidate.first_hit_tick != Some(incumbent.first_hit_tick)
        || incumbent.first_hit_tick.checked_add(1) != Some(candidate.simulated_ticks)
    {
        return Err(native_message(
            "incumbent demonstration did not reproduce its exact terminal proof",
        ));
    }
    let request = artifact_reference(root, &output.request_path).map_err(native_error)?;
    let result = artifact_reference(root, &output.result_path).map_err(native_error)?;
    let episode = artifact_reference(root, Path::new(&output.validated.episode_shard_path))
        .map_err(native_error)?;
    let shard = NativeEpisodeShard::read(root.join(&episode.path)).map_err(native_error)?;
    if shard.content_sha256 != episode.sha256 {
        return Err(native_message(
            "incumbent demonstration shard differs from its artifact identity",
        ));
    }
    let replay = append_incumbent_demonstration_replay(
        root,
        campaign,
        config.optimization,
        &shard,
        &candidate.id,
    )
    .map_err(native_error)?;
    let attempt = native_attempt(
        1,
        pool.lanes[0].seed,
        candidate,
        request,
        result,
        episode,
        &output.validated,
    );
    let demonstration =
        NativeIncumbentDemonstration::seal(config.optimization, config.execution, attempt, replay)
            .map_err(native_error)?;
    let path = campaign.join("demonstration").join("manifest.json");
    write_exact_or_new(
        &path,
        &demonstration.to_pretty_json().map_err(native_error)?,
    )
    .map_err(native_error)?;
    let reference = artifact_reference(root, &path).map_err(native_error)?;
    let resume = append_optimization_resume_events_from_validated_state(
        config.optimization,
        root,
        resume,
        vec![OptimizationResumeEvent::DemonstrationSeeded {
            demonstration: reference,
            simulated_ticks: demonstration.attempt.simulated_ticks,
        }],
    )
    .map_err(native_error)?;
    Ok((resume, demonstration))
}

pub(super) fn incumbent_demonstration_batch(
    optimization: &OptimizationRequest,
    execution: &NativeResidualExecutionBinding,
    profile: dusklight_search::search::SegmentProfile,
    parent: &InputTape,
) -> Result<NativeSuffixBatch, NativeResidualCampaignRunnerError> {
    let mut horizon_tape = parent.clone();
    extend_tape_with_released_input(
        &mut horizon_tape,
        optimization.budgets.exploration_horizon_ticks,
    )
    .map_err(native_error)?;
    let imported = Candidate::from_absolute_tape(profile, &horizon_tape).map_err(native_error)?;
    Ok(NativeSuffixBatch {
        schema: NATIVE_SUFFIX_BATCH_SCHEMA.into(),
        source_frame: usize::try_from(optimization.route.source_boundary_index)
            .map_err(native_error)?,
        source_boundary_fingerprint: optimization
            .route
            .native_source_boundary_fingerprint
            .clone(),
        checkpoint_validation: NativeCheckpointValidation {
            kind: "recorded_replay_window".into(),
            ticks: usize::try_from(execution.checkpoint_validation_ticks).map_err(native_error)?,
        },
        maximum_ticks: usize::try_from(optimization.budgets.exploration_horizon_ticks)
            .map_err(native_error)?,
        verify_state_hashes: execution.verify_state_hashes,
        checkpoint_cache: None,
        candidates: vec![NativeSuffixCandidate {
            id: "incumbent-demonstration".into(),
            actions: project_native_port_one_actions(imported.actions)?,
            controller_program_hex: None,
            maximum_ticks: None,
            cancellation_guard: None,
        }],
    })
}

pub(super) fn validate_checkpoint_replay(
    root: &Path,
    optimization: &OptimizationRequest,
    execution: &NativeResidualExecutionBinding,
    parent: &InputTape,
    parent_bytes: &[u8],
    resume: &OptimizationResumeState,
    checkpoint: &ResidualCampaignCheckpoint,
) -> Result<(), NativeResidualCampaignRunnerError> {
    let Some(replay) = &checkpoint.replay_corpus else {
        // Checkpoints written before automatic ingestion remain migratable. The
        // next completed generation backfills all authenticated evaluations.
        if resume.demonstration.is_some() {
            return Err(native_message(
                "seeded optimization checkpoint omits its incumbent demonstration replay",
            ));
        }
        return Ok(());
    };
    let corpus = load_corpus(root, &replay.artifact).map_err(native_error)?;
    replay.validate_corpus(&corpus).map_err(native_error)?;
    validate_residual_corpus_scope(optimization, &corpus).map_err(native_error)?;
    let expected_randomized = checkpoint
        .completed_candidates
        .checked_mul(u64::from(optimization.execution.repetitions))
        .ok_or_else(|| native_message("residual replay checkpoint entry count overflowed"))?;
    let demonstration_entries = corpus
        .entries
        .iter()
        .filter(|entry| {
            entry.role
                == dusklight_learning::native_replay_corpus::ReplayExperienceRole::Demonstration
        })
        .count() as u64;
    let randomized_entries = corpus
        .entries
        .iter()
        .filter(|entry| {
            entry.role
                == dusklight_learning::native_replay_corpus::ReplayExperienceRole::RandomizedCoverage
        })
        .count() as u64;
    let alternate_entries = corpus
        .entries
        .iter()
        .filter(|entry| {
            entry.role
                == dusklight_learning::native_replay_corpus::ReplayExperienceRole::AlternateTerminal
        })
        .map(|entry| {
            (
                entry.shard_sha256,
                entry.episode_id.clone(),
                entry.objective.clone(),
            )
        })
        .collect::<BTreeSet<_>>();
    let mut authenticated_alternates = BTreeSet::new();
    let mut artifact_cache = NativeAttemptArtifactCache::default();
    for row in resume.candidates.iter().filter(|row| row.result.is_some()) {
        let candidate =
            load_candidate(root, optimization, parent, parent_bytes, row).map_err(native_error)?;
        let evaluation = load_native_evaluation(
            root,
            optimization,
            execution,
            row,
            &candidate,
            &mut artifact_cache,
        )?;
        for alternate in &evaluation.alternate_terminals {
            for attempt in alternate
                .attempts
                .iter()
                .filter(|attempt| attempt.first_hit_tick.is_some())
            {
                authenticated_alternates.insert((
                    attempt.episode_shard.sha256,
                    attempt.wire_candidate_id.clone(),
                    alternate.terminal.goal.clone(),
                ));
            }
        }
    }
    if randomized_entries != expected_randomized
        || demonstration_entries != u64::from(resume.demonstration.is_some())
        || !alternate_entries.is_subset(&authenticated_alternates)
        || (checkpoint.completed_candidates == resume.completed_candidates
            && alternate_entries != authenticated_alternates)
    {
        return Err(native_message(
            "residual replay corpus does not cover every checkpointed native attempt",
        ));
    }
    Ok(())
}
