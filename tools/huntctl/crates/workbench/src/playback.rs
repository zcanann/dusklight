use super::*;

/// Resolve a pinned continuation and concatenate its immutable segment artifacts. A frame
/// selector is inclusive: frame N's input is present in the output tape.
pub fn materialize_lineage(
    timeline: &Timeline,
    repository_root: &Path,
    lineage_name: &str,
    target: MaterializeTarget,
) -> Result<MaterializedLineage, WorkbenchError> {
    let inspection = timeline
        .inspect()
        .map_err(|error| WorkbenchError::new(error.to_string()))?;
    let lineage = inspection
        .lineages
        .iter()
        .find(|lineage| lineage.name == lineage_name)
        .ok_or_else(|| WorkbenchError::new(format!("unknown lineage {lineage_name:?}")))?;
    let selected = selected_step_count(timeline, lineage, &target)?;
    if selected == 0 {
        return Ok(MaterializedLineage {
            lineage: lineage_name.into(),
            tape: InputTape::default(),
            steps: Vec::new(),
        });
    }
    ensure_composable_lineage(timeline, lineage, selected)?;

    let mut chain = Vec::with_capacity(selected);
    for (index, step) in lineage.steps.iter().take(selected).enumerate() {
        let segment = &timeline.segments[&step.segment];
        let tape = load_segment_tape(segment, repository_root)?;
        let logical_last = logical_last_frame(segment, &tape)?;
        let frame_window = match &target {
            MaterializeTarget::ThroughSegmentFrame { segment, frame }
                if index + 1 == selected && step.segment == *segment =>
            {
                if *frame > logical_last {
                    return Err(WorkbenchError::new(format!(
                        "frame {frame} is outside segment {segment:?} (last logical frame is {logical_last})"
                    )));
                }
                SegmentFrames::ThroughMilestone { tape_frame: *frame }
            }
            _ => SegmentFrames::ThroughMilestone {
                tape_frame: logical_last,
            },
        };
        chain.push(ChainSegment {
            name: Some(segment.id.clone()),
            tape,
            markers: Vec::new(),
            frames: frame_window,
        });
    }
    let chained = concatenate(chain).map_err(|error| WorkbenchError::new(error.to_string()))?;
    let steps = chained
        .segments
        .iter()
        .map(|boundary| {
            let segment_id = boundary
                .segment_name
                .as_ref()
                .expect("workbench always names chain segments");
            MaterializedStep {
                segment: segment_id.clone(),
                source_start_frame: boundary.source_start_tick,
                source_end_frame: boundary.source_end_tick,
                chain_start_frame: boundary.chain_start_tick,
                chain_end_frame: boundary.chain_end_tick,
            }
        })
        .collect();
    Ok(MaterializedLineage {
        lineage: lineage_name.into(),
        tape: chained.tape,
        steps,
    })
}

/// Materialize the unique Boot-rooted ancestry of a segment. Named
/// continuations are bookmarks, not playback authorization: the parent links
/// and their exact fingerprints are the structural authority.
pub fn materialize_segment_chain(
    timeline: &Timeline,
    repository_root: &Path,
    segment_id: &str,
) -> Result<MaterializedLineage, WorkbenchError> {
    let mut reversed = Vec::new();
    let mut cursor = segment_id;
    let mut seen = BTreeSet::new();
    loop {
        if !seen.insert(cursor.to_owned()) {
            return Err(WorkbenchError::new(format!(
                "segment ancestry contains a cycle at {cursor:?}"
            )));
        }
        let segment = timeline
            .segments
            .get(cursor)
            .ok_or_else(|| WorkbenchError::new(format!("unknown segment {cursor:?}")))?;
        reversed.push(segment);
        let Some(parent_id) = segment.parent.as_deref() else {
            break;
        };
        let parent = timeline.segments.get(parent_id).ok_or_else(|| {
            WorkbenchError::new(format!(
                "segment {:?} references missing parent {parent_id:?}",
                segment.id
            ))
        })?;
        if parent.end_fingerprint != segment.start_fingerprint {
            return Err(WorkbenchError::new(format!(
                "segment {:?} starts at {}, but parent {parent_id:?} ends at {}",
                segment.id, segment.start_fingerprint, parent.end_fingerprint
            )));
        }
        cursor = parent_id;
    }
    reversed.reverse();

    let mut chain = Vec::with_capacity(reversed.len());
    for segment in reversed {
        if !artifact_is_canonical_payload(&segment.artifact) {
            return Err(WorkbenchError::new(format!(
                "segment {} is a stage-seeded baseline/candidate, not a canonical continuation tape",
                segment.id
            )));
        }
        if !fingerprints_are_exact(segment) {
            return Err(WorkbenchError::new(format!(
                "segment {} uses placeholder fingerprints",
                segment.id
            )));
        }
        let tape = load_segment_tape(segment, repository_root)?;
        let logical_last = logical_last_frame(segment, &tape)?;
        chain.push(ChainSegment {
            name: Some(segment.id.clone()),
            tape,
            markers: Vec::new(),
            frames: SegmentFrames::ThroughMilestone {
                tape_frame: logical_last,
            },
        });
    }
    let chained = concatenate(chain).map_err(|error| WorkbenchError::new(error.to_string()))?;
    let steps = chained
        .segments
        .iter()
        .map(|boundary| MaterializedStep {
            segment: boundary
                .segment_name
                .clone()
                .expect("workbench always names chain segments"),
            source_start_frame: boundary.source_start_tick,
            source_end_frame: boundary.source_end_tick,
            chain_start_frame: boundary.chain_start_tick,
            chain_end_frame: boundary.chain_end_tick,
        })
        .collect();
    Ok(MaterializedLineage {
        lineage: segment_id.to_owned(),
        tape: chained.tape,
        steps,
    })
}

/// Materialize and launch a headful fixed-step process. No shell is involved;
/// all paths become individual process arguments.
pub fn play(
    timeline: &Timeline,
    config: &WorkbenchConfig,
    request: &PlayRequest,
) -> Result<(PlayResponse, Child), WorkbenchError> {
    validate_play_request(request)?;
    let artifact_root = configured_artifact_root(config)?;
    let materialized = materialize_play_request(timeline, &artifact_root, request)?;
    launch_materialized(
        timeline,
        config,
        materialized,
        MaterializedLaunchOptions {
            takeover: request.takeover,
            origin: PlaybackOrigin::Boot,
            fast_forward_frames: None,
            thumbnail: None,
            playback: PlaybackSettings {
                speed_percent: 100,
                fast: false,
            },
        },
    )
}

