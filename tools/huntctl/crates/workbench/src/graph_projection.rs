//! Timeline-to-browser graph projection and content-addressed thumbnails.

use super::*;

pub fn graph_from_timeline(
    timeline: &Timeline,
    repository_root: &Path,
) -> Result<WorkbenchGraph, WorkbenchError> {
    timeline
        .inspect()
        .map_err(|error| WorkbenchError::new(error.to_string()))?;
    let predicate_program = milestone_program_projection(timeline, repository_root)?;
    let predicate_digests = predicate_program
        .as_ref()
        .map(|program| {
            program
                .definitions
                .iter()
                .map(|definition| {
                    (
                        definition.name.as_str(),
                        definition.definition_sha256.as_str(),
                    )
                })
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    let mut boot_configurations = Vec::new();
    let mut boot_configuration_keys = BTreeSet::new();
    for segment in timeline
        .segments
        .values()
        .filter(|segment| segment.parent.is_none())
    {
        if let Ok(tape) = load_segment_tape(segment, repository_root) {
            let key = serde_json::to_string(&tape.boot).unwrap_or_default();
            if boot_configuration_keys.insert(key) {
                boot_configurations.push(tape.boot);
            }
        }
    }
    let origin = timeline.origin.as_ref().map(|origin| GraphOrigin {
        id: origin.id.clone(),
        predicate: origin.predicate.clone(),
        recordable_from_boot: predicate_program
            .as_ref()
            .and_then(|program| {
                program
                    .definitions
                    .iter()
                    .find(|definition| definition.name == origin.predicate)
            })
            .is_some_and(is_exact_boot_boundary_predicate),
        configurations: boot_configurations,
    });
    let goals = timeline
        .goals
        .values()
        .map(|goal| GraphGoal {
            id: goal.id.clone(),
            segment: goal.segment.clone(),
            predicate: goal.predicate.clone(),
        })
        .collect();

    let segments = timeline
        .segments
        .values()
        .map(|segment| {
            let loaded = load_segment_tape(segment, repository_root);
            let relevant_goals = timeline
                .goals
                .values()
                .filter(|goal| {
                    goal.segment == segment.id
                        || timeline
                            .proofs
                            .iter()
                            .any(|proof| proof.segment == segment.id && proof.goal == goal.id)
                })
                .collect::<Vec<_>>();
            let goal_proofs = relevant_goals
                .iter()
                .map(|goal| {
                    let proof = timeline
                        .proofs
                        .iter()
                        .find(|proof| proof.segment == segment.id && proof.goal == goal.id);
                    let status = match (predicate_program.as_ref(), proof) {
                        (None, _) => "not_required",
                        (Some(_), None) => "missing",
                        (Some(program), Some(proof))
                            if proof.predicate_program_sha256 == program.program_sha256
                                && predicate_digests.get(goal.predicate.as_str()).is_some_and(
                                    |digest| *digest == proof.predicate_definition_sha256,
                                ) =>
                        {
                            "verified"
                        }
                        (Some(_), Some(_)) => "stale",
                    };
                    GraphGoalProof {
                        goal: goal.id.clone(),
                        predicate: goal.predicate.clone(),
                        program_sha256: proof
                            .map(|proof| proof.predicate_program_sha256.clone())
                            .unwrap_or_default(),
                        definition_sha256: proof
                            .map(|proof| proof.predicate_definition_sha256.clone())
                            .unwrap_or_default(),
                        status: status.into(),
                        first_hit_tick: proof.and_then(|proof| proof.first_hit_tick),
                    }
                })
                .collect::<Vec<_>>();
            let predicate_proof = if goal_proofs.is_empty() {
                "not_required"
            } else if goal_proofs.iter().all(|proof| proof.status == "verified") {
                "verified"
            } else if goal_proofs.iter().any(|proof| proof.status == "stale") {
                "stale"
            } else {
                "missing"
            };
            let first_hit_tick = goal_proofs
                .iter()
                .filter(|proof| proof.status == "verified")
                .filter_map(|proof| proof.first_hit_tick)
                .min();
            let record_anchors = goal_proofs
                .iter()
                .filter(|proof| proof.status == "verified")
                .map(|proof| GraphRecordAnchor {
                    goal: proof.goal.clone(),
                    predicate: proof.predicate.clone(),
                })
                .collect::<Vec<_>>();
            let playable = loaded.is_ok()
                && artifact_is_canonical_payload(&segment.artifact)
                && fingerprints_are_exact(segment)
                && materialize_segment_chain(timeline, repository_root, &segment.id).is_ok();
            let (option_visualization, option_diagnostic_error) = loaded
                .as_ref()
                .ok()
                .map(|tape| load_option_visualization(segment, repository_root, tape))
                .unwrap_or_else(|| Ok(Vec::new()))
                .map(|visualization| (visualization, None))
                .unwrap_or_else(|error| (Vec::new(), Some(error.to_string())));
            GraphSegment {
                id: segment.id.clone(),
                name: segment.name.clone(),
                parent: segment.parent.clone(),
                profile: segment.profile.as_str().into(),
                artifact: graph_artifact(&segment.artifact),
                start_fingerprint: segment.start_fingerprint.clone(),
                boundary_fingerprint: segment.end_fingerprint.clone(),
                goal_proofs,
                predicate_proof: predicate_proof.into(),
                first_hit_tick,
                frame_count: loaded.as_ref().ok().map(|tape| tape.frames.len() as u64),
                start_tick: 0,
                end_tick: loaded
                    .as_ref()
                    .ok()
                    .and_then(|tape| (tape.frames.len() as u64).checked_sub(1)),
                ticks: first_hit_tick,
                playable,
                recordable: loaded.is_ok() && !record_anchors.is_empty(),
                record_anchors,
                option_visualization,
                option_diagnostic_error,
                generated: None,
                thumbnail: None,
                error: loaded.err().map(|error| error.to_string()),
            }
        })
        .collect();
    Ok(WorkbenchGraph {
        schema: GRAPH_SCHEMA.into(),
        timeline: timeline.name.clone(),
        origin,
        segments,
        goals,
        drafts: Vec::new(),
        projects: GraphProjectCatalog {
            schema: PROJECT_CATALOG_SCHEMA.into(),
            ..GraphProjectCatalog::default()
        },
        draft_graph_revision: None,
        predicate_program,
    })
}

pub(super) fn native_fingerprint(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

pub(super) fn graph_with_drafts(
    timeline: &Timeline,
    repository_root: &Path,
    state_root: &Path,
) -> Result<WorkbenchGraph, WorkbenchError> {
    let mut graph = graph_from_timeline(timeline, repository_root)?;
    let manifests = scan_draft_manifests(state_root)?;
    graph.draft_graph_revision = Some(draft_graph_revision(&manifests)?);
    graph.drafts = graph_drafts_from_manifests(timeline, repository_root, state_root, manifests)?;
    Ok(graph)
}

pub(super) fn bounded_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > MAX_SEARCH_ARTIFACT_BYTES
    {
        return None;
    }
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

pub(super) fn median_tick(ticks: &[u64]) -> Option<u64> {
    let mut ticks = ticks.to_vec();
    ticks.sort_unstable();
    ticks.get(ticks.len() / 2).copied()
}

pub(super) fn generated_search_projections(
    timeline: &Timeline,
    search_root: &Path,
) -> Vec<GeneratedProjection> {
    let Ok(root_metadata) = fs::symlink_metadata(search_root) else {
        return Vec::new();
    };
    if !root_metadata.is_dir() || root_metadata.file_type().is_symlink() {
        return Vec::new();
    }
    let Ok(canonical_search_root) = fs::canonicalize(search_root) else {
        return Vec::new();
    };
    let authored_boundaries = timeline
        .segments
        .values()
        .map(|segment| segment.end_fingerprint.as_str())
        .collect::<BTreeSet<_>>();
    let mut runs = fs::read_dir(search_root)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            if !metadata.is_dir() || entry.file_type().ok()?.is_symlink() {
                return None;
            }
            let modified = metadata.modified().ok()?;
            Some((modified, entry.path()))
        })
        .collect::<Vec<_>>();
    runs.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
    runs.truncate(MAX_SEARCH_RUNS);

    let mut projections = Vec::new();
    let mut seen_boundaries = BTreeSet::new();
    for (_, run) in runs {
        let mut completed_generations = fs::read_dir(&run)
            .ok()
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let name = entry.file_name();
                let generation = name.to_str()?.strip_prefix('g')?.parse::<u32>().ok()?;
                entry
                    .path()
                    .join("results.json")
                    .is_file()
                    .then_some((generation, entry.path()))
            })
            .collect::<Vec<_>>();
        completed_generations.sort_by_key(|item| std::cmp::Reverse(item.0));
        let Some((generation, generation_root)) = completed_generations.into_iter().next() else {
            continue;
        };
        let Some(results) =
            bounded_json::<GeneratedAnchoredResults>(&generation_root.join("results.json"))
        else {
            continue;
        };
        if results.schema != "dusklight-anchored-search-results/v2"
            || results.results.segment != results.objective.segment
            || results.objective.digest.len() != 64
            || !results
                .objective
                .digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            continue;
        }
        let objective = &results.objective;
        let Some(parent) = timeline
            .segments
            .values()
            .find(|segment| segment.end_fingerprint == objective.source_boundary_fingerprint)
        else {
            continue;
        };
        let goal = timeline.goals.values().find(|goal| {
            goal.predicate == objective.goal_milestone
                && (goal.segment == parent.id
                    || timeline.segments.get(&goal.segment).is_some_and(|segment| {
                        segment.parent.as_deref() == Some(parent.id.as_str())
                    }))
        });
        let display_base = goal
            .and_then(|goal| timeline.segments.get(&goal.segment))
            .and_then(|segment| segment.name.clone())
            .unwrap_or_else(|| objective.goal_milestone.replace('_', " "));
        let mut ranked = results
            .results
            .candidates
            .iter()
            .filter(|(_, result)| {
                (result.goal_reached == Some(true)
                    || (result.goal_reached.is_none() && result.milestone_depth == 2))
                    && result.attempts >= 2
                    && result.successes == result.attempts
                    && result.first_hit_ticks.len() == result.attempts as usize
                    && result
                        .first_hit_ticks
                        .windows(2)
                        .all(|pair| pair[0] == pair[1])
            })
            .map(|(id, result)| (id, result, median_tick(&result.first_hit_ticks).unwrap()))
            .collect::<Vec<_>>();
        ranked.sort_by(|left, right| left.2.cmp(&right.2).then_with(|| left.0.cmp(right.0)));
        ranked.truncate(GENERATED_SEGMENTS_PER_RUN);
        for (candidate_id, result, tick) in ranked {
            if candidate_id.len() != 64
                || !candidate_id.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                continue;
            }
            let candidate_path = generation_root.join(format!("{candidate_id}.candidate.json"));
            let suffix_path = generation_root.join(format!("{candidate_id}.tape"));
            let attempt_root = generation_root
                .join("evaluations")
                .join("candidates")
                .join(candidate_id);
            let mut attempts = Vec::new();
            for attempt in 1..=result.attempts {
                let path = attempt_root
                    .join(format!("attempt-{attempt:03}"))
                    .join("attempt.json");
                let Some(evidence) = bounded_json::<GeneratedAttempt>(&path) else {
                    attempts.clear();
                    break;
                };
                attempts.push(evidence);
            }
            let Some(first) = attempts.first() else {
                continue;
            };
            let Some(output_fingerprint) = first
                .boundary_fingerprints
                .get(&objective.goal_milestone)
                .map(|fingerprint| fingerprint.digest.clone())
            else {
                continue;
            };
            if authored_boundaries.contains(output_fingerprint.as_str())
                || !native_fingerprint(&output_fingerprint)
                || !seen_boundaries.insert((
                    objective.source_boundary_fingerprint.clone(),
                    output_fingerprint.clone(),
                ))
                || attempts.iter().any(|attempt| {
                    attempt.candidate_id != *candidate_id
                        || attempt.exit_code != Some(0)
                        || attempt.infrastructure_error.is_some()
                        || !attempt.goal_reached
                        || attempt.first_hit_tick != Some(tick)
                        || attempt
                            .boundary_fingerprints
                            .get(&objective.goal_milestone)
                            .is_none_or(|fingerprint| fingerprint.digest != output_fingerprint)
                })
            {
                continue;
            }
            let (Ok(candidate_path), Ok(suffix_path), Ok(full_tape)) = (
                fs::canonicalize(&candidate_path),
                fs::canonicalize(&suffix_path),
                fs::canonicalize(&first.tape),
            ) else {
                continue;
            };
            if !candidate_path.starts_with(&canonical_search_root)
                || !suffix_path.starts_with(&canonical_search_root)
                || !full_tape.starts_with(&canonical_search_root)
                || fs::symlink_metadata(&candidate_path)
                    .ok()
                    .is_none_or(|metadata| {
                        !metadata.is_file()
                            || metadata.file_type().is_symlink()
                            || metadata.len() > MAX_SEARCH_ARTIFACT_BYTES
                    })
                || fs::symlink_metadata(&suffix_path)
                    .ok()
                    .is_none_or(|metadata| {
                        !metadata.is_file()
                            || metadata.file_type().is_symlink()
                            || metadata.len() > MAX_SEARCH_ARTIFACT_BYTES
                    })
                || fs::symlink_metadata(&full_tape)
                    .ok()
                    .is_none_or(|metadata| {
                        !metadata.is_file()
                            || metadata.file_type().is_symlink()
                            || metadata.len() > MAX_SEARCH_ARTIFACT_BYTES
                    })
            {
                continue;
            }
            let Ok(suffix_bytes) = fs::read(&suffix_path) else {
                continue;
            };
            if suffix_bytes.len() as u64 > MAX_SEARCH_ARTIFACT_BYTES {
                continue;
            }
            let Ok(suffix) = InputTape::decode(&suffix_bytes) else {
                continue;
            };
            let short_objective = &objective.digest[..16];
            let short_candidate = &candidate_id[..16];
            let id = format!("search-{short_objective}-{short_candidate}");
            let goal_proofs = goal
                .map(|goal| {
                    vec![GraphGoalProof {
                        goal: goal.id.clone(),
                        predicate: goal.predicate.clone(),
                        program_sha256: objective.milestone_program_sha256.clone(),
                        definition_sha256: objective.goal_definition_sha256.clone(),
                        status: "verified".into(),
                        first_hit_tick: Some(tick),
                    }]
                })
                .unwrap_or_default();
            projections.push(GeneratedProjection {
                segment: GraphSegment {
                    id,
                    name: Some(format!("{display_base} · {tick}f · {}", &candidate_id[..6])),
                    parent: Some(parent.id.clone()),
                    profile: objective.segment.as_str().into(),
                    artifact: GraphArtifact {
                        kind: "tape".into(),
                        value: suffix_path.display().to_string(),
                    },
                    start_fingerprint: objective.source_boundary_fingerprint.clone(),
                    boundary_fingerprint: output_fingerprint,
                    goal_proofs,
                    predicate_proof: "verified".into(),
                    first_hit_tick: Some(tick),
                    frame_count: Some(suffix.tape.frames.len() as u64),
                    start_tick: 0,
                    end_tick: (suffix.tape.frames.len() as u64).checked_sub(1),
                    ticks: Some(tick),
                    playable: true,
                    recordable: false,
                    record_anchors: Vec::new(),
                    option_visualization: Vec::new(),
                    option_diagnostic_error: None,
                    generated: Some(GraphGeneratedSegment {
                        kind: "search_candidate".into(),
                        status: "proved".into(),
                        uncommitted: true,
                        run: run.display().to_string(),
                        generation,
                        candidate_id: candidate_id.clone(),
                        candidate: candidate_path.display().to_string(),
                        tape: suffix_path.display().to_string(),
                        objective_sha256: objective.digest.clone(),
                        source_predicate: objective.source_milestone.clone(),
                        goal_predicate: objective.goal_milestone.clone(),
                        proof_attempts: result.attempts,
                    }),
                    thumbnail: None,
                    error: None,
                },
                full_tape,
            });
            if projections.len() >= MAX_GENERATED_SEGMENTS {
                return projections;
            }
        }
    }
    projections
}

