use super::*;

pub(super) fn prepare_launch(
    config: &NativeSuffixWorkerLaunch,
    identities: Option<NativeSuffixPrevalidatedFileIdentities>,
    comparators: NativeHeadlessAuditComparators,
) -> Result<PreparedLaunch, NativeSuffixWorkerError> {
    if (comparators.presentation_lifecycle && !comparators.gpu_frame_submission)
        || (comparators.imgui_frame_lifecycle && !comparators.presentation_lifecycle)
        || (comparators.cpu_renderer_submission && !comparators.imgui_frame_lifecycle)
    {
        return Err(worker_message(
            "headless audit comparators must retain the GPU -> presentation -> ImGui -> CPU renderer dependency chain",
        ));
    }
    if config.world_context_sha256 == Digest::ZERO
        || config.card_fixture_sha256 == Digest::ZERO
        || config.terminal.program_sha256 == Digest::ZERO
        || config.terminal.definition_sha256 == Digest::ZERO
        || config.terminal.goal.is_empty()
    {
        return Err(worker_message(
            "native suffix launch identities are incomplete",
        ));
    }
    let executable = canonical_file(&config.executable, "executable")?;
    let game_data = canonical_file(&config.game_data, "game data")?;
    let input_tape = canonical_file(&config.input_tape, "input tape")?;
    let milestone_program = canonical_file(&config.milestone_program, "milestone program")?;
    let card_fixture = canonical_directory(&config.card_fixture, "card fixture")?;
    let working_directory = canonical_directory(&config.working_directory, "working directory")?;
    let batch_path = canonical_file(&config.initial_batch, "initial suffix batch")?;
    let batch_bytes = fs::read(&batch_path).map_err(|source| {
        worker_message(format!(
            "cannot read initial suffix batch {}: {source}",
            batch_path.display()
        ))
    })?;
    let batch: NativeSuffixBatch = serde_json::from_slice(&batch_bytes).map_err(worker_error)?;
    validate_batch_shape(&batch)?;

    let tape_bytes = fs::read(&input_tape).map_err(|source| {
        worker_message(format!(
            "cannot read native suffix input tape {}: {source}",
            input_tape.display()
        ))
    })?;
    let tape = InputTape::decode(&tape_bytes).map_err(worker_error)?.tape;
    if tape.boot != TapeBoot::Process
        || batch
            .source_frame
            .checked_add(batch.maximum_ticks)
            .is_none_or(|end| end > tape.frames.len())
        || batch
            .source_frame
            .checked_add(batch.checkpoint_validation.ticks)
            .is_none_or(|end| end > tape.frames.len())
    {
        return Err(worker_message(
            "native suffix source and horizon do not fit the absolute process-boot tape",
        ));
    }

    let program_bytes = fs::read(&milestone_program).map_err(|source| {
        worker_message(format!(
            "cannot read native suffix milestone program {}: {source}",
            milestone_program.display()
        ))
    })?;
    let decoded =
        dusklight_objectives::milestone_dsl::decode(&program_bytes).map_err(worker_error)?;
    let definition_index = decoded
        .program
        .definitions
        .iter()
        .position(|definition| definition.name == config.terminal.goal)
        .ok_or_else(|| worker_message("milestone program does not define the terminal goal"))?;
    if Digest(decoded.program_sha256) != config.terminal.program_sha256
        || Digest(decoded.definitions[definition_index].sha256) != config.terminal.definition_sha256
    {
        return Err(worker_message(
            "milestone program identities differ from the terminal binding",
        ));
    }

    let result = prepare_new_result_output(&config.initial_result, "initial suffix result")?;
    let winner_tape = config
        .initial_winner_tape
        .as_deref()
        .map(|path| prepare_new_output(path, "initial suffix winner tape"))
        .transpose()?;
    prepare_state_root(&config.state_root)?;
    let state_root = config.state_root.canonicalize().map_err(|source| {
        worker_message(format!(
            "cannot canonicalize native suffix state root {}: {source}",
            config.state_root.display()
        ))
    })?;
    let renderer_cache = state_root
        .parent()
        .unwrap_or(&state_root)
        .join("renderer-cache");
    fs::create_dir_all(&renderer_cache).map_err(|source| {
        worker_message(format!(
            "cannot create native suffix renderer cache {}: {source}",
            renderer_cache.display()
        ))
    })?;

    let (executable_sha256, game_data_sha256) = match identities {
        Some(identities)
            if identities.executable_sha256 != Digest::ZERO
                && identities.game_data_sha256 != Digest::ZERO =>
        {
            (identities.executable_sha256, identities.game_data_sha256)
        }
        Some(_) => {
            return Err(worker_message(
                "prevalidated native suffix file identities are incomplete",
            ));
        }
        None => (sha256_file(&executable)?, sha256_file(&game_data)?),
    };
    let identity = NativeSuffixWorkerIdentity {
        executable_sha256,
        game_data_sha256,
        input_tape_sha256: sha256(&tape_bytes),
        milestone_program_sha256: sha256(&program_bytes),
        card_fixture_sha256: config.card_fixture_sha256,
        world_context_sha256: config.world_context_sha256,
        source_frame: batch.source_frame as u64,
        source_boundary_fingerprint: batch.source_boundary_fingerprint.clone(),
        checkpoint_validation_kind: batch.checkpoint_validation.kind.clone(),
        checkpoint_validation_ticks: batch.checkpoint_validation.ticks as u64,
        maximum_ticks: batch.maximum_ticks as u64,
        terminal: config.terminal.clone(),
    };
    let mut args = vec![
        "--automation-engine-worker".into(),
        "--headless".into(),
        "--dvd".into(),
        path_text(&game_data, "game data")?.into(),
        "--input-tape".into(),
        path_text(&input_tape, "input tape")?.into(),
        "--input-tape-end".into(),
        "release".into(),
        "--automation-data-root".into(),
        path_text(&state_root, "state root")?.into(),
        "--automation-card-fixture".into(),
        path_text(&card_fixture, "card fixture")?.into(),
        "--renderer-cache-root".into(),
        path_text(&renderer_cache, "renderer cache")?.into(),
        "--suffix-batch".into(),
        path_text(&batch_path, "initial suffix batch")?.into(),
        "--suffix-batch-result".into(),
        path_text(&result, "initial suffix result")?.into(),
        "--automation-game-data-sha256".into(),
        game_data_sha256.to_string(),
        "--automation-world-context-sha256".into(),
        config.world_context_sha256.to_string(),
        "--milestone-program".into(),
        path_text(&milestone_program, "milestone program")?.into(),
        "--milestones".into(),
        config.terminal.goal.clone(),
        "--milestone-goal".into(),
        config.terminal.goal.clone(),
        "--milestone-result".into(),
        path_text(&state_root.join("milestones.json"), "milestone result")?.into(),
    ];
    if let Some(winner_tape) = &winner_tape {
        args.push("--suffix-batch-winner-tape".into());
        args.push(path_text(winner_tape, "initial suffix winner tape")?.into());
    }
    for cvar in FIXED_AUTOMATION_CVARS {
        args.push("--cvar".into());
        args.push((*cvar).into());
    }
    for (enabled, argument) in [
        (
            comparators.gpu_frame_submission,
            "--headless-submit-gpu-frames",
        ),
        (
            comparators.cpu_renderer_submission,
            "--headless-retain-cpu-renderer-submission",
        ),
        (
            comparators.presentation_lifecycle,
            "--headless-retain-presentation-lifecycle",
        ),
        (
            comparators.imgui_frame_lifecycle,
            "--headless-retain-imgui-frame-lifecycle",
        ),
        (comparators.host_pacing, "--headless-retain-host-pacing"),
        (
            comparators.host_audio_device,
            "--headless-retain-host-audio-device",
        ),
    ] {
        if enabled {
            args.push(argument.into());
        }
    }
    Ok(PreparedLaunch {
        executable,
        working_directory,
        args,
        batch,
        result,
        identity,
        terminal: config.terminal.clone(),
    })
}