pub(super) fn launch_materialized(
    timeline: &Timeline,
    config: &WorkbenchConfig,
    mut materialized: MaterializedPlayback,
    options: MaterializedLaunchOptions,
) -> Result<(PlayResponse, Child), WorkbenchError> {
    if !materialized
        .segment
        .as_deref()
        .is_some_and(|segment| segment.starts_with("project:"))
        && let Some(configuration) =
            active_timeline_boot_override(&config.repository_root, &config.timeline_path)?
        && configuration.enabled
    {
        materialized.tape.boot = configuration.boot;
    }
    let game = canonical_file(&config.game, "game executable")?;
    let dvd = canonical_file(&config.dvd, "DVD image")?;
    verify_native_fidelity(&game, &config.working_directory)?;
    fs::create_dir_all(&config.state_root).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot create state root {}: {error}",
            config.state_root.display()
        ))
    })?;
    let state_parent = fs::canonicalize(&config.state_root).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot resolve state root {}: {error}",
            config.state_root.display()
        ))
    })?;
    let renderer_cache_root = state_parent.join("renderer-cache");
    fs::create_dir_all(&renderer_cache_root).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot create renderer cache {}: {error}",
            renderer_cache_root.display()
        ))
    })?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let session_id = format!("{}-{nonce}", std::process::id());
    let state_root = state_parent.join(format!("session-{session_id}"));
    fs::create_dir(&state_root).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot create fresh session {}: {error}",
            state_root.display()
        ))
    })?;
    let tape_path = state_root.join("playback.tape");
    let encoded = materialized
        .tape
        .encode()
        .map_err(|error| WorkbenchError::new(error.to_string()))?;
    fs::write(&tape_path, encoded).map_err(|error| {
        WorkbenchError::new(format!("cannot write {}: {error}", tape_path.display()))
    })?;
    let end = if options.takeover { "release" } else { "hold" };
    let mut command = Command::new(&game);
    command.current_dir(&config.working_directory);
    append_playback_args(
        &mut command,
        &dvd,
        &tape_path,
        end,
        &state_root,
        PlaybackCliOptions {
            seed_stage: materialized.seed_stage,
            fast_forward_frames: options.fast_forward_frames,
            playback: options.playback,
        },
    );
    append_native_oracle_args(&mut command, &state_root, materialized.native_oracle);
    if let Some(thumbnail) = &options.thumbnail {
        command
            .arg("--input-tape-thumbnail-png")
            .arg(&thumbnail.path);
    }
    let artifact_root = configured_artifact_root(config)?;
    append_authored_milestone_args(timeline, &artifact_root, &state_root, &mut command, None)?;
    let child = command
        .spawn()
        .map_err(|error| WorkbenchError::new(format!("cannot launch Dusklight: {error}")))?;
    let response = PlayResponse {
        pid: child.id(),
        lineage: materialized.lineage,
        segment: materialized.segment,
        tape: tape_path,
        session_state_root: state_root,
        session_id,
        frames: materialized.tape.frames.len() as u64,
        input_tape_end: end.into(),
        origin: options.origin,
        speed_percent: options.playback.speed_percent,
        fast: options.playback.fast,
        fast_forward_frames: options.fast_forward_frames,
        thumbnail: options.thumbnail.map(|thumbnail| thumbnail.url),
    };
    Ok((response, child))
}

pub(super) fn capture_thumbnail(
    timeline: &Timeline,
    config: &WorkbenchConfig,
    request: &BrowserThumbnailCaptureRequest,
) -> Result<(ThumbnailCaptureResponse, Child), WorkbenchError> {
    let game = canonical_file(&config.game, "game executable")?;
    let dvd = canonical_file(&config.dvd, "DVD image")?;
    let artifact_root = configured_artifact_root(config)?;
    let mut graph = graph_with_drafts(timeline, &artifact_root, &config.state_root)?;
    graph.projects = project_catalog_projection(&config.repository_root, &config.timeline_path)?;
    let key = graph_node_thumbnail_key(&graph, &request.selection)?;
    let materialized = match &request.selection {
        BrowserSelection::Segment { id } => {
            materialize_segment_playback(timeline, &artifact_root, id, None)?
        }
        BrowserSelection::Draft { id } => {
            materialize_draft(timeline, &artifact_root, &config.state_root, id)?
        }
        BrowserSelection::Project { id } => {
            project_materialized_playback(&config.repository_root, id)?
        }
    };
    verify_native_fidelity(&game, &config.working_directory)?;

    fs::create_dir_all(&config.state_root).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot create state root {}: {error}",
            config.state_root.display()
        ))
    })?;
    let state_parent = fs::canonicalize(&config.state_root).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot resolve state root {}: {error}",
            config.state_root.display()
        ))
    })?;
    let thumbnail_root = state_parent.join(THUMBNAIL_DIRECTORY);
    fs::create_dir_all(&thumbnail_root).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot create thumbnail cache {}: {error}",
            thumbnail_root.display()
        ))
    })?;
    let thumbnail_root = fs::canonicalize(&thumbnail_root).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot resolve thumbnail cache {}: {error}",
            thumbnail_root.display()
        ))
    })?;
    if thumbnail_root.parent() != Some(state_parent.as_path()) {
        return Err(WorkbenchError::new(
            "thumbnail cache escapes the workbench state root",
        ));
    }
    let thumbnail_path = thumbnail_root.join(format!("{key}.png"));
    let renderer_cache_root = state_parent.join("renderer-cache");
    fs::create_dir_all(&renderer_cache_root).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot create renderer cache {}: {error}",
            renderer_cache_root.display()
        ))
    })?;

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let session_root =
        state_parent.join(format!("thumbnail-session-{}-{nonce}", std::process::id()));
    fs::create_dir(&session_root).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot create fresh thumbnail session {}: {error}",
            session_root.display()
        ))
    })?;
    let tape_path = session_root.join("playback.tape");
    let encoded = materialized
        .tape
        .encode()
        .map_err(|error| WorkbenchError::new(error.to_string()))?;
    fs::write(&tape_path, encoded).map_err(|error| {
        WorkbenchError::new(format!("cannot write {}: {error}", tape_path.display()))
    })?;

    let mut command = Command::new(&game);
    command.current_dir(&config.working_directory);
    append_playback_args(
        &mut command,
        &dvd,
        &tape_path,
        "release",
        &session_root,
        PlaybackCliOptions {
            seed_stage: materialized.seed_stage,
            fast_forward_frames: None,
            playback: PlaybackSettings {
                speed_percent: 100,
                fast: false,
            },
        },
    );
    append_native_oracle_args(&mut command, &session_root, materialized.native_oracle);
    command
        .arg("--unpaced")
        .arg("--exit-after-tape")
        .arg("--frame-capture-png")
        .arg(&thumbnail_path)
        .arg("--frame-capture-width")
        .arg(THUMBNAIL_WIDTH.to_string())
        .arg("--frame-capture-height")
        .arg(THUMBNAIL_HEIGHT.to_string());
    append_authored_milestone_args(timeline, &artifact_root, &session_root, &mut command, None)?;
    let child = command.spawn().map_err(|error| {
        WorkbenchError::new(format!("cannot launch thumbnail capture: {error}"))
    })?;
    let response = ThumbnailCaptureResponse {
        schema: THUMBNAIL_CAPTURE_SCHEMA.into(),
        pid: child.id(),
        key: key.clone(),
        thumbnail: thumbnail_url(&key),
        frames: materialized.tape.frames.len() as u64,
    };
    Ok((response, child))
}