pub(super) fn generated_search_tombstone_path(state_root: &Path) -> PathBuf {
    state_root.join(GENERATED_SEARCH_TOMBSTONES)
}

pub(super) fn load_generated_search_tombstones(
    state_root: &Path,
) -> Result<GeneratedSearchTombstones, WorkbenchError> {
    let path = generated_search_tombstone_path(state_root);
    let Ok(metadata) = fs::symlink_metadata(&path) else {
        return Ok(GeneratedSearchTombstones {
            schema: GENERATED_SEARCH_TOMBSTONE_SCHEMA.into(),
            candidate_ids: BTreeSet::new(),
        });
    };
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() > MAX_SEARCH_ARTIFACT_BYTES
    {
        return Err(WorkbenchError::new(
            "generated search tombstones are not a bounded physical file",
        ));
    }
    let tombstones: GeneratedSearchTombstones =
        serde_json::from_slice(&fs::read(&path).map_err(|error| {
            WorkbenchError::new(format!("cannot read generated search tombstones: {error}"))
        })?)
        .map_err(|error| WorkbenchError::new(format!("invalid search tombstones: {error}")))?;
    if tombstones.schema != GENERATED_SEARCH_TOMBSTONE_SCHEMA
        || tombstones.candidate_ids.iter().any(|id| {
            id.len() != 64
                || !id
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        })
    {
        return Err(WorkbenchError::new(
            "generated search tombstones have an invalid schema or candidate ID",
        ));
    }
    Ok(tombstones)
}

