use super::*;

pub(crate) fn append_origin_card_fixture_arg(
    timeline: &Timeline,
    repository_root: &Path,
    boot: &TapeBoot,
    command: &mut Command,
) -> Result<(), WorkbenchError> {
    if !matches!(boot, TapeBoot::Process) {
        return Ok(());
    }
    let Some(relative) = timeline
        .origin
        .as_ref()
        .and_then(|origin| origin.card_fixture.as_deref())
    else {
        return Ok(());
    };
    command
        .arg("--automation-card-fixture")
        .arg(validated_card_fixture_root(relative, repository_root)?);
    Ok(())
}

pub(crate) fn append_native_oracle_args(
    command: &mut Command,
    state_root: &Path,
    oracle: NativePlaybackOracle,
) {
    if oracle == NativePlaybackOracle::EyeShredder {
        command
            .arg("--automation-oracle")
            .arg("eye-shredder")
            .arg("--automation-oracle-continue-on-pass")
            .arg("--automation-oracle-result")
            .arg(state_root.join("eye-shredder.oracle.json"))
            .arg("--name-entry-trace")
            .arg(state_root.join("eye-shredder.name-entry.trace.json"));
    }
}

pub(crate) fn verify_native_fidelity(
    game: &Path,
    working_directory: &Path,
) -> Result<(), WorkbenchError> {
    let output = Command::new(game)
        .current_dir(working_directory)
        .arg("--automation-hello")
        .output()
        .map_err(|error| {
            WorkbenchError::new(format!(
                "cannot inspect native console-fidelity support in {}: {error}",
                game.display()
            ))
        })?;
    if !output.status.success() {
        return Err(WorkbenchError::new(format!(
            "native console-fidelity preflight failed for {} (exit {})",
            game.display(),
            output.status
        )));
    }
    let hello: serde_json::Value = serde_json::from_slice(&output.stdout).map_err(|error| {
        WorkbenchError::new(format!(
            "native console-fidelity preflight returned invalid automation identity: {error}"
        ))
    })?;
    validate_native_fidelity_identity(&hello)
}

pub(crate) fn validate_native_fidelity_identity(
    hello: &serde_json::Value,
) -> Result<(), WorkbenchError> {
    let feature_switches = hello
        .pointer("/build/feature_switches")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let fidelity_profile = hello
        .pointer("/build/fidelity_profile")
        .and_then(serde_json::Value::as_str);
    let supported = hello.get("ok").and_then(serde_json::Value::as_bool) == Some(true)
        && fidelity_profile == Some("cursor_breakout_shadow")
        && feature_switches.contains("automation_observers=ON")
        && feature_switches.contains("automation_fidelity_models=ON");
    if supported {
        return Ok(());
    }
    Err(WorkbenchError::new(format!(
        "the workbench requires the console-correct cursor-breakout fidelity model; this executable reports profile {:?} and feature switches {:?}",
        fidelity_profile, feature_switches
    )))
}

pub(crate) fn append_fixed_step_pacing(command: &mut Command, speed_percent: u16) {
    command
        .arg("--fixed-step")
        .arg("--fixed-step-speed-percent")
        .arg(speed_percent.to_string());
}

pub(crate) fn validate_draft_label(label: &str) -> Result<String, WorkbenchError> {
    let label = label.trim();
    if label.is_empty() || label.len() > 160 || label.chars().any(char::is_control) {
        return Err(WorkbenchError::new(
            "draft label must be 1 to 160 UTF-8 bytes without controls",
        ));
    }
    Ok(label.to_owned())
}

pub(crate) fn append_accelerated_recording_prefix(
    command: &mut Command,
    playback: &Path,
    parent_frames: usize,
    countdown_seconds: u8,
) {
    command
        .arg("--input-tape")
        .arg(playback)
        .arg("--input-tape-end")
        .arg("release")
        .arg("--input-tape-fast-forward-frames")
        .arg(parent_frames.to_string())
        .arg("--record-input-countdown-seconds")
        .arg(countdown_seconds.to_string());
}