pub(super) fn play_draft(
    timeline: &Timeline,
    config: &WorkbenchConfig,
    draft_id: &str,
    speed_percent: u16,
    fast: bool,
) -> Result<(PlayResponse, Child), WorkbenchError> {
    let artifact_root = configured_artifact_root(config)?;
    let materialized = materialize_draft(timeline, &artifact_root, &config.state_root, draft_id)?;
    let fast_forward_frames = playback_fast_forward_frames(
        PlaybackSettings {
            speed_percent: if fast { 0 } else { speed_percent },
            fast,
        },
        materialized.tape.frames.len() as u64,
    );
    let thumbnail = prepare_missing_playback_thumbnail(
        timeline,
        config,
        &BrowserSelection::Draft {
            id: draft_id.into(),
        },
    )?;
    launch_materialized(
        timeline,
        config,
        materialized,
        MaterializedLaunchOptions {
            takeover: true,
            origin: PlaybackOrigin::Boot,
            fast_forward_frames,
            thumbnail,
            playback: PlaybackSettings {
                speed_percent: if fast { 0 } else { speed_percent },
                fast,
            },
        },
    )
}

pub(super) fn play_project(
    timeline: &Timeline,
    config: &WorkbenchConfig,
    project_id: &str,
    handoff: bool,
    playback: PlaybackSettings,
) -> Result<(PlayResponse, Child), WorkbenchError> {
    let materialized = project_materialized_playback(&config.repository_root, project_id)?;
    let fast_forward_frames =
        playback_fast_forward_frames(playback, materialized.tape.frames.len() as u64);
    let thumbnail = prepare_missing_playback_thumbnail(
        timeline,
        config,
        &BrowserSelection::Project {
            id: project_id.into(),
        },
    )?;
    launch_materialized(
        timeline,
        config,
        materialized,
        MaterializedLaunchOptions {
            takeover: handoff,
            origin: PlaybackOrigin::Boot,
            fast_forward_frames,
            thumbnail,
            playback,
        },
    )
}

#[cfg(test)]
pub(super) fn draft_parent_frame_count(
    timeline: &Timeline,
    repository_root: &Path,
    state_root: &Path,
    draft_id: &str,
    full_frames: u64,
) -> Result<u64, WorkbenchError> {
    let manifests = scan_draft_manifests(state_root)?;
    let manifest = manifests
        .get(draft_id)
        .ok_or_else(|| WorkbenchError::new(format!("unknown draft {draft_id:?}")))?;
    if manifest.status != DraftStatus::Ready {
        return Err(WorkbenchError::new(format!(
            "draft {draft_id:?} is not ready"
        )));
    }
    let draft_directory = drafts_root(state_root)?.join(draft_id);
    let (_, continuation) = read_draft_tape(&draft_directory)?;
    let continuation_frames = continuation.frames.len() as u64;
    let parent = match &manifest.parent {
        DraftParent::Milestone { .. } => InputTape::default(),
        DraftParent::Segment { id, .. } => {
            materialize_segment_chain(timeline, repository_root, id)?.tape
        }
        DraftParent::Draft { id, .. } => {
            materialize_draft(timeline, repository_root, state_root, id)?.tape
        }
    };
    let parent_frames = parent.frames.len() as u64;
    if tape_digest(&parent)? != manifest.parent_tape_sha256 {
        return Err(WorkbenchError::new(format!(
            "draft {draft_id:?} direct-parent tape fingerprint changed"
        )));
    }
    validate_parent_boundary_metadata(
        parent_frames,
        continuation_frames,
        manifest.parent_frames,
        manifest.frames,
        full_frames,
    )
    .map_err(|_| {
        WorkbenchError::new(format!(
            "draft {draft_id:?} has invalid direct-parent playback boundary metadata"
        ))
    })?;
    Ok(parent_frames)
}

#[cfg(test)]
pub(super) fn validate_parent_boundary_metadata(
    actual_parent_frames: u64,
    actual_continuation_frames: u64,
    declared_parent_frames: u64,
    declared_continuation_frames: Option<u64>,
    full_frames: u64,
) -> Result<(), ()> {
    if declared_parent_frames != actual_parent_frames
        || declared_continuation_frames != Some(actual_continuation_frames)
    {
        return Err(());
    }
    validate_parent_boundary(
        actual_parent_frames,
        actual_continuation_frames,
        full_frames,
    )
}

#[cfg(test)]
pub(super) fn validate_parent_boundary(
    parent_frames: u64,
    continuation_frames: u64,
    full_frames: u64,
) -> Result<(), ()> {
    if parent_frames == 0
        || parent_frames >= full_frames
        || continuation_frames == 0
        || parent_frames.checked_add(continuation_frames) != Some(full_frames)
    {
        Err(())
    } else {
        Ok(())
    }
}