pub(super) fn visible_generated_search_projections(
    timeline: &Timeline,
    search_root: &Path,
    state_root: &Path,
) -> Result<Vec<GeneratedProjection>, WorkbenchError> {
    let hidden = load_generated_search_tombstones(state_root)?.candidate_ids;
    Ok(generated_search_projections(timeline, search_root)
        .into_iter()
        .filter(|projection| {
            projection
                .segment
                .generated
                .as_ref()
                .is_none_or(|generated| !hidden.contains(&generated.candidate_id))
        })
        .collect())
}

pub(super) fn append_generated_search_segments(
    graph: &mut WorkbenchGraph,
    timeline: &Timeline,
    search_root: &Path,
    state_root: &Path,
) -> Result<(), WorkbenchError> {
    graph.segments.extend(
        visible_generated_search_projections(timeline, search_root, state_root)?
            .into_iter()
            .map(|projection| projection.segment),
    );
    Ok(())
}

pub(super) fn thumbnail_key(kind: &str, materialization: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(b"dusklight.route-workbench.thumbnail.v3-4x3\0");
    for value in [kind, materialization] {
        digest.update((value.len() as u64).to_le_bytes());
        digest.update(value.as_bytes());
    }
    format!("{:x}", digest.finalize())
}

pub(super) fn graph_node_thumbnail_key(
    graph: &WorkbenchGraph,
    selection: &BrowserSelection,
) -> Result<String, WorkbenchError> {
    match selection {
        BrowserSelection::Segment { id } => {
            let segment = graph
                .segments
                .iter()
                .find(|segment| segment.id == *id)
                .ok_or_else(|| WorkbenchError::new(format!("unknown segment {id:?}")))?;
            if !segment.playable {
                return Err(WorkbenchError::new(format!(
                    "segment {id:?} is not playable"
                )));
            }
            Ok(thumbnail_key("segment", &segment.boundary_fingerprint))
        }
        BrowserSelection::Draft { id } => {
            let draft = graph
                .drafts
                .iter()
                .find(|draft| draft.id == *id)
                .ok_or_else(|| WorkbenchError::new(format!("unknown draft {id:?}")))?;
            if !draft.playable {
                return Err(WorkbenchError::new(format!("draft {id:?} is not playable")));
            }
            let identity = draft.result_tape_sha256.as_deref().ok_or_else(|| {
                WorkbenchError::new(format!("draft {id:?} has no finalized chain fingerprint"))
            })?;
            Ok(thumbnail_key("draft", identity))
        }
        BrowserSelection::Project { id } => {
            let project = graph
                .projects
                .entries
                .iter()
                .find(|project| project.id == *id)
                .ok_or_else(|| WorkbenchError::new(format!("unknown workspace tape {id:?}")))?;
            if !project.playable || project.kind == "timeline" {
                return Err(WorkbenchError::new(format!(
                    "workspace tape {id:?} is not playable"
                )));
            }
            let identity = project.materialization_sha256.as_deref().ok_or_else(|| {
                WorkbenchError::new(format!(
                    "workspace tape {id:?} has no materialization identity"
                ))
            })?;
            Ok(thumbnail_key("project", identity))
        }
    }
}

