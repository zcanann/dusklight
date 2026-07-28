//! Load and validate residual-attempt evidence and result artifacts.

use super::*;

pub(super) fn load_native_evaluation(
    root: &Path,
    optimization: &OptimizationRequest,
    execution: &NativeResidualExecutionBinding,
    row: &crate::optimization_resume::OptimizationResumeCandidate,
    candidate: &PreparedCandidate,
    artifact_cache: &mut NativeAttemptArtifactCache,
) -> Result<NativeResidualCampaignEvaluation, NativeResidualCampaignRunnerError> {
    let reference = row
        .result
        .as_ref()
        .ok_or_else(|| native_message("native residual evaluation is not journaled"))?;
    let evaluation: NativeResidualCampaignEvaluation =
        serde_json::from_slice(&read_artifact(root, reference).map_err(native_error)?)
            .map_err(native_error)?;
    evaluation
        .validate(optimization, execution, &candidate.envelope)
        .map_err(native_error)?;
    validate_evaluation_artifacts_cached(root, optimization, &evaluation, artifact_cache)?;
    Ok(evaluation)
}

pub(super) fn validate_evaluation_artifacts(
    root: &Path,
    optimization: &OptimizationRequest,
    evaluation: &NativeResidualCampaignEvaluation,
) -> Result<(), NativeResidualCampaignRunnerError> {
    validate_evaluation_artifacts_cached(
        root,
        optimization,
        evaluation,
        &mut NativeAttemptArtifactCache::default(),
    )
}

pub(super) fn validate_evaluation_artifacts_cached(
    root: &Path,
    optimization: &OptimizationRequest,
    evaluation: &NativeResidualCampaignEvaluation,
    artifact_cache: &mut NativeAttemptArtifactCache,
) -> Result<(), NativeResidualCampaignRunnerError> {
    let terminal = NativeTerminalBinding {
        goal: optimization.terminal_predicate.goal.clone(),
        program_sha256: optimization.terminal_predicate.program_sha256,
        definition_sha256: optimization.terminal_predicate.definition_sha256,
    };
    for attempt in &evaluation.attempts {
        validate_attempt_artifacts_cached(root, &terminal, attempt, artifact_cache)?;
    }
    if artifact_cache.alternate_terminals.is_none() {
        artifact_cache.alternate_terminals = Some(
            optimization
                .alternate_terminal_predicates_after_request_validation(root)
                .map_err(native_error)?
                .into_iter()
                .map(|binding| NativeTerminalBinding {
                    goal: binding.goal,
                    program_sha256: binding.program_sha256,
                    definition_sha256: binding.definition_sha256,
                })
                .collect(),
        );
    }
    let expected = if evaluation
        .attempts
        .first()
        .is_some_and(|attempt| attempt.first_hit_tick.is_none())
    {
        artifact_cache
            .alternate_terminals
            .as_deref()
            .unwrap_or_default()
    } else {
        &[]
    };
    if evaluation
        .alternate_terminals
        .iter()
        .map(|alternate| &alternate.terminal)
        .ne(expected.iter())
    {
        return Err(native_message(
            "native residual alternate terminals differ from the sealed optimization request",
        ));
    }
    for alternate in &evaluation.alternate_terminals {
        for attempt in &alternate.attempts {
            validate_attempt_artifacts_cached(root, &alternate.terminal, attempt, artifact_cache)?;
        }
    }
    Ok(())
}