pub(super) fn append_authored_milestone_args(
    timeline: &Timeline,
    artifact_root: &Path,
    state_root: &Path,
    command: &mut Command,
    additional_builtin: Option<&str>,
) -> Result<(), WorkbenchError> {
    let mut source_paths = Vec::new();
    if let Some(path) = timeline.origin_predicate_source() {
        source_paths.push(path);
    }
    for goal in timeline.goals.values() {
        if let Some(path) = timeline.goal_predicate_source(&goal.id)
            && !source_paths.contains(&path)
        {
            source_paths.push(path);
        }
    }
    if source_paths.is_empty() {
        return Ok(());
    }
    let mut combined: Option<MilestoneProgram> = None;
    let mut definition_names = BTreeSet::new();
    for relative in source_paths {
        let source_path = validated_predicate_source_path(relative, artifact_root)?;
        let source = fs::read_to_string(&source_path).map_err(|error| {
            WorkbenchError::new(format!(
                "cannot read configured predicate source {}: {error}",
                source_path.display()
            ))
        })?;
        let mut program = milestone_dsl::parse(&source).map_err(|error| {
            WorkbenchError::new(format!(
                "invalid predicate source {}: {error}",
                source_path.display()
            ))
        })?;
        if let Some(combined) = &mut combined {
            if combined.version != program.version {
                return Err(WorkbenchError::new(
                    "owned predicate sources use incompatible language versions",
                ));
            }
            for definition in program.definitions.drain(..) {
                if !definition_names.insert(definition.name.clone()) {
                    return Err(WorkbenchError::new(format!(
                        "owned predicate sources define duplicate predicate {:?}",
                        definition.name
                    )));
                }
                combined.definitions.push(definition);
            }
        } else {
            for definition in &program.definitions {
                definition_names.insert(definition.name.clone());
            }
            combined = Some(program);
        }
    }
    let compiled = milestone_dsl::compile(
        &combined.expect("at least one owned predicate source was collected"),
    )
    .map_err(|error| WorkbenchError::new(format!("cannot compile owned predicates: {error}")))?;
    let program_path = state_root.join("route-milestones.dmsp");
    let result_path = state_root.join("route-milestones.json");
    fs::write(&program_path, &compiled.bytes).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot write compiled milestone program {}: {error}",
            program_path.display()
        ))
    })?;
    let mut requested = timeline
        .origin
        .iter()
        .map(|origin| origin.predicate.clone())
        .collect::<Vec<_>>();
    for predicate in timeline.goals.values().map(|goal| goal.predicate.clone()) {
        if !requested.contains(&predicate) {
            requested.push(predicate);
        }
    }
    if let Some(id) = additional_builtin
        && !requested.iter().any(|existing| existing == id)
    {
        requested.push(id.to_owned());
    }
    command
        .arg("--milestone-program")
        .arg(program_path)
        .arg("--milestones")
        .arg(requested.join(","))
        .arg("--milestone-result")
        .arg(result_path);
    Ok(())
}

pub(super) fn append_playback_args(
    command: &mut Command,
    dvd: &Path,
    tape: &Path,
    end: &str,
    state_root: &Path,
    options: PlaybackCliOptions<'_>,
) {
    let renderer_cache_root = state_root
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join("renderer-cache");
    command
        .arg("--dvd")
        .arg(dvd)
        .arg("--input-tape")
        .arg(tape)
        .arg("--input-tape-end")
        .arg(end);
    if let Some(frames) = options.fast_forward_frames {
        command
            .arg("--input-tape-fast-forward-frames")
            .arg(frames.to_string());
    }
    command
        .arg("--automation-data-root")
        .arg(state_root)
        .arg("--renderer-cache-root")
        .arg(&renderer_cache_root)
        .arg("--cvar")
        .arg("game.instantSaves=true")
        .arg("--cvar")
        .arg("backend.cardFileType=1")
        .arg("--cvar")
        .arg("backend.wasPresetChosen=true")
        .arg("--cvar")
        .arg("game.enableMenuPointer=false");
    append_fixed_step_pacing(command, options.playback.speed_percent);
    if let Some(stage) = options.seed_stage {
        command.arg("--stage").arg(stage);
    }
}