pub(super) fn thumbnail_url(key: &str) -> String {
    format!("/api/thumbnails/{key}.png")
}

pub(super) fn thumbnail_cache_path(state_root: &Path, key: &str) -> PathBuf {
    state_root
        .join(THUMBNAIL_DIRECTORY)
        .join(format!("{key}.png"))
}

pub(super) fn thumbnail_file_is_valid(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() || metadata.len() < 8 || metadata.len() > MAX_THUMBNAIL_BYTES {
        return false;
    }
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    let mut signature = [0_u8; 8];
    file.read_exact(&mut signature).is_ok() && signature == *b"\x89PNG\r\n\x1a\n"
}

pub(super) fn reachable_thumbnail_keys(graph: &WorkbenchGraph) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    for segment in &graph.segments {
        keys.insert(thumbnail_key("segment", &segment.boundary_fingerprint));
    }
    for draft in &graph.drafts {
        if let Some(identity) = draft.result_tape_sha256.as_deref() {
            keys.insert(thumbnail_key("draft", identity));
        }
    }
    for project in &graph.projects.entries {
        if project.kind != "timeline"
            && let Some(identity) = project.materialization_sha256.as_deref()
        {
            keys.insert(thumbnail_key("project", identity));
        }
    }
    keys
}

#[derive(Clone, Debug, Serialize)]
pub struct ThumbnailPruneEntry {
    pub key: String,
    pub source: PathBuf,
    pub size: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct ThumbnailPruneReport {
    pub schema: &'static str,
    pub dry_run: bool,
    pub reachable: usize,
    pub orphaned: Vec<ThumbnailPruneEntry>,
    pub moved: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trash_transaction: Option<PathBuf>,
}

pub(super) fn prune_orphaned_thumbnails(
    graph: &WorkbenchGraph,
    state_root: &Path,
    apply: bool,
) -> Result<ThumbnailPruneReport, WorkbenchError> {
    let thumbnail_root = state_root.join(THUMBNAIL_DIRECTORY);
    let entries = match fs::read_dir(&thumbnail_root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ThumbnailPruneReport {
                schema: THUMBNAIL_PRUNE_SCHEMA,
                dry_run: !apply,
                reachable: 0,
                orphaned: Vec::new(),
                moved: 0,
                trash_transaction: None,
            });
        }
        Err(error) => {
            return Err(WorkbenchError::new(format!(
                "cannot inspect thumbnail cache {}: {error}",
                thumbnail_root.display()
            )));
        }
    };
    let reachable = reachable_thumbnail_keys(graph);
    let mut orphaned = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            WorkbenchError::new(format!(
                "cannot inspect thumbnail cache {}: {error}",
                thumbnail_root.display()
            ))
        })?;
        let file_type = entry.file_type().map_err(|error| {
            WorkbenchError::new(format!(
                "cannot inspect thumbnail cache entry {}: {error}",
                entry.path().display()
            ))
        })?;
        if !file_type.is_file() {
            continue;
        }
        let Some(filename) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Some(key) = filename.strip_suffix(".png") else {
            continue;
        };
        if !valid_sha256(key) || reachable.contains(key) {
            continue;
        }
        orphaned.push(ThumbnailPruneEntry {
            key: key.into(),
            source: entry.path(),
            size: entry
                .metadata()
                .map_err(|error| {
                    WorkbenchError::new(format!(
                        "cannot inspect orphaned thumbnail {}: {error}",
                        entry.path().display()
                    ))
                })?
                .len(),
        });
    }
    orphaned.sort_by(|left, right| left.key.cmp(&right.key));
    let trash_transaction = if apply && !orphaned.is_empty() {
        let state_root = fs::canonicalize(state_root).map_err(|error| {
            WorkbenchError::new(format!("cannot resolve workbench state root: {error}"))
        })?;
        let trash_root = state_root.join(DRAFT_TRASH_DIRECTORY).join("thumbnails");
        fs::create_dir_all(&trash_root).map_err(|error| {
            WorkbenchError::new(format!("cannot create thumbnail trash: {error}"))
        })?;
        let trash_root = fs::canonicalize(&trash_root).map_err(|error| {
            WorkbenchError::new(format!("cannot resolve thumbnail trash: {error}"))
        })?;
        if !trash_root.starts_with(&state_root) || trash_root == state_root {
            return Err(WorkbenchError::new(
                "thumbnail trash escapes the workbench state root",
            ));
        }
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| WorkbenchError::new(error.to_string()))?
            .as_nanos();
        let transaction = trash_root.join(format!("prune-{}-{nonce}", std::process::id()));
        fs::create_dir(&transaction).map_err(|error| {
            WorkbenchError::new(format!(
                "cannot create thumbnail trash transaction: {error}"
            ))
        })?;
        let mut moved: Vec<&ThumbnailPruneEntry> = Vec::new();
        for entry in &orphaned {
            let destination = transaction.join(format!("{}.png", entry.key));
            if let Err(error) = fs::rename(&entry.source, &destination) {
                for prior in moved.iter().rev() {
                    let _ = fs::rename(
                        transaction.join(format!("{}.png", prior.key)),
                        &prior.source,
                    );
                }
                let _ = fs::remove_dir(&transaction);
                return Err(WorkbenchError::new(format!(
                    "cannot move orphaned thumbnail {} to recoverable trash: {error}",
                    entry.source.display()
                )));
            }
            moved.push(entry);
        }
        Some(transaction)
    } else {
        None
    };
    Ok(ThumbnailPruneReport {
        schema: THUMBNAIL_PRUNE_SCHEMA,
        dry_run: !apply,
        reachable: reachable.len(),
        moved: trash_transaction.as_ref().map_or(0, |_| orphaned.len()),
        orphaned,
        trash_transaction,
    })
}