pub(crate) fn validate_attempt_artifacts(
    root: &Path,
    terminal: &NativeTerminalBinding,
    attempt: &NativeResidualAttempt,
) -> Result<(), NativeResidualCampaignRunnerError> {
    validate_attempt_artifacts_cached(
        root,
        terminal,
        attempt,
        &mut NativeAttemptArtifactCache::default(),
    )
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct NativeAttemptArtifactCacheKey {
    batch_request_path: String,
    batch_request_sha256: Digest,
    batch_result_path: String,
    batch_result_sha256: Digest,
    terminal_goal: String,
    terminal_program_sha256: Digest,
    terminal_definition_sha256: Digest,
}

#[derive(Clone, Debug)]
struct CachedNativeAttemptArtifacts {
    validated: ValidatedNativeSuffixBatch,
    episode: ArtifactReference,
}

#[derive(Default)]
pub(super) struct NativeAttemptArtifactCache {
    attempts: BTreeMap<NativeAttemptArtifactCacheKey, CachedNativeAttemptArtifacts>,
    alternate_terminals: Option<Vec<NativeTerminalBinding>>,
}

pub(super) fn validate_attempt_artifacts_cached(
    root: &Path,
    terminal: &NativeTerminalBinding,
    attempt: &NativeResidualAttempt,
    artifact_cache: &mut NativeAttemptArtifactCache,
) -> Result<(), NativeResidualCampaignRunnerError> {
    let cache_key = NativeAttemptArtifactCacheKey {
        batch_request_path: attempt.batch_request.path.clone(),
        batch_request_sha256: attempt.batch_request.sha256,
        batch_result_path: attempt.batch_result.path.clone(),
        batch_result_sha256: attempt.batch_result.sha256,
        terminal_goal: terminal.goal.clone(),
        terminal_program_sha256: terminal.program_sha256,
        terminal_definition_sha256: terminal.definition_sha256,
    };
    if !artifact_cache.attempts.contains_key(&cache_key) {
        let batch: NativeSuffixBatch = serde_json::from_slice(
            &read_artifact(root, &attempt.batch_request).map_err(native_error)?,
        )
        .map_err(native_error)?;
        let result_path = root.join(&attempt.batch_result.path);
        if artifact_reference(root, &result_path).map_err(native_error)? != attempt.batch_result {
            return Err(native_message(
                "native residual batch result artifact digest differs",
            ));
        }
        let validated = validate_native_suffix_artifacts(&batch, &result_path, terminal)
            .map_err(native_error)?;
        let episode = artifact_reference(root, Path::new(&validated.episode_shard_path))
            .map_err(native_error)?;
        artifact_cache.attempts.insert(
            cache_key.clone(),
            CachedNativeAttemptArtifacts { validated, episode },
        );
    }
    let cached = artifact_cache
        .attempts
        .get(&cache_key)
        .expect("validated native attempt artifacts were cached");
    let candidate = cached
        .validated
        .candidates
        .iter()
        .find(|candidate| candidate.id == attempt.wire_candidate_id)
        .ok_or_else(|| native_message("native residual attempt is absent from its batch"))?;
    if cached.episode != attempt.episode_shard
        || cached.validated.restore_identity != attempt.restore_identity
        || cached.validated.checkpoint_bytes != attempt.checkpoint_bytes
        || candidate.simulated_ticks != attempt.simulated_ticks
        || candidate.first_hit_tick != attempt.first_hit_tick
        || candidate.terminal_boundary_fingerprint != attempt.terminal_boundary_fingerprint
        || candidate.behavior_sha256 != attempt.behavior_sha256
    {
        return Err(native_message(
            "native residual attempt differs from its validated batch artifacts",
        ));
    }
    Ok(())
}

pub(crate) fn validate_exact_replay_attempt_artifacts(
    root: &Path,
    optimization: &OptimizationRequest,
    execution: &NativeResidualExecutionBinding,
    terminal: &NativeTerminalBinding,
    expected_tape: &InputTape,
    attempt: &NativeResidualAttempt,
) -> Result<(), NativeResidualCampaignRunnerError> {
    validate_attempt_artifacts(root, terminal, attempt)?;
    let batch: NativeSuffixBatch =
        serde_json::from_slice(&read_artifact(root, &attempt.batch_request).map_err(native_error)?)
            .map_err(native_error)?;
    let expected =
        Candidate::from_absolute_tape(segment_profile(root, optimization)?, expected_tape)
            .map_err(native_error)?;
    let actual = batch
        .candidates
        .iter()
        .find(|candidate| candidate.id == attempt.wire_candidate_id)
        .ok_or_else(|| native_message("exact replay candidate is absent from its batch"))?;
    if batch.source_frame
        != usize::try_from(optimization.route.source_boundary_index).map_err(native_error)?
        || batch.source_boundary_fingerprint
            != optimization.route.native_source_boundary_fingerprint
        || batch.checkpoint_validation.kind != "recorded_replay_window"
        || batch.checkpoint_validation.ticks
            != usize::try_from(execution.checkpoint_validation_ticks).map_err(native_error)?
        || batch.maximum_ticks
            != usize::try_from(optimization.budgets.exploration_horizon_ticks)
                .map_err(native_error)?
        || batch.verify_state_hashes != execution.verify_state_hashes
        || actual.actions != expected.actions
    {
        return Err(native_message(
            "exact replay attempt differs from its expected residual tape or execution boundary",
        ));
    }
    Ok(())
}

pub(super) fn replay_completed(
    config: &NativeResidualCampaignRunConfig<'_>,
    root: &Path,
    parent: &InputTape,
    parent_bytes: &[u8],
    resume: &OptimizationResumeState,
    archive: &mut ResidualOutcomeArchive,
) -> Result<(), NativeResidualCampaignRunnerError> {
    let mut artifact_cache = NativeAttemptArtifactCache::default();
    for row in resume.candidates.iter().filter(|row| row.result.is_some()) {
        ensure_not_cancelled(config)?;
        let candidate = load_candidate(root, config.optimization, parent, parent_bytes, row)
            .map_err(native_error)?;
        let evaluation = load_native_evaluation(
            root,
            config.optimization,
            config.execution,
            row,
            &candidate,
            &mut artifact_cache,
        )?;
        archive
            .record(&candidate.compiled, evaluation.evidence)
            .map_err(native_error)?;
    }
    Ok(())
}

pub(super) fn existing_evaluation(
    root: &Path,
    path: &Path,
    optimization: &OptimizationRequest,
    execution: &NativeResidualExecutionBinding,
    candidate: &PreparedCandidate,
) -> Result<Option<NativeResidualCampaignEvaluation>, NativeResidualCampaignRunnerError> {
    if !path.exists() {
        return Ok(None);
    }
    let evaluation: NativeResidualCampaignEvaluation =
        serde_json::from_slice(&fs::read(path).map_err(native_error)?).map_err(native_error)?;
    evaluation
        .validate(optimization, execution, &candidate.envelope)
        .map_err(native_error)?;
    validate_evaluation_artifacts(root, optimization, &evaluation)?;
    Ok(Some(evaluation))
}

pub(super) fn batch_group_id(batch: &NativeSuffixBatch) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight.native-residual-batch-group/v1\0");
    for candidate in &batch.candidates {
        hasher.update((candidate.id.len() as u64).to_le_bytes());
        hasher.update(candidate.id.as_bytes());
    }
    let digest: [u8; 32] = hasher.finalize().into();
    digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(super) fn select_result_path(
    batch_root: &Path,
    batch: &NativeSuffixBatch,
    terminal: &NativeTerminalBinding,
) -> Result<(PathBuf, Option<ValidatedNativeSuffixBatch>), NativeResidualCampaignRunnerError> {
    for trial in 1..=100_u32 {
        let result = batch_root.join(format!("result-try{trial:03}.json"));
        if result.is_file() {
            match validate_native_suffix_artifacts(batch, &result, terminal) {
                Ok(validated) => return Ok((result, Some(validated))),
                Err(_) => continue,
            }
        }
        let mut episode = result.as_os_str().to_os_string();
        episode.push(".episodes.dseps");
        if !result.exists() && !Path::new(&episode).exists() {
            return Ok((result, None));
        }
    }
    Err(native_message(
        "native residual batch exhausted crash-recovery result paths",
    ))
}