fn append_native_oracle_args(
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

fn verify_native_fidelity(game: &Path, working_directory: &Path) -> Result<(), WorkbenchError> {
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

fn validate_native_fidelity_identity(hello: &serde_json::Value) -> Result<(), WorkbenchError> {
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

pub(super) fn append_fixed_step_pacing(command: &mut Command, speed_percent: u16) {
    command
        .arg("--fixed-step")
        .arg("--fixed-step-speed-percent")
        .arg(speed_percent.to_string());
}

pub(super) fn validate_draft_label(label: &str) -> Result<String, WorkbenchError> {
    let label = label.trim();
    if label.is_empty() || label.len() > 160 || label.chars().any(char::is_control) {
        return Err(WorkbenchError::new(
            "draft label must be 1 to 160 UTF-8 bytes without controls",
        ));
    }
    Ok(label.to_owned())
}

pub(super) fn append_accelerated_recording_prefix(
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

pub(super) fn record_continuation(
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
        .arg(&renderer_cache_root)
        .arg("--cvar")
        .arg("game.instantSaves=true")
        .arg("--cvar")
        .arg("backend.cardFileType=1")
        .arg("--cvar")
        .arg("backend.wasPresetChosen=true")
        .arg("--cvar")
        .arg("game.enableMenuPointer=false");
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

pub(super) fn monitor_recording(
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

pub(super) fn finalize_recording(
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

pub(super) fn capture_tape_metadata(
    directory: &Path,
    manifest: &mut DraftManifest,
    expected_frames: u64,
    allow_empty: bool,
) -> bool {
    let expected = directory.join(DRAFT_TAPE);
    let status_tape_matches = fs::canonicalize(&expected)
        .ok()
        .zip(
            fs::read(directory.join(format!("{DRAFT_TAPE}.status.json")))
                .ok()
                .and_then(|bytes| serde_json::from_slice::<NativeRecordStatus>(&bytes).ok())
                .and_then(|status| fs::canonicalize(status.tape).ok()),
        )
        .is_some_and(|(left, right)| left == right);
    let result = read_draft_tape(directory);
    match result {
        Ok((bytes, tape))
            if status_tape_matches
                && tape.frames.len() as u64 == expected_frames
                && (allow_empty || !tape.frames.is_empty()) =>
        {
            manifest.tape_sha256 = Some(format!("{:x}", Sha256::digest(&bytes)));
            manifest.tape_bytes = Some(bytes.len() as u64);
            manifest.result_tape_sha256 = fs::read(directory.join("playback.tape"))
                .ok()
                .and_then(|prefix| InputTape::decode(&prefix).ok())
                .and_then(|prefix| {
                    concatenate(vec![
                        ChainSegment::all(prefix.tape),
                        ChainSegment::all(tape),
                    ])
                    .ok()
                })
                .and_then(|chain| chain.tape.encode().ok())
                .map(|result| format!("{:x}", Sha256::digest(result)));
            if manifest.result_tape_sha256.is_none() {
                manifest.error = Some("cannot fingerprint finalized draft chain".into());
                return false;
            }
            true
        }
        _ => {
            manifest.error = Some("native recording tape is missing or inconsistent".into());
            false
        }
    }
}

/// Serve the graph and playback API. The listener must be loopback-only: the
/// play endpoint can start a user-selected executable and must not be exposed.
pub(super) fn graph_artifact(source: &ArtifactSource) -> GraphArtifact {
    match source {
        ArtifactSource::Baseline(profile) => GraphArtifact {
            kind: "baseline".into(),
            value: profile.as_str().into(),
        },
        ArtifactSource::Candidate(path) => GraphArtifact {
            kind: "candidate".into(),
            value: path.display().to_string(),
        },
        ArtifactSource::Tas(path) => GraphArtifact {
            kind: "tas".into(),
            value: path.display().to_string(),
        },
        ArtifactSource::Tape(path) => GraphArtifact {
            kind: "tape".into(),
            value: path.display().to_string(),
        },
        #[allow(unreachable_patterns)]
        _ => GraphArtifact {
            kind: "unsupported".into(),
            value: "artifact source is not supported by this workbench build".into(),
        },
    }
}

pub(super) fn selected_step_count(
    timeline: &Timeline,
    lineage: &ResolvedLineage,
    target: &MaterializeTarget,
) -> Result<usize, WorkbenchError> {
    match target {
        MaterializeTarget::FullLineage => Ok(lineage.steps.len()),
        MaterializeTarget::ThroughSegment(segment) => {
            unique_segment_step(timeline, lineage, segment)
        }
        MaterializeTarget::ThroughSegmentFrame { segment, .. } => {
            unique_segment_step(timeline, lineage, segment)
        }
        MaterializeTarget::ThroughStepCount(count) => {
            if *count <= lineage.steps.len() {
                Ok(*count)
            } else {
                Err(WorkbenchError::new(format!(
                    "step count {count} is outside lineage {:?} ({} steps)",
                    lineage.name,
                    lineage.steps.len()
                )))
            }
        }
    }
}

pub(super) fn unique_segment_step(
    _timeline: &Timeline,
    lineage: &ResolvedLineage,
    segment: &str,
) -> Result<usize, WorkbenchError> {
    let matches = lineage
        .steps
        .iter()
        .enumerate()
        .filter(|(_, step)| step.segment == segment)
        .map(|(index, _)| index + 1)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [step] => Ok(*step),
        [] => Err(WorkbenchError::new(format!(
            "segment {segment:?} is not on lineage {:?}",
            lineage.name
        ))),
        _ => Err(WorkbenchError::new(format!(
            "segment {segment:?} occurs more than once on lineage {:?}; use an exact step count",
            lineage.name
        ))),
    }
}

pub(super) fn ensure_composable_lineage(
    timeline: &Timeline,
    lineage: &ResolvedLineage,
    selected: usize,
) -> Result<(), WorkbenchError> {
    // A single segment can always be played under its profile seed. Crossing a
    // boundary is stricter: generated baselines/candidates may contain their
    // evaluation seed harness and therefore are not continuation payloads.
    if selected <= 1 {
        return Ok(());
    }
    ensure_canonical_prefix(timeline, lineage, selected)
}

pub(super) fn ensure_canonical_prefix(
    timeline: &Timeline,
    lineage: &ResolvedLineage,
    selected: usize,
) -> Result<(), WorkbenchError> {
    for step in lineage.steps.iter().take(selected) {
        let segment = &timeline.segments[&step.segment];
        if !artifact_is_canonical_payload(&segment.artifact) {
            return Err(WorkbenchError::new(format!(
                "continuation {:?} cannot cross segment {}: it is a stage-seeded baseline/candidate, not a canonical continuation tape",
                lineage.name, segment.id
            )));
        }
        if !fingerprints_are_exact(segment)
            || contains_placeholder(&step.after.checkpoint_fingerprint)
        {
            return Err(WorkbenchError::new(format!(
                "continuation {:?} cannot cross segment {} because it uses placeholder fingerprints",
                lineage.name, segment.id
            )));
        }
    }
    Ok(())
}

pub(super) fn artifact_is_canonical_payload(source: &ArtifactSource) -> bool {
    // `uses tape` is the current DSL's explicit compact, immutable payload.
    // Baseline and candidate sources are profile-seeded evaluation programs.
    matches!(source, ArtifactSource::Tas(_) | ArtifactSource::Tape(_))
}

pub(super) fn fingerprints_are_exact(segment: &Segment) -> bool {
    !contains_placeholder(&segment.start_fingerprint)
        && !contains_placeholder(&segment.end_fingerprint)
}

pub(super) fn contains_placeholder(value: &str) -> bool {
    value.trim().is_empty() || value.to_ascii_lowercase().contains("unknown")
}

pub(super) fn logical_last_frame(
    segment: &Segment,
    tape: &InputTape,
) -> Result<u64, WorkbenchError> {
    if tape.frames.is_empty() {
        return Err(WorkbenchError::new(format!(
            "segment {} has an empty tape",
            segment.id
        )));
    }
    // first_hit_tick is a simulation score, not an artifact-local frame
    // boundary. Until canonical bundles carry an explicit tape_frame, the
    // complete artifact is the only safe payload boundary.
    Ok(tape.frames.len() as u64 - 1)
}

pub(super) fn option_diagnostic_relative_path(source: &ArtifactSource) -> Option<PathBuf> {
    let artifact = match source {
        ArtifactSource::Candidate(path)
        | ArtifactSource::Tas(path)
        | ArtifactSource::Tape(path) => path,
        ArtifactSource::Baseline(_) => return None,
        #[allow(unreachable_patterns)]
        _ => return None,
    };
    let mut sidecar = artifact.as_os_str().to_os_string();
    sidecar.push(".options.json");
    Some(PathBuf::from(sidecar))
}

pub(super) fn load_option_visualization(
    segment: &Segment,
    repository_root: &Path,
    tape: &InputTape,
) -> Result<Vec<OptionVisualization>, WorkbenchError> {
    let Some(relative) = option_diagnostic_relative_path(&segment.artifact) else {
        return Ok(Vec::new());
    };
    let unresolved = repository_root.join(&relative);
    let Ok(metadata) = fs::symlink_metadata(&unresolved) else {
        return Ok(Vec::new());
    };
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > MAX_OPTION_DIAGNOSTIC_BYTES
    {
        return Err(WorkbenchError::new(format!(
            "option diagnostic sidecar {} must be a regular file no larger than {} bytes",
            relative.display(),
            MAX_OPTION_DIAGNOSTIC_BYTES
        )));
    }
    let path = checked_artifact_path(repository_root, &relative)?;
    let bytes = fs::read(&path)
        .map_err(|error| WorkbenchError::new(format!("cannot read {}: {error}", path.display())))?;
    let bundle: OptionDiagnosticBundle = serde_json::from_slice(&bytes).map_err(|error| {
        WorkbenchError::new(format!("cannot decode {}: {error}", path.display()))
    })?;
    bundle.validate_against_tape(tape).map_err(|error| {
        WorkbenchError::new(format!(
            "invalid option diagnostics {}: {error}",
            path.display()
        ))
    })?;
    Ok(bundle.visualization())
}

pub(super) fn load_segment_tape(
    segment: &Segment,
    repository_root: &Path,
) -> Result<InputTape, WorkbenchError> {
    let profile = segment.profile;
    match &segment.artifact {
        ArtifactSource::Baseline(candidate_profile) => {
            if *candidate_profile != profile {
                return Err(WorkbenchError::new(format!(
                    "segment {} baseline profile does not match its profile",
                    segment.id
                )));
            }
            Candidate::baseline(*candidate_profile)
                .compile()
                .map_err(|error| WorkbenchError::new(error.to_string()))
        }
        ArtifactSource::Candidate(path) => {
            let path = checked_artifact_path(repository_root, path)?;
            let bytes = fs::read(&path).map_err(|error| {
                WorkbenchError::new(format!("cannot read {}: {error}", path.display()))
            })?;
            let candidate: Candidate = serde_json::from_slice(&bytes).map_err(|error| {
                WorkbenchError::new(format!("cannot decode {}: {error}", path.display()))
            })?;
            if candidate.segment != profile {
                return Err(WorkbenchError::new(format!(
                    "candidate {} has the wrong segment profile",
                    path.display()
                )));
            }
            candidate
                .compile()
                .map_err(|error| WorkbenchError::new(error.to_string()))
        }
        ArtifactSource::Tas(path) => {
            let path = checked_artifact_path(repository_root, path)?;
            let source = fs::read_to_string(&path).map_err(|error| {
                WorkbenchError::new(format!("cannot read {}: {error}", path.display()))
            })?;
            crate::tape_dsl::parse(&source)
                .map_err(|error| {
                    WorkbenchError::new(format!("cannot parse {}: {error}", path.display()))
                })?
                .compile()
                .map(|compiled| compiled.tape)
                .map_err(|error| {
                    WorkbenchError::new(format!("cannot compile {}: {error}", path.display()))
                })
        }
        ArtifactSource::Tape(path) => {
            let path = checked_artifact_path(repository_root, path)?;
            let bytes = fs::read(&path).map_err(|error| {
                WorkbenchError::new(format!("cannot read {}: {error}", path.display()))
            })?;
            InputTape::decode(&bytes)
                .map(|decoded| decoded.tape)
                .map_err(|error| {
                    WorkbenchError::new(format!("cannot decode {}: {error}", path.display()))
                })
        }
        #[allow(unreachable_patterns)]
        _ => Err(WorkbenchError::new(
            "artifact source is not supported by this workbench build",
        )),
    }
}

pub(super) fn checked_artifact_path(
    root: &Path,
    relative: &Path,
) -> Result<PathBuf, WorkbenchError> {
    if relative.is_absolute() {
        return Err(WorkbenchError::new(format!(
            "artifact path {} must be repository-relative",
            relative.display()
        )));
    }
    let canonical_root = fs::canonicalize(root).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot resolve repository root {}: {error}",
            root.display()
        ))
    })?;
    let candidate = fs::canonicalize(canonical_root.join(relative)).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot resolve artifact {}: {error}",
            relative.display()
        ))
    })?;
    if !candidate.starts_with(&canonical_root) {
        return Err(WorkbenchError::new(format!(
            "artifact {} escapes the repository root",
            relative.display()
        )));
    }
    if !candidate.is_file() {
        return Err(WorkbenchError::new(format!(
            "artifact {} is not a file",
            relative.display()
        )));
    }
    Ok(candidate)
}