/// Preview or apply cache pruning against the same graph projection used by
/// Route Workbench. Applying moves files into recoverable state-root trash.
pub fn prune_thumbnails(
    timeline: &Timeline,
    timeline_path: &Path,
    repository_root: &Path,
    state_root: &Path,
    apply: bool,
) -> Result<ThumbnailPruneReport, WorkbenchError> {
    let repository_root = fs::canonicalize(repository_root)
        .map_err(|error| WorkbenchError::new(format!("cannot resolve repository root: {error}")))?;
    let timeline_path = fs::canonicalize(timeline_path)
        .map_err(|error| WorkbenchError::new(format!("cannot resolve timeline: {error}")))?;
    if !timeline_path.starts_with(&repository_root) {
        return Err(WorkbenchError::new(
            "timeline is outside the repository root",
        ));
    }
    let artifact_root = timeline_path
        .parent()
        .ok_or_else(|| WorkbenchError::new("timeline has no artifact root"))?;
    let mut graph = graph_with_drafts(timeline, artifact_root, state_root)?;
    graph.projects = project_catalog_projection(&repository_root, &timeline_path)?;
    append_generated_search_segments(
        &mut graph,
        timeline,
        &repository_root.join("build/search"),
        state_root,
    )?;
    prune_orphaned_thumbnails(&graph, state_root, apply)
}

