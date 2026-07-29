use super::*;
use dusklight_automation_contracts::native_fidelity::FIXED_AUTOMATION_CVARS;

mod artifacts;
mod native;
pub(crate) use artifacts::*;
pub(crate) use native::*;

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
    if !materialized.segment.as_deref().is_some_and(|segment| {
        segment.starts_with("project:") || segment.starts_with("tactic-route:")
    }) && let Some(configuration) =
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
    append_origin_card_fixture_arg(
        timeline,
        &config.repository_root,
        &materialized.tape.boot,
        &mut command,
    )?;
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
    append_generated_search_segments(
        &mut graph,
        timeline,
        &config.repository_root.join("build/search"),
        &config.state_root,
    )?;
    append_optimization_campaigns(
        &mut graph,
        &config.repository_root,
        &config.timeline_path,
        Some(config),
    )?;
    let key = graph_node_thumbnail_key(&graph, &request.selection)?;
    let materialized = match &request.selection {
        BrowserSelection::Segment { id } if timeline.segments.contains_key(id) => {
            materialize_segment_playback(timeline, &artifact_root, id, None)?
        }
        BrowserSelection::Segment { id } => {
            if let Some(projection) =
                optimization_candidate_projections(&config.repository_root, &config.timeline_path)?
                    .into_iter()
                    .find(|projection| projection.segment.id == *id)
            {
                MaterializedPlayback {
                    lineage: None,
                    segment: Some(id.clone()),
                    tape: projection.full_tape,
                    seed_stage: None,
                    native_oracle: NativePlaybackOracle::None,
                }
            } else {
                let projection = visible_generated_search_projections(
                    timeline,
                    &config.repository_root.join("build/search"),
                    &config.state_root,
                )?
                .into_iter()
                .find(|projection| projection.segment.id == *id)
                .ok_or_else(|| {
                    WorkbenchError::new(format!(
                        "unknown, deleted, or expired generated candidate {id:?}"
                    ))
                })?;
                let bytes = fs::read(&projection.full_tape).map_err(|error| {
                    WorkbenchError::new(format!(
                        "cannot read generated candidate tape {}: {error}",
                        projection.full_tape.display()
                    ))
                })?;
                MaterializedPlayback {
                    lineage: None,
                    segment: Some(id.clone()),
                    tape: InputTape::decode(&bytes)
                        .map_err(|error| {
                            WorkbenchError::new(format!(
                                "invalid generated candidate tape: {error}"
                            ))
                        })?
                        .tape,
                    seed_stage: None,
                    native_oracle: NativePlaybackOracle::None,
                }
            }
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
    append_origin_card_fixture_arg(
        timeline,
        &config.repository_root,
        &materialized.tape.boot,
        &mut command,
    )?;
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
            if combined.version.major != program.version.major {
                return Err(WorkbenchError::new(
                    "owned predicate sources use incompatible language versions",
                ));
            }
            combined.version.minor = combined.version.minor.max(program.version.minor);
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
        .arg(&renderer_cache_root);
    for cvar in FIXED_AUTOMATION_CVARS {
        command.arg("--cvar").arg(cvar);
    }
    append_fixed_step_pacing(command, options.playback.speed_percent);
    if let Some(stage) = options.seed_stage {
        command.arg("--stage").arg(stage);
    }
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
    fn process_boot_launches_bind_the_declared_repository_card_fixture() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let repository = std::env::temp_dir().join(format!(
            "dusklight-workbench-card-fixture-{}-{nonce}",
            std::process::id()
        ));
        let fixture = repository.join("orig/process-boot");
        fs::create_dir_all(&fixture).unwrap();
        let timeline = Timeline::parse(
            "timeline test\norigin boot predicate process_boot source predicate.milestones card_fixture orig/process-boot\n",
        )
        .unwrap();

        let mut process = Command::new("dusklight");
        append_origin_card_fixture_arg(&timeline, &repository, &TapeBoot::Process, &mut process)
            .unwrap();
        let arguments = process
            .get_args()
            .map(std::ffi::OsStr::to_os_string)
            .collect::<Vec<_>>();
        assert_eq!(
            arguments,
            [
                std::ffi::OsString::from("--automation-card-fixture"),
                fs::canonicalize(&fixture).unwrap().into_os_string(),
            ]
        );

        let mut stage = Command::new("dusklight");
        append_origin_card_fixture_arg(
            &timeline,
            &repository,
            &TapeBoot::Stage {
                stage: "F_SP103".into(),
                room: 0,
                point: 0,
                layer: -1,
                save_slot: None,
                fixture: None,
            },
            &mut stage,
        )
        .unwrap();
        assert_eq!(stage.get_args().count(), 0);

        fs::remove_dir_all(repository).unwrap();
    }

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