pub(super) fn canonical_file(path: &Path, label: &str) -> Result<PathBuf, WorkbenchError> {
    let canonical = fs::canonicalize(path).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot resolve {label} {}: {error}",
            path.display()
        ))
    })?;
    if !canonical.is_file() {
        return Err(WorkbenchError::new(format!(
            "{label} {} is not a file",
            path.display()
        )));
    }
    Ok(canonical)
}

pub(super) fn validate_play_request(request: &PlayRequest) -> Result<(), WorkbenchError> {
    match (&request.lineage, &request.standalone_segment) {
        (Some(lineage), None) if !lineage.trim().is_empty() => {}
        (None, Some(segment)) if !segment.trim().is_empty() => {}
        (Some(_), Some(_)) => {
            return Err(WorkbenchError::new(
                "lineage and standalone_segment are mutually exclusive",
            ));
        }
        _ => {
            return Err(WorkbenchError::new(
                "lineage or standalone_segment is required",
            ));
        }
    }
    match (
        &request.segment,
        request.frame,
        request.standalone_segment.is_some(),
    ) {
        (Some(_), Some(_), false) | (None, None, _) | (None, Some(_), true) => {}
        _ => {
            return Err(WorkbenchError::new(
                "segment and frame must be supplied together",
            ));
        }
    }
    if request.through_segment.is_some() && request.segment.is_some() {
        return Err(WorkbenchError::new(
            "through_segment and segment/frame are mutually exclusive",
        ));
    }
    if request.standalone_segment.is_some()
        && (request.through_segment.is_some() || request.segment.is_some())
    {
        return Err(WorkbenchError::new(
            "standalone segment playback accepts frame only, not lineage segment selectors",
        ));
    }
    Ok(())
}

pub(super) fn materialize_play_request(
    timeline: &Timeline,
    repository_root: &Path,
    request: &PlayRequest,
) -> Result<MaterializedPlayback, WorkbenchError> {
    validate_play_request(request)?;
    if let Some(segment_id) = &request.standalone_segment {
        return materialize_segment_playback(timeline, repository_root, segment_id, request.frame);
    }
    let lineage = request.lineage.as_deref().expect("validated lineage");
    let materialized =
        materialize_lineage(timeline, repository_root, lineage, play_target(request)?)?;
    let seed_stage = materialized.steps.first().and_then(|step| {
        legacy_seed_stage(&materialized.tape, timeline.segments[&step.segment].profile)
    });
    Ok(MaterializedPlayback {
        lineage: Some(lineage.into()),
        segment: None,
        tape: materialized.tape,
        seed_stage,
        native_oracle: NativePlaybackOracle::None,
    })
}