pub(super) fn decorate_graph_thumbnails(
    graph: &mut WorkbenchGraph,
    config: &WorkbenchConfig,
) -> Result<(), WorkbenchError> {
    for segment in &mut graph.segments {
        let key = thumbnail_key("segment", &segment.boundary_fingerprint);
        let path = thumbnail_cache_path(&config.state_root, &key);
        if thumbnail_file_is_valid(&path) {
            content_address_thumbnail(config, &path)?;
            segment.thumbnail = Some(thumbnail_url(&key));
        }
    }
    for draft in &mut graph.drafts {
        let Some(identity) = draft.result_tape_sha256.as_deref() else {
            continue;
        };
        let key = thumbnail_key("draft", identity);
        let path = thumbnail_cache_path(&config.state_root, &key);
        if thumbnail_file_is_valid(&path) {
            content_address_thumbnail(config, &path)?;
            draft.thumbnail = Some(thumbnail_url(&key));
        }
    }
    for project in &mut graph.projects.entries {
        let Some(identity) = project.materialization_sha256.as_deref() else {
            continue;
        };
        let key = thumbnail_key("project", identity);
        let path = thumbnail_cache_path(&config.state_root, &key);
        if thumbnail_file_is_valid(&path) {
            content_address_thumbnail(config, &path)?;
            project.thumbnail = Some(thumbnail_url(&key));
        }
    }
    Ok(())
}