pub(super) fn prepare_frozen_launch(
    config: &NativeFrozenPolicyWorkerLaunch,
) -> Result<PreparedFrozenLaunch, NativeSuffixWorkerError> {
    if config.world_context_sha256 == Digest::ZERO
        || config.card_fixture_sha256 == Digest::ZERO
        || config.terminal.program_sha256 == Digest::ZERO
        || config.terminal.definition_sha256 == Digest::ZERO
        || config.terminal.goal.is_empty()
    {
        return Err(worker_message(
            "native frozen-policy launch identities are incomplete",
        ));
    }
    let executable = canonical_file(&config.executable, "executable")?;
    let game_data = canonical_file(&config.game_data, "game data")?;
    let input_tape = canonical_file(&config.input_tape, "input tape")?;
    let milestone_program = canonical_file(&config.milestone_program, "milestone program")?;
    let card_fixture = canonical_directory(&config.card_fixture, "card fixture")?;
    let working_directory = canonical_directory(&config.working_directory, "working directory")?;
    let batch_path = canonical_file(&config.initial_batch, "initial frozen policy batch")?;
    let batch: NativeFrozenPolicySuffixBatch =
        serde_json::from_slice(&fs::read(&batch_path).map_err(worker_error)?)
            .map_err(worker_error)?;
    let model_path = canonical_frozen_model(&batch)?;
    let model_bytes = fs::read(&model_path).map_err(worker_error)?;
    batch.validate(&model_bytes).map_err(worker_error)?;
    let model = FrozenInferenceModel::from_bytes(&model_bytes).map_err(worker_error)?;
    if model.objective_sha256 != config.terminal.definition_sha256 {
        return Err(worker_message(
            "native frozen policy objective differs from the terminal definition",
        ));
    }

    let tape_bytes = fs::read(&input_tape).map_err(worker_error)?;
    let tape = InputTape::decode(&tape_bytes).map_err(worker_error)?.tape;
    if tape.boot != TapeBoot::Process
        || batch
            .source_frame
            .checked_add(batch.maximum_ticks)
            .is_none_or(|end| end > tape.frames.len())
        || batch
            .source_frame
            .checked_add(batch.checkpoint_validation.ticks)
            .is_none_or(|end| end > tape.frames.len())
    {
        return Err(worker_message(
            "native frozen policy source and horizon do not fit the process-boot tape",
        ));
    }

    let program_bytes = fs::read(&milestone_program).map_err(worker_error)?;
    let decoded =
        dusklight_objectives::milestone_dsl::decode(&program_bytes).map_err(worker_error)?;
    let definition_index = decoded
        .program
        .definitions
        .iter()
        .position(|definition| definition.name == config.terminal.goal)
        .ok_or_else(|| worker_message("milestone program does not define the terminal goal"))?;
    if Digest(decoded.program_sha256) != config.terminal.program_sha256
        || Digest(decoded.definitions[definition_index].sha256) != config.terminal.definition_sha256
    {
        return Err(worker_message(
            "milestone program identities differ from the frozen policy terminal binding",
        ));
    }

    let result = prepare_new_result_output(&config.initial_result, "initial frozen policy result")?;
    prepare_state_root(&config.state_root)?;
    let state_root = config.state_root.canonicalize().map_err(worker_error)?;
    let renderer_cache = state_root
        .parent()
        .unwrap_or(&state_root)
        .join("renderer-cache");
    fs::create_dir_all(&renderer_cache).map_err(worker_error)?;

    let game_data_sha256 = sha256_file(&game_data)?;
    let identity = NativeSuffixWorkerIdentity {
        executable_sha256: sha256_file(&executable)?,
        game_data_sha256,
        input_tape_sha256: sha256(&tape_bytes),
        milestone_program_sha256: sha256(&program_bytes),
        card_fixture_sha256: config.card_fixture_sha256,
        world_context_sha256: config.world_context_sha256,
        source_frame: batch.source_frame as u64,
        source_boundary_fingerprint: batch.source_boundary_fingerprint.clone(),
        checkpoint_validation_kind: batch.checkpoint_validation.kind.clone(),
        checkpoint_validation_ticks: batch.checkpoint_validation.ticks as u64,
        maximum_ticks: batch.maximum_ticks as u64,
        terminal: config.terminal.clone(),
    };
    validate_frozen_batch_identity(&batch, &model_bytes, &identity, &config.terminal)?;
    let mut args = vec![
        "--automation-engine-worker".into(),
        "--headless".into(),
        "--dvd".into(),
        path_text(&game_data, "game data")?.into(),
        "--input-tape".into(),
        path_text(&input_tape, "input tape")?.into(),
        "--input-tape-end".into(),
        "release".into(),
        "--automation-data-root".into(),
        path_text(&state_root, "state root")?.into(),
        "--automation-card-fixture".into(),
        path_text(&card_fixture, "card fixture")?.into(),
        "--renderer-cache-root".into(),
        path_text(&renderer_cache, "renderer cache")?.into(),
        "--suffix-batch".into(),
        path_text(&batch_path, "initial frozen policy batch")?.into(),
        "--suffix-batch-result".into(),
        path_text(&result, "initial frozen policy result")?.into(),
        "--automation-game-data-sha256".into(),
        game_data_sha256.to_string(),
        "--automation-world-context-sha256".into(),
        config.world_context_sha256.to_string(),
        "--milestone-program".into(),
        path_text(&milestone_program, "milestone program")?.into(),
        "--milestones".into(),
        config.terminal.goal.clone(),
        "--milestone-goal".into(),
        config.terminal.goal.clone(),
        "--milestone-result".into(),
        path_text(&state_root.join("milestones.json"), "milestone result")?.into(),
    ];
    for cvar in FIXED_AUTOMATION_CVARS {
        args.push("--cvar".into());
        args.push((*cvar).into());
    }
    Ok(PreparedFrozenLaunch {
        executable,
        working_directory,
        args,
        batch,
        model_bytes,
        result,
        identity,
        terminal: config.terminal.clone(),
    })
}