pub(super) fn materialize_segment_playback(
    timeline: &Timeline,
    repository_root: &Path,
    segment_id: &str,
    local_frame: Option<u64>,
) -> Result<MaterializedPlayback, WorkbenchError> {
    let segment = timeline
        .segments
        .get(segment_id)
        .ok_or_else(|| WorkbenchError::new(format!("unknown segment {segment_id:?}")))?;
    let mut chain = materialize_segment_chain(timeline, repository_root, segment_id)?;
    if let Some(frame) = local_frame {
        let local_last =
            logical_last_frame(segment, &load_segment_tape(segment, repository_root)?)?;
        if frame > local_last {
            return Err(WorkbenchError::new(format!(
                "frame {frame} is outside segment {segment_id:?} (last logical frame is {local_last})"
            )));
        }
        let selected = chain.steps.last().expect("segment chain is non-empty");
        let chain_last = selected
            .chain_start_frame
            .checked_add(frame)
            .ok_or_else(|| WorkbenchError::new("frame selection overflow"))?;
        chain.tape.frames.truncate(chain_last as usize + 1);
    }
    let seed_profile = chain
        .steps
        .first()
        .map(|step| timeline.segments[&step.segment].profile)
        .unwrap_or(segment.profile);
    let seed_stage = legacy_seed_stage(&chain.tape, seed_profile);
    Ok(MaterializedPlayback {
        lineage: None,
        segment: Some(segment_id.into()),
        tape: chain.tape,
        seed_stage,
        native_oracle: NativePlaybackOracle::None,
    })
}

pub(super) fn play_segment(
    timeline: &Timeline,
    config: &WorkbenchConfig,
    segment_id: &str,
    stop: &BrowserStop,
    options: SegmentPlaybackOptions,
) -> Result<(PlayResponse, Child), WorkbenchError> {
    if !timeline.segments.contains_key(segment_id) {
        if !matches!(stop, BrowserStop::Segment { segment } if segment == segment_id) {
            return Err(WorkbenchError::new(
                "generated search playback only supports its proved endpoint",
            ));
        }
        let projection = visible_generated_search_projections(
            timeline,
            &config.repository_root.join("build/search"),
            &config.state_root,
        )?
        .into_iter()
        .find(|projection| projection.segment.id == segment_id)
        .ok_or_else(|| {
            WorkbenchError::new(format!(
                "unknown or expired generated search segment {segment_id:?}"
            ))
        })?;
        let bytes = fs::read(&projection.full_tape).map_err(|error| {
            WorkbenchError::new(format!(
                "cannot read generated search tape {}: {error}",
                projection.full_tape.display()
            ))
        })?;
        let tape = InputTape::decode(&bytes)
            .map_err(|error| WorkbenchError::new(format!("invalid generated tape: {error}")))?
            .tape;
        let materialized = MaterializedPlayback {
            lineage: None,
            segment: Some(segment_id.into()),
            tape,
            seed_stage: None,
            native_oracle: NativePlaybackOracle::None,
        };
        let fast_forward_frames =
            playback_fast_forward_frames(options.playback, materialized.tape.frames.len() as u64);
        return launch_materialized(
            timeline,
            config,
            materialized,
            MaterializedLaunchOptions {
                takeover: options.handoff,
                origin: PlaybackOrigin::Boot,
                fast_forward_frames,
                thumbnail: None,
                playback: options.playback,
            },
        );
    }
    let local_frame = match stop {
        BrowserStop::Segment { segment } if segment == segment_id => None,
        BrowserStop::Segment { segment } => {
            return Err(WorkbenchError::new(format!(
                "selected segment {segment_id:?} cannot stop at {segment:?}"
            )));
        }
        BrowserStop::Tick { tick } => Some(*tick),
    };
    let artifact_root = configured_artifact_root(config)?;
    let materialized =
        materialize_segment_playback(timeline, &artifact_root, segment_id, local_frame)?;
    let fast_forward_frames =
        playback_fast_forward_frames(options.playback, materialized.tape.frames.len() as u64);
    let thumbnail = prepare_missing_playback_thumbnail(
        timeline,
        config,
        &BrowserSelection::Segment {
            id: segment_id.into(),
        },
    )?;
    launch_materialized(
        timeline,
        config,
        materialized,
        MaterializedLaunchOptions {
            takeover: options.handoff,
            origin: PlaybackOrigin::Boot,
            fast_forward_frames,
            thumbnail,
            playback: options.playback,
        },
    )
}

#[cfg(test)]
pub(super) fn segment_parent_frame_count(
    timeline: &Timeline,
    repository_root: &Path,
    parent_id: Option<&str>,
    full_tape: &InputTape,
    segment_id: &str,
) -> Result<u64, WorkbenchError> {
    let parent_id = parent_id.ok_or_else(|| {
        WorkbenchError::new(format!(
            "segment {segment_id:?} begins at Boot and has no parent playback boundary"
        ))
    })?;
    let parent = materialize_segment_chain(timeline, repository_root, parent_id)?.tape;
    let parent_frames = parent.frames.len();
    let continuation_frames = full_tape.frames.len().saturating_sub(parent_frames);
    validate_parent_boundary(
        parent_frames as u64,
        continuation_frames as u64,
        full_tape.frames.len() as u64,
    )
    .map_err(|_| {
        WorkbenchError::new(format!(
            "segment {segment_id:?} has no nonempty continuation after its parent"
        ))
    })?;
    if full_tape.tick_rate_numerator != parent.tick_rate_numerator
        || full_tape.tick_rate_denominator != parent.tick_rate_denominator
        || full_tape.frames[..parent_frames] != parent.frames
    {
        return Err(WorkbenchError::new(format!(
            "segment {segment_id:?} playback does not begin with its exact parent chain"
        )));
    }
    Ok(parent_frames as u64)
}