pub(super) fn content_address_thumbnail(
    config: &WorkbenchConfig,
    path: &Path,
) -> Result<(), WorkbenchError> {
    ContentStore::initialize(config.state_root.join("content"))
        .and_then(|store| store.put_file(path, ContentKind::Screenshot))
        .map(|_| ())
        .map_err(|error| WorkbenchError::new(format!("cannot content-address thumbnail: {error}")))
}

pub(super) fn prepare_missing_playback_thumbnail(
    timeline: &Timeline,
    config: &WorkbenchConfig,
    selection: &BrowserSelection,
) -> Result<Option<PlaybackThumbnailCapture>, WorkbenchError> {
    let artifact_root = configured_artifact_root(config)?;
    let mut graph = graph_with_drafts(timeline, &artifact_root, &config.state_root)?;
    graph.projects = project_catalog_projection(&config.repository_root, &config.timeline_path)?;
    let key = graph_node_thumbnail_key(&graph, selection)?;
    let path = thumbnail_cache_path(&config.state_root, &key);
    if thumbnail_file_is_valid(&path) {
        return Ok(None);
    }
    fs::create_dir_all(config.state_root.join(THUMBNAIL_DIRECTORY)).map_err(|error| {
        WorkbenchError::new(format!("cannot create playback thumbnail cache: {error}"))
    })?;
    if path.exists() {
        fs::remove_file(&path).map_err(|error| {
            WorkbenchError::new(format!(
                "cannot remove invalid playback thumbnail {}: {error}",
                path.display()
            ))
        })?;
    }
    Ok(Some(PlaybackThumbnailCapture {
        path,
        url: thumbnail_url(&key),
    }))
}