pub(crate) fn record_continuation(
    timeline: &Timeline,
    config: &WorkbenchConfig,
    request: BrowserRecordRequest,
) -> Result<RecordResponse, WorkbenchError> {
    let artifact_root = configured_artifact_root(config)?;
    let game = canonical_file(&config.game, "game executable")?;
    let dvd = canonical_file(&config.dvd, "DVD image")?;
    let existing = scan_draft_manifests(&config.state_root)?;
    let generated_number = existing.len() + 1;
    let label = if request.label.trim().is_empty() {
        format!("Manual branch {generated_number}")
    } else {
        validate_draft_label(&request.label)?
    };
    let (
        mut materialized,
        parent,
        expected_start_milestone,
        expected_start_fingerprint,
        mut record_from_boot,
    ) = match request.parent {
        BrowserRecordParent::Origin { id } => {
            let graph = graph_from_timeline(timeline, &artifact_root)?;
            let origin = graph
                .origin
                .as_ref()
                .filter(|origin| origin.id == id)
                .ok_or_else(|| WorkbenchError::new(format!("unknown origin {id:?}")))?;
            if !origin.recordable_from_boot {
                return Err(WorkbenchError::new(format!(
                    "origin {id:?} is not the exact authored Boot boundary"
                )));
            }
            let program = origin.predicate_program.clone();
            let definition = program
                .definitions
                .iter()
                .find(|definition| definition.name == origin.predicate)
                .expect("graph origin predicate definition must exist");
            (
                MaterializedPlayback {
                    lineage: None,
                    segment: Some(format!("origin:{id}")),
                    tape: InputTape::default(),
                    seed_stage: None,
                    native_oracle: NativePlaybackOracle::None,
                },
                DraftParent::Milestone {
                    id: id.clone(),
                    program_sha256: program.program_sha256,
                    definition_sha256: definition.definition_sha256.clone(),
                    boundary_fingerprint: None,
                },
                Some(origin.predicate.clone()),
                None,
                true,
            )
        }
        BrowserRecordParent::Segment { id, terminal_goal } => {
            let segment = timeline
                .segments
                .get(&id)
                .ok_or_else(|| WorkbenchError::new(format!("unknown segment {id:?}")))?;
            let anchors = graph_from_timeline(timeline, &artifact_root)?
                .segments
                .into_iter()
                .find(|candidate| candidate.id == id)
                .expect("timeline segment must appear in its graph")
                .record_anchors;
            let anchor = anchors.iter().find(|anchor| anchor.goal == terminal_goal);
            if anchor.is_none() || !native_fingerprint(&segment.end_fingerprint) {
                return Err(WorkbenchError::new(
                    "recording requires a verified goal attached to the selected segment",
                ));
            }
            let segment_chain = materialize_segment_chain(timeline, &artifact_root, &id)?;
            let seed_stage = segment_chain.steps.first().and_then(|step| {
                legacy_seed_stage(
                    &segment_chain.tape,
                    timeline.segments[&step.segment].profile,
                )
            });
            let materialized = MaterializedPlayback {
                lineage: None,
                segment: Some(id.clone()),
                tape: segment_chain.tape,
                seed_stage,
                native_oracle: NativePlaybackOracle::None,
            };
            let parent = DraftParent::Segment {
                id: id.clone(),
                terminal_milestone: anchor.expect("checked anchor").predicate.clone(),
                boundary_fingerprint: segment.end_fingerprint.clone(),
            };
            (
                materialized,
                parent,
                Some(anchor.expect("checked anchor").predicate.clone()),
                Some(segment.end_fingerprint.clone()),
                false,
            )
        }
        BrowserRecordParent::Draft { id } => {
            let materialized =
                materialize_draft(timeline, &artifact_root, &config.state_root, &id)?;
            let digest = tape_digest(&materialized.tape)?;
            (
                materialized,
                DraftParent::Draft {
                    id,
                    parent_tape_sha256: digest,
                },
                None,
                None,
                false,
            )
        }
    };
    if let Some(configuration) =
        active_timeline_boot_override(&config.repository_root, &config.timeline_path)?
        && configuration.enabled
    {
        materialized.tape.boot = configuration.boot;
        if !matches!(materialized.tape.boot, TapeBoot::Process) {
            // A configured stage/loadout boot is carried by the zero-frame
            // playback prefix. Native recording then begins at that exact
            // configured origin instead of silently falling back to process boot.
            record_from_boot = false;
        }
    }
    verify_native_fidelity(&game, &config.working_directory)?;
    let parent_tape_sha256 = tape_digest(&materialized.tape)?;
    let root = drafts_root(&config.state_root)?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let draft_id = format!("draft-{}-{nonce}", std::process::id());
    let directory = root.join(&draft_id);
    fs::create_dir(&directory).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot create draft {}: {error}",
            directory.display()
        ))
    })?;
    let playback = directory.join("playback.tape");
    fs::write(
        &playback,
        materialized
            .tape
            .encode()
            .map_err(|error| WorkbenchError::new(error.to_string()))?,
    )
    .map_err(|error| WorkbenchError::new(format!("cannot write playback prefix: {error}")))?;
    let continuation = directory.join(DRAFT_TAPE);
    let state = directory.join("state");
    fs::create_dir(&state).map_err(|error| WorkbenchError::new(error.to_string()))?;
    let renderer_cache_root = config.state_root.join("renderer-cache");
    fs::create_dir_all(&renderer_cache_root).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot create renderer cache {}: {error}",
            renderer_cache_root.display()
        ))
    })?;
    let created_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let session_token = random_session_token()?;
    let manifest = DraftManifest {
        schema: DRAFT_SCHEMA.into(),
        id: draft_id.clone(),
        label,
        parent,
        parent_tape_sha256,
        created_unix_ms,
        session_token: session_token.clone(),
        expected_start_milestone: expected_start_milestone.clone(),
        expected_start_fingerprint: expected_start_fingerprint.clone(),
        tape: DRAFT_TAPE.into(),
        status: DraftStatus::Preparing,
        endpoint_kind: "manual_stop".into(),
        verification: "unverified".into(),
        start_boundary_verified: false,
        accelerated_parent_replay: !record_from_boot,
        parent_frames: materialized.tape.frames.len() as u64,
        tape_sha256: None,
        tape_bytes: None,
        result_tape_sha256: None,
        frames: None,
        error: None,
    };
    write_draft_manifest(&directory, &manifest, false)?;
    let mut command = Command::new(game);
    command
        .current_dir(&config.working_directory)
        .arg("--dvd")
        .arg(dvd);
    if record_from_boot {
        command.arg("--record-input-from-boot");
    } else {
        append_accelerated_recording_prefix(
            &mut command,
            &playback,
            materialized.tape.frames.len(),
            request.countdown_seconds,
        );
    }
    command
        .arg("--record-input-tape")
        .arg(&continuation)
        .arg("--record-input-thumbnail-png")
        .arg(directory.join(DRAFT_TERMINAL_THUMBNAIL))
        .arg("--record-input-capacity")
        .arg("1080000")
        .arg("--record-input-session")
        .arg(&session_token)
        .arg("--automation-data-root")
        .arg(&state)
        .arg("--renderer-cache-root")
        .arg(&renderer_cache_root);
    for cvar in FIXED_AUTOMATION_CVARS {
        command.arg("--cvar").arg(cvar);
    }
    append_origin_card_fixture_arg(
        timeline,
        &config.repository_root,
        &materialized.tape.boot,
        &mut command,
    )?;
    append_fixed_step_pacing(&mut command, request.speed_percent);
    if record_from_boot {
        command.arg("--record-input-start-milestone").arg(
            expected_start_milestone
                .as_deref()
                .expect("Boot recording has an authored start milestone"),
        );
    } else if let (Some(milestone), Some(fingerprint)) =
        (&expected_start_milestone, &expected_start_fingerprint)
    {
        command
            .arg("--record-input-start-milestone")
            .arg(milestone)
            .arg("--record-input-start-fingerprint")
            .arg(fingerprint);
    }
    if let Some(stage) = materialized.seed_stage {
        command.arg("--stage").arg(stage);
    }
    append_authored_milestone_args(
        timeline,
        &artifact_root,
        &state,
        &mut command,
        (!record_from_boot)
            .then_some(expected_start_milestone.as_deref())
            .flatten(),
    )?;
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            let mut failed = manifest;
            failed.status = DraftStatus::ProcessFailure;
            failed.error = Some(format!("cannot launch Dusklight: {error}"));
            let _ = write_draft_manifest(&directory, &failed, true);
            return Err(WorkbenchError::new(format!(
                "cannot launch Dusklight: {error}"
            )));
        }
    };
    let pid = child.id();
    let launch = DraftLaunch {
        schema: "dusklight.route-workbench.launch.v2".into(),
        id: draft_id.clone(),
        pid,
        session_token,
    };
    if let Err(error) = write_draft_launch(&directory, &launch) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    let monitor_directory = directory.clone();
    active_recordings()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(draft_id.clone());
    let monitor_id = draft_id.clone();
    let monitor_config = config.clone();
    thread::spawn(move || {
        monitor_recording(
            child,
            monitor_directory,
            manifest,
            monitor_id,
            monitor_config,
        )
    });
    Ok(RecordResponse {
        pid,
        draft_id,
        manifest: directory.join(DRAFT_MANIFEST),
        tape: continuation,
        frames_before_recording: materialized.tape.frames.len() as u64,
        speed_percent: request.speed_percent,
    })
}