pub(super) fn materialize_draft(
    timeline: &Timeline,
    repository_root: &Path,
    state_root: &Path,
    draft_id: &str,
) -> Result<MaterializedPlayback, WorkbenchError> {
    enum DraftBase {
        Boot,
        Segment { id: String },
    }

    let manifests = scan_draft_manifests(state_root)?;
    let mut cursor = draft_id.to_owned();
    let mut seen = BTreeSet::new();
    let mut continuations = Vec::new();
    let base_segment = loop {
        if !seen.insert(cursor.clone()) {
            return Err(WorkbenchError::new("draft parent graph contains a cycle"));
        }
        let manifest = manifests
            .get(&cursor)
            .ok_or_else(|| WorkbenchError::new(format!("unknown draft {cursor:?}")))?;
        if manifest.status != DraftStatus::Ready {
            return Err(WorkbenchError::new(format!(
                "draft {cursor:?} is {:?}, not ready",
                manifest.status
            )));
        }
        continuations.push(manifest.clone());
        match &manifest.parent {
            DraftParent::Milestone {
                id,
                program_sha256,
                definition_sha256,
                boundary_fingerprint,
            } => {
                let program = origin_predicate_program_projection(timeline, repository_root)?
                    .ok_or_else(|| WorkbenchError::new("Boot parent has no predicate source"))?;
                let definition = program
                    .definitions
                    .iter()
                    .find(|definition| definition.name == *id)
                    .ok_or_else(|| WorkbenchError::new("Boot parent milestone is missing"))?;
                if program.program_sha256 != *program_sha256
                    || definition.definition_sha256 != *definition_sha256
                    || !is_exact_boot_boundary_predicate(definition)
                    || !manifest.start_boundary_verified
                    || !boundary_fingerprint
                        .as_deref()
                        .is_some_and(native_fingerprint)
                    || manifest.parent_tape_sha256 != tape_digest(&InputTape::default())?
                {
                    return Err(WorkbenchError::new("Boot parent proof is missing or stale"));
                }
                break DraftBase::Boot;
            }
            DraftParent::Segment {
                id,
                terminal_milestone: _,
                boundary_fingerprint,
            } => {
                let segment = timeline
                    .segments
                    .get(id)
                    .ok_or_else(|| WorkbenchError::new("draft parent segment is missing"))?;
                if segment.end_fingerprint != *boundary_fingerprint {
                    return Err(WorkbenchError::new("draft parent segment boundary changed"));
                }
                break DraftBase::Segment { id: id.clone() };
            }
            DraftParent::Draft { id, .. } => cursor = id.clone(),
        }
    };
    continuations.reverse();

    let (mut tape, seed_stage, base_label) = match base_segment {
        DraftBase::Boot => (InputTape::default(), None, "boot".to_owned()),
        DraftBase::Segment { id: base_segment } => {
            let base_tape = materialize_segment_chain(timeline, repository_root, &base_segment)?;
            let seed_stage = base_tape.steps.first().and_then(|step| {
                legacy_seed_stage(&base_tape.tape, timeline.segments[&step.segment].profile)
            });
            (base_tape.tape, seed_stage, base_segment)
        }
    };
    let root = drafts_root(state_root)?;
    for manifest in continuations {
        let digest = tape_digest(&tape)?;
        if digest != manifest.parent_tape_sha256 {
            return Err(WorkbenchError::new(format!(
                "draft {:?} parent tape fingerprint changed",
                manifest.id
            )));
        }
        if let DraftParent::Draft {
            parent_tape_sha256, ..
        } = &manifest.parent
            && *parent_tape_sha256 != digest
        {
            return Err(WorkbenchError::new(format!(
                "draft {:?} has inconsistent parent metadata",
                manifest.id
            )));
        }
        let (encoded, continuation) = read_draft_tape(&root.join(&manifest.id))?;
        if continuation.frames.is_empty()
            || manifest.tape_bytes != Some(encoded.len() as u64)
            || manifest.frames != Some(continuation.frames.len() as u64)
            || manifest.tape_sha256.as_deref()
                != Some(format!("{:x}", Sha256::digest(&encoded)).as_str())
        {
            return Err(WorkbenchError::new(format!(
                "draft {:?} continuation metadata is unverified",
                manifest.id
            )));
        }
        tape = concatenate(vec![
            ChainSegment::all(tape),
            ChainSegment::all(continuation),
        ])
        .map_err(|error| WorkbenchError::new(error.to_string()))?
        .tape;
        if manifest.result_tape_sha256.as_deref() != Some(tape_digest(&tape)?.as_str()) {
            return Err(WorkbenchError::new(format!(
                "draft {:?} finalized chain fingerprint changed",
                manifest.id
            )));
        }
    }
    Ok(MaterializedPlayback {
        lineage: None,
        segment: Some(format!("{base_label}:{draft_id}")),
        tape,
        seed_stage,
        native_oracle: NativePlaybackOracle::None,
    })
}

pub(super) fn play_target(request: &PlayRequest) -> Result<MaterializeTarget, WorkbenchError> {
    if let Some(segment) = &request.through_segment {
        return Ok(MaterializeTarget::ThroughSegment(segment.clone()));
    }
    match (&request.segment, request.frame) {
        (Some(segment), Some(frame)) => Ok(MaterializeTarget::ThroughSegmentFrame {
            segment: segment.clone(),
            frame,
        }),
        (None, None) => Ok(MaterializeTarget::FullLineage),
        _ => Err(WorkbenchError::new(
            "segment and frame must be supplied together",
        )),
    }
}

pub(super) fn validate_playback_origin(request: &BrowserPlayRequest) -> Result<(), WorkbenchError> {
    if request.mode == PlaybackMode::ResumeAccelerated && !request.handoff {
        return Err(WorkbenchError::new(
            "accelerated resume requires controller handoff at the selected endpoint",
        ));
    }
    Ok(())
}

pub(super) fn playback_fast_forward_frames(playback: PlaybackSettings, frames: u64) -> Option<u64> {
    playback.fast.then_some(frames)
}

#[cfg(test)]
mod native_fidelity_tests {
    use super::*;

    #[test]
    fn eye_shredder_oracle_supplies_trace_and_oracle_arguments() {
        let mut command = Command::new("dusklight");
        append_native_oracle_args(
            &mut command,
            Path::new("session"),
            NativePlaybackOracle::EyeShredder,
        );
        let arguments = command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            arguments,
            [
                "--automation-oracle",
                "eye-shredder",
                "--automation-oracle-continue-on-pass",
                "--automation-oracle-result",
                &Path::new("session")
                    .join("eye-shredder.oracle.json")
                    .to_string_lossy(),
                "--name-entry-trace",
                &Path::new("session")
                    .join("eye-shredder.name-entry.trace.json")
                    .to_string_lossy(),
            ]
        );
    }

    #[test]
    fn workbench_refuses_any_binary_without_default_console_fidelity() {
        let unsupported = serde_json::json!({
            "ok": true,
            "build": {
                "feature_switches": "automation_observers=OFF;automation_fidelity_models=OFF",
                "fidelity_profile": "cursor_breakout_shadow"
            }
        });
        assert!(validate_native_fidelity_identity(&unsupported).is_err());

        let supported = serde_json::json!({
            "ok": true,
            "build": {
                "feature_switches": "automation_observers=ON;automation_fidelity_models=ON",
                "fidelity_profile": "cursor_breakout_shadow"
            }
        });
        validate_native_fidelity_identity(&supported).unwrap();
    }
}