pub(super) fn install_recording_thumbnail(
    directory: &Path,
    manifest: &DraftManifest,
    config: &WorkbenchConfig,
) -> Result<(), WorkbenchError> {
    let source = directory.join(DRAFT_TERMINAL_THUMBNAIL);
    if manifest.status != DraftStatus::Ready {
        let _ = fs::remove_file(source);
        return Ok(());
    }
    if !source.exists() {
        return Ok(());
    }
    if !thumbnail_file_is_valid(&source) {
        let _ = fs::remove_file(source);
        return Err(WorkbenchError::new(
            "native recording terminal thumbnail is invalid",
        ));
    }
    let identity = manifest.result_tape_sha256.as_deref().ok_or_else(|| {
        WorkbenchError::new("ready recording lacks a finalized chain fingerprint")
    })?;
    let key = thumbnail_key("draft", identity);
    let thumbnail_root = config.state_root.join(THUMBNAIL_DIRECTORY);
    fs::create_dir_all(&thumbnail_root).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot create recording thumbnail cache {}: {error}",
            thumbnail_root.display()
        ))
    })?;
    let destination = thumbnail_cache_path(&config.state_root, &key);
    if thumbnail_file_is_valid(&destination) {
        content_address_thumbnail(config, &destination)?;
        fs::remove_file(&source).map_err(|error| {
            WorkbenchError::new(format!(
                "cannot remove duplicate recording thumbnail {}: {error}",
                source.display()
            ))
        })?;
        return Ok(());
    }
    if destination.exists() {
        fs::remove_file(&destination).map_err(|error| {
            WorkbenchError::new(format!(
                "cannot replace invalid recording thumbnail {}: {error}",
                destination.display()
            ))
        })?;
    }
    fs::rename(&source, &destination).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot install recording thumbnail {}: {error}",
            destination.display()
        ))
    })?;
    content_address_thumbnail(config, &destination)
}