pub(super) fn validate_completed_batch(
    complete: &BatchComplete,
    expected_result: &Path,
    batch: &NativeSuffixBatch,
    terminal: &NativeTerminalBinding,
) -> Result<ValidatedNativeSuffixBatch, NativeSuffixWorkerError> {
    let result_path = canonical_file(expected_result, "native suffix result")?;
    if Path::new(&complete.result) != result_path {
        return Err(worker_message(
            "engine worker returned a different suffix result path",
        ));
    }
    let validated = validate_native_suffix_artifacts(batch, &result_path, terminal)?;
    let episode_path = canonical_file(Path::new(&complete.episode_shard), "native episode shard")?;
    if Path::new(&complete.episode_shard) != episode_path
        || Path::new(&validated.episode_shard_path) != episode_path
    {
        return Err(worker_message(
            "engine worker response, suffix result, and episode shard paths differ",
        ));
    }
    Ok(validated)
}

pub(super) fn validate_completed_frozen_batch(
    complete: &BatchComplete,
    expected_result: &Path,
    batch: &NativeFrozenPolicySuffixBatch,
    model_bytes: &[u8],
    terminal: &NativeTerminalBinding,
) -> Result<ValidatedNativeFrozenPolicyBatch, NativeSuffixWorkerError> {
    let result_path = canonical_file(expected_result, "native frozen policy result")?;
    if Path::new(&complete.result) != result_path {
        return Err(worker_message(
            "engine worker returned a different frozen policy result path",
        ));
    }
    let validated =
        validate_native_frozen_policy_artifacts(batch, model_bytes, &result_path, terminal)?;
    let episode_path = canonical_file(
        Path::new(&complete.episode_shard),
        "native frozen policy episode shard",
    )?;
    if Path::new(&complete.episode_shard) != episode_path
        || Path::new(&validated.execution.episode_shard_path) != episode_path
    {
        return Err(worker_message(
            "engine worker response, frozen policy result, and episode shard paths differ",
        ));
    }
    Ok(validated)
}