pub(crate) fn monitor_recording(
    mut child: Child,
    directory: PathBuf,
    mut manifest: DraftManifest,
    draft_id: String,
    config: WorkbenchConfig,
) {
    match child.wait() {
        Ok(exit) => finalize_recording(&directory, &mut manifest, Some(exit.success())),
        Err(error) => {
            manifest.status = DraftStatus::ProcessFailure;
            manifest.error = Some(format!("cannot wait for Dusklight: {error}"));
        }
    }
    if let Err(error) = install_recording_thumbnail(&directory, &manifest, &config) {
        eprintln!("Route Workbench: {error}");
    }
    let _ = write_draft_manifest(&directory, &manifest, true);
    active_recordings()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(&draft_id);
}

pub(crate) fn finalize_recording(
    directory: &Path,
    manifest: &mut DraftManifest,
    exit_success: Option<bool>,
) {
    let status_path = directory.join(format!("{DRAFT_TAPE}.status.json"));
    let native: NativeRecordStatus = match fs::read(&status_path)
        .map_err(|error| error.to_string())
        .and_then(|bytes| serde_json::from_slice(&bytes).map_err(|error| error.to_string()))
    {
        Ok(status) => status,
        Err(error) => {
            manifest.status = DraftStatus::ProcessFailure;
            manifest.error = Some(format!(
                "missing or invalid native recording status: {error}"
            ));
            return;
        }
    };
    if native.schema != "dusklight.input-recording/v2"
        || native.session_token.as_deref() != Some(&manifest.session_token)
        || native.frame_capacity != 1_080_000
        || native.frame_count > native.frame_capacity
    {
        manifest.status = DraftStatus::ProcessFailure;
        manifest.error = Some("native recording status is inconsistent".into());
        return;
    }
    if exit_success.is_some_and(|exit_success| exit_success != native.process_success)
        || (native.process_success && native.status != "success")
    {
        manifest.status = DraftStatus::ProcessFailure;
        manifest.error = Some("native status and observed process exit disagree".into());
        return;
    }
    let boot_parent = match &manifest.parent {
        DraftParent::Milestone {
            id,
            program_sha256,
            definition_sha256,
            ..
        } => Some((id, program_sha256, definition_sha256)),
        DraftParent::Segment { .. } | DraftParent::Draft { .. } => None,
    };
    let expected_boundary_matches = match (
        boot_parent,
        &manifest.expected_start_milestone,
        &manifest.expected_start_fingerprint,
    ) {
        (Some((id, program, definition)), Some(milestone), None) => {
            milestone == id
                && native.start_milestone.as_deref() == Some(id)
                && native
                    .start_fingerprint
                    .as_deref()
                    .is_some_and(native_fingerprint)
                && native.expected_start_fingerprint.is_none()
                && native.start_boundary_kind.as_deref() == Some("boot")
                && native.start_boundary_index == Some(0)
                && native.start_program_digest.as_deref() == Some(program)
                && native.start_definition_digest.as_deref() == Some(definition)
                && native.start_tape_frame.is_none()
        }
        (None, Some(milestone), Some(fingerprint)) => {
            native.start_milestone.as_deref() == Some(milestone)
                && native.start_fingerprint.as_deref() == Some(fingerprint)
                && native.expected_start_fingerprint.as_deref() == Some(fingerprint)
                && (!manifest.accelerated_parent_replay
                    || (native.start_boundary_kind.as_deref() == Some("tick")
                        && native.start_boundary_index == Some(manifest.parent_frames)))
                && native.start_tape_frame == manifest.parent_frames.checked_sub(1)
        }
        (None, None, None) if manifest.accelerated_parent_replay => {
            native.start_milestone.is_none()
                && native.start_fingerprint.is_none()
                && native.expected_start_fingerprint.is_none()
                && native.start_boundary_kind.as_deref() == Some("tick")
                && native.start_boundary_index == Some(manifest.parent_frames)
                && native.start_tape_frame == manifest.parent_frames.checked_sub(1)
        }
        (None, None, None) => {
            native.start_milestone.is_none()
                && native.start_fingerprint.is_none()
                && native.expected_start_fingerprint.is_none()
                && native.start_tape_frame.is_none()
        }
        _ => false,
    };
    manifest.frames = Some(native.frame_count);
    manifest.error = native.error;
    manifest.status = match native.status.as_str() {
        "success"
            if native.process_success
                && native.handoff_reached
                && !native.capacity_exhausted
                && native.frame_count > 0
                && expected_boundary_matches =>
        {
            if capture_tape_metadata(directory, manifest, native.frame_count, false) {
                manifest.start_boundary_verified =
                    manifest.expected_start_milestone.is_some() && expected_boundary_matches;
                if let DraftParent::Milestone {
                    boundary_fingerprint,
                    ..
                } = &mut manifest.parent
                {
                    *boundary_fingerprint = native.start_fingerprint.clone();
                }
                DraftStatus::Ready
            } else {
                DraftStatus::ProcessFailure
            }
        }
        "zero_frames"
            if native.handoff_reached
                && !native.capacity_exhausted
                && native.frame_count == 0
                && expected_boundary_matches =>
        {
            if capture_tape_metadata(directory, manifest, 0, true) {
                DraftStatus::ZeroFrames
            } else {
                DraftStatus::ProcessFailure
            }
        }
        "never_reached_handoff" if !native.handoff_reached && native.frame_count == 0 => {
            DraftStatus::NeverReachedHandoff
        }
        "capacity_exhausted"
            if native.handoff_reached
                && native.capacity_exhausted
                && native.frame_count == native.frame_capacity
                && expected_boundary_matches =>
        {
            if capture_tape_metadata(directory, manifest, native.frame_count, false) {
                DraftStatus::CapacityExhausted
            } else {
                DraftStatus::ProcessFailure
            }
        }
        "write_failure" => DraftStatus::WriteFailure,
        "start_boundary_mismatch" => DraftStatus::StartBoundaryMismatch,
        _ => {
            manifest.error = Some("native recording status contradicts process result".into());
            DraftStatus::ProcessFailure
        }
    };
}
