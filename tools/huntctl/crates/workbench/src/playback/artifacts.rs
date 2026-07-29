use super::*;

pub(crate) fn capture_tape_metadata(
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
pub(crate) fn graph_artifact(source: &ArtifactSource) -> GraphArtifact {
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

pub(crate) fn selected_step_count(
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

pub(crate) fn unique_segment_step(
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

pub(crate) fn ensure_composable_lineage(
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

pub(crate) fn ensure_canonical_prefix(
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

pub(crate) fn artifact_is_canonical_payload(source: &ArtifactSource) -> bool {
    // `uses tape` is the current DSL's explicit compact, immutable payload.
    // Baseline and candidate sources are profile-seeded evaluation programs.
    matches!(source, ArtifactSource::Tas(_) | ArtifactSource::Tape(_))
}

pub(crate) fn fingerprints_are_exact(segment: &Segment) -> bool {
    !contains_placeholder(&segment.start_fingerprint)
        && !contains_placeholder(&segment.end_fingerprint)
}

pub(crate) fn contains_placeholder(value: &str) -> bool {
    value.trim().is_empty() || value.to_ascii_lowercase().contains("unknown")
}

pub(crate) fn logical_last_frame(
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

pub(crate) fn option_diagnostic_relative_path(source: &ArtifactSource) -> Option<PathBuf> {
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

pub(crate) fn load_option_visualization(
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

pub(crate) fn load_segment_tape(
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

pub(crate) fn checked_artifact_path(
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

pub(crate) fn canonical_file(path: &Path, label: &str) -> Result<PathBuf, WorkbenchError> {
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

pub(crate) fn validate_play_request(request: &PlayRequest) -> Result<(), WorkbenchError> {
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

pub(crate) fn materialize_play_request(
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

pub(crate) fn materialize_segment_playback(
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

pub(crate) fn play_segment(
    timeline: &Timeline,
    config: &WorkbenchConfig,
    segment_id: &str,
    stop: &BrowserStop,
    options: SegmentPlaybackOptions,
) -> Result<(PlayResponse, Child), WorkbenchError> {
    if !timeline.segments.contains_key(segment_id) {
        if !matches!(stop, BrowserStop::Segment { segment } if segment == segment_id) {
            return Err(WorkbenchError::new(
                "generated candidate playback only supports its evaluated endpoint",
            ));
        }
        if let Some(projection) =
            optimization_candidate_projections(&config.repository_root, &config.timeline_path)?
                .into_iter()
                .find(|projection| projection.segment.id == segment_id)
        {
            let materialized = MaterializedPlayback {
                lineage: None,
                segment: Some(segment_id.into()),
                tape: projection.full_tape,
                seed_stage: None,
                native_oracle: NativePlaybackOracle::None,
            };
            let fast_forward_frames = playback_fast_forward_frames(
                options.playback,
                materialized.tape.frames.len() as u64,
            );
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
        let projection = visible_generated_search_projections(
            timeline,
            &config.repository_root.join("build/search"),
            &config.state_root,
        )?
        .into_iter()
        .find(|projection| projection.segment.id == segment_id)
        .ok_or_else(|| {
            WorkbenchError::new(format!(
                "unknown, deleted, or expired generated candidate {segment_id:?}"
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
