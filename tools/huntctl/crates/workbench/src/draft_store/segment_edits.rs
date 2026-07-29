use super::*;

#[derive(Debug)]
pub(crate) struct SegmentSourceDeletion {
    pub(crate) segments: BTreeSet<String>,
    pub(crate) goals: BTreeSet<String>,
    pub(crate) proofs: usize,
    pub(crate) lineages: BTreeSet<String>,
    pub(crate) replacement: String,
}

pub(crate) fn segment_descendants_from_roots<'a>(
    timeline: &Timeline,
    roots: impl IntoIterator<Item = &'a str>,
) -> Result<BTreeSet<String>, WorkbenchError> {
    let mut children = BTreeMap::<&str, Vec<&str>>::new();
    for segment in timeline.segments.values() {
        if let Some(parent) = segment.parent.as_deref() {
            children.entry(parent).or_default().push(&segment.id);
        }
    }
    let mut deletion = BTreeSet::new();
    let mut pending = roots
        .into_iter()
        .map(|root| {
            if timeline.segments.contains_key(root) {
                Ok(root)
            } else {
                Err(WorkbenchError::new(format!("unknown segment {root:?}")))
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    if pending.is_empty() {
        return Err(WorkbenchError::new("segment deletion has no roots"));
    }
    while let Some(next) = pending.pop() {
        if !deletion.insert(next.to_owned()) {
            continue;
        }
        if let Some(descendants) = children.get(next) {
            pending.extend(descendants.iter().copied());
        }
    }
    Ok(deletion)
}

#[cfg(test)]
pub(crate) fn delete_segment_subtree_in_timeline_source(
    source: &str,
    id: &str,
) -> Result<SegmentSourceDeletion, WorkbenchError> {
    delete_segment_subtrees_in_timeline_source(source, [id])
}

#[cfg(test)]
pub(crate) fn delete_segment_subtrees_in_timeline_source<'a>(
    source: &str,
    roots: impl IntoIterator<Item = &'a str>,
) -> Result<SegmentSourceDeletion, WorkbenchError> {
    delete_segment_subtrees_in_timeline_source_preferring(source, roots, None)
}

pub(crate) fn delete_segment_subtrees_in_timeline_source_preferring<'a>(
    source: &str,
    roots: impl IntoIterator<Item = &'a str>,
    preferred_goal_anchor: Option<&str>,
) -> Result<SegmentSourceDeletion, WorkbenchError> {
    let timeline =
        Timeline::parse(source).map_err(|error| WorkbenchError::new(error.to_string()))?;
    let segments = segment_descendants_from_roots(&timeline, roots)?;
    let mut reanchored_goals = BTreeMap::<String, String>::new();
    let mut goals = BTreeSet::new();
    for goal in timeline
        .goals
        .values()
        .filter(|goal| segments.contains(&goal.segment))
    {
        let reference = &timeline.segments[&goal.segment];
        let mut compatible = timeline
            .proofs
            .iter()
            .filter(|proof| proof.goal == goal.id && !segments.contains(&proof.segment))
            .filter_map(|proof| timeline.segments.get(&proof.segment))
            .filter(|candidate| {
                candidate.parent == reference.parent
                    && candidate.profile == reference.profile
                    && candidate.start_fingerprint == reference.start_fingerprint
            })
            .map(|candidate| candidate.id.clone())
            .collect::<BTreeSet<_>>();
        let anchor = preferred_goal_anchor
            .filter(|preferred| compatible.contains(*preferred))
            .map(str::to_owned)
            .or_else(|| compatible.pop_first());
        if let Some(anchor) = anchor {
            reanchored_goals.insert(goal.id.clone(), anchor);
        } else {
            goals.insert(goal.id.clone());
        }
    }
    let proofs = timeline
        .proofs
        .iter()
        .filter(|proof| segments.contains(&proof.segment) || goals.contains(&proof.goal))
        .count();
    let removed_subgraphs = timeline
        .subgraphs
        .values()
        .filter(|subgraph| {
            timeline
                .subgraph_segment_closure(&subgraph.id)
                .iter()
                .any(|segment| segments.contains(segment))
        })
        .map(|subgraph| subgraph.id.clone())
        .collect::<BTreeSet<_>>();
    let surviving_subgraph_parent = |id: &str| {
        let mut parent = timeline.subgraphs[id].parent.as_deref();
        while parent.is_some_and(|candidate| removed_subgraphs.contains(candidate)) {
            parent = parent.and_then(|candidate| timeline.subgraphs[candidate].parent.as_deref());
        }
        parent.map(str::to_owned)
    };

    let mut reanchored_steps = BTreeMap::<(String, String), String>::new();
    if let Some(preferred_id) = preferred_goal_anchor
        && let Some(preferred) = timeline.segments.get(preferred_id)
        && !segments.contains(preferred_id)
    {
        let mut collect = |lineage: &str, steps: &[crate::timeline::ContinuationStep]| {
            if steps.iter().any(|step| step.segment == preferred_id) {
                return;
            }
            for step in steps.iter().filter(|step| segments.contains(&step.segment)) {
                let removed = &timeline.segments[&step.segment];
                let expected_parent = preferred.parent.as_deref().unwrap_or("root");
                if removed.parent == preferred.parent
                    && removed.profile == preferred.profile
                    && removed.start_fingerprint == preferred.start_fingerprint
                    && step.after.parent_segment == expected_parent
                    && step.after.checkpoint_fingerprint == preferred.start_fingerprint
                {
                    reanchored_steps.insert(
                        (lineage.to_owned(), step.segment.clone()),
                        preferred_id.to_owned(),
                    );
                }
            }
        };
        for continuation in timeline.continuations.values() {
            collect(&continuation.name, &continuation.steps);
        }
        for branch in timeline.branches.values() {
            collect(&branch.name, &branch.steps);
        }
    }

    let mut lineages = timeline
        .continuations
        .values()
        .filter(|continuation| {
            !continuation.steps.is_empty()
                && continuation.steps.iter().all(|step| {
                    segments.contains(&step.segment)
                        && !reanchored_steps
                            .contains_key(&(continuation.name.clone(), step.segment.clone()))
                })
        })
        .map(|continuation| continuation.name.clone())
        .collect::<BTreeSet<_>>();
    loop {
        let mut changed = false;
        for branch in timeline.branches.values() {
            if lineages.contains(&branch.name) {
                continue;
            }
            let all_steps_removed = !branch.steps.is_empty()
                && branch.steps.iter().all(|step| {
                    segments.contains(&step.segment)
                        && !reanchored_steps
                            .contains_key(&(branch.name.clone(), step.segment.clone()))
                });
            if segments.contains(&branch.after_segment)
                || lineages.contains(&branch.from_lineage)
                || all_steps_removed
            {
                changed |= lineages.insert(branch.name.clone());
            }
        }
        if !changed {
            break;
        }
    }

    let mut replacement = String::with_capacity(source.len());
    for (index, line) in source.split_inclusive('\n').enumerate() {
        let raw = line.trim_end_matches(['\r', '\n']);
        let tokens =
            tokenize(raw, index + 1).map_err(|error| WorkbenchError::new(error.to_string()))?;
        if tokens.first().map(String::as_str) == Some("goal")
            && let Some(goal_id) = tokens.get(1)
            && let Some(anchor) = reanchored_goals.get(goal_id)
        {
            replacement.push_str(&format!(
                "goal {} on {} predicate {}{}",
                goal_id,
                anchor,
                tokens
                    .get(5)
                    .expect("parsed goal declaration has a predicate"),
                timeline_line_ending(line)
            ));
            continue;
        }
        if tokens.first().map(String::as_str) == Some("continue")
            && let (Some(lineage), Some(segment), Some(pin)) =
                (tokens.get(1), tokens.get(3), tokens.get(5))
            && let Some(anchor) = reanchored_steps.get(&(lineage.clone(), segment.clone()))
        {
            replacement.push_str(&format!(
                "continue {lineage} with {anchor} after {pin}{}",
                timeline_line_ending(line)
            ));
            continue;
        }
        if tokens.first().map(String::as_str) == Some("subgraph")
            && let Some(id) = tokens.get(1)
            && !removed_subgraphs.contains(id)
            && let Some(subgraph) = timeline.subgraphs.get(id)
            && subgraph.parent != surviving_subgraph_parent(id)
        {
            match surviving_subgraph_parent(id) {
                Some(parent) => replacement.push_str(&format!(
                    "subgraph {id} inside {parent} entry {} exit {}{}",
                    subgraph.entry_segment,
                    subgraph.exit_segment,
                    timeline_line_ending(line)
                )),
                None => replacement.push_str(&format!(
                    "subgraph {id} root entry {} exit {}{}",
                    subgraph.entry_segment,
                    subgraph.exit_segment,
                    timeline_line_ending(line)
                )),
            }
            continue;
        }
        let remove = match tokens.first().map(String::as_str) {
            Some("segment") | Some("label") => tokens
                .get(1)
                .is_some_and(|segment| segments.contains(segment)),
            Some("goal") => {
                tokens.get(1).is_some_and(|goal| goals.contains(goal))
                    || tokens
                        .get(3)
                        .is_some_and(|segment| segments.contains(segment))
            }
            Some("proof") => {
                tokens
                    .get(1)
                    .is_some_and(|segment| segments.contains(segment))
                    || tokens.get(3).is_some_and(|goal| goals.contains(goal))
            }
            Some("continuation") | Some("branch") => tokens
                .get(1)
                .is_some_and(|lineage| lineages.contains(lineage)),
            Some("continue") => {
                let removed_lineage = tokens
                    .get(1)
                    .is_some_and(|lineage| lineages.contains(lineage));
                let removed_segment = tokens
                    .get(3)
                    .is_some_and(|segment| segments.contains(segment));
                let removed_parent = tokens.get(5).is_some_and(|pin| {
                    pin.rsplit_once('@')
                        .is_some_and(|(parent, _)| segments.contains(parent))
                });
                removed_lineage || removed_segment || removed_parent
            }
            Some("subgraph") | Some("subgraph_label") => tokens
                .get(1)
                .is_some_and(|id| removed_subgraphs.contains(id)),
            Some("subgraph_member") => {
                tokens
                    .get(1)
                    .is_some_and(|id| removed_subgraphs.contains(id))
                    || tokens
                        .get(3)
                        .is_some_and(|segment| segments.contains(segment))
            }
            _ => false,
        };
        if !remove {
            replacement.push_str(line);
        }
    }

    let replacement_timeline = Timeline::parse(&replacement)
        .map_err(|error| WorkbenchError::new(format!("deleted timeline is invalid: {error}")))?;
    if segments
        .iter()
        .any(|segment| replacement_timeline.segments.contains_key(segment))
        || replacement_timeline.segments.len() + segments.len() != timeline.segments.len()
    {
        return Err(WorkbenchError::new(
            "segment deletion changed unexpected timeline identities",
        ));
    }

    Ok(SegmentSourceDeletion {
        segments,
        goals,
        proofs,
        lineages,
        replacement,
    })
}

pub(crate) fn validated_timeline_edit_path(path: &Path) -> Result<PathBuf, WorkbenchError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot inspect timeline {}: {error}",
            path.display()
        ))
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(WorkbenchError::new(format!(
            "timeline {} is not a physical regular file",
            path.display()
        )));
    }
    fs::canonicalize(path).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot resolve timeline {}: {error}",
            path.display()
        ))
    })
}

pub(crate) struct SegmentDeletePlan {
    pub(crate) preview: SegmentDeletePreview,
    pub(crate) deletion_roots: Vec<String>,
    pub(crate) direct_draft_roots: Vec<String>,
    pub(crate) path: PathBuf,
    pub(crate) original: Vec<u8>,
    pub(crate) replacement: String,
    pub(crate) draft_ids: Vec<String>,
}

pub(crate) struct SegmentDeleteScope<'a> {
    deletion_roots: Vec<String>,
    direct_draft_roots: Vec<String>,
    operation_domain: &'static [u8],
    preferred_goal_anchor: Option<&'a str>,
}

pub(crate) fn segment_delete_plan(
    timeline_path: &Path,
    state_root: &Path,
    id: &str,
    manifests: &BTreeMap<String, DraftManifest>,
    active: &BTreeSet<String>,
) -> Result<SegmentDeletePlan, WorkbenchError> {
    segment_delete_plan_for_roots(
        timeline_path,
        state_root,
        id,
        SegmentDeleteScope {
            deletion_roots: vec![id.to_owned()],
            direct_draft_roots: Vec::new(),
            operation_domain: b"subtree",
            preferred_goal_anchor: None,
        },
        manifests,
        active,
    )
}

pub(crate) fn segment_delete_plan_for_roots(
    timeline_path: &Path,
    state_root: &Path,
    request_id: &str,
    scope: SegmentDeleteScope<'_>,
    manifests: &BTreeMap<String, DraftManifest>,
    active: &BTreeSet<String>,
) -> Result<SegmentDeletePlan, WorkbenchError> {
    let SegmentDeleteScope {
        deletion_roots,
        direct_draft_roots,
        operation_domain,
        preferred_goal_anchor,
    } = scope;
    let path = validated_timeline_edit_path(timeline_path)?;
    let original = fs::read(&path).map_err(|error| {
        WorkbenchError::new(format!("cannot read timeline {}: {error}", path.display()))
    })?;
    let source = std::str::from_utf8(&original)
        .map_err(|_| WorkbenchError::new("timeline source is not UTF-8"))?;
    let deletion = if deletion_roots.is_empty() {
        SegmentSourceDeletion {
            segments: BTreeSet::new(),
            goals: BTreeSet::new(),
            proofs: 0,
            lineages: BTreeSet::new(),
            replacement: source.into(),
        }
    } else {
        delete_segment_subtrees_in_timeline_source_preferring(
            source,
            deletion_roots.iter().map(String::as_str),
            preferred_goal_anchor,
        )?
    };

    for draft_id in &direct_draft_roots {
        if !manifests.contains_key(draft_id) {
            return Err(WorkbenchError::new(format!(
                "unknown direct sibling draft {draft_id:?}"
            )));
        }
    }
    let roots = manifests
        .values()
        .filter_map(|manifest| match &manifest.parent {
            DraftParent::Segment { id, .. } if deletion.segments.contains(id) => {
                Some(manifest.id.as_str())
            }
            _ => None,
        })
        .chain(direct_draft_roots.iter().map(String::as_str));
    let draft_deletion = draft_descendants_from_roots(manifests, roots);
    let drafts_root = validated_drafts_root(state_root)?;
    for draft_id in &draft_deletion {
        let manifest = &manifests[draft_id];
        if draft_is_active(&drafts_root.join(draft_id), manifest, active) {
            return Err(WorkbenchError::new(format!(
                "cannot delete segment {request_id:?}: attached recording {draft_id:?} is active"
            )));
        }
    }

    let graph_revision = draft_graph_revision(manifests)?;
    let mut digest = Sha256::new();
    digest.update(b"dusklight.route-workbench.segment-delete.v1\0");
    digest.update((operation_domain.len() as u64).to_le_bytes());
    digest.update(operation_domain);
    digest.update((original.len() as u64).to_le_bytes());
    digest.update(&original);
    digest.update(graph_revision.as_bytes());
    digest.update(deletion.replacement.as_bytes());
    for segment in &deletion.segments {
        digest.update((segment.len() as u64).to_le_bytes());
        digest.update(segment.as_bytes());
    }
    for draft in &draft_deletion {
        digest.update((draft.len() as u64).to_le_bytes());
        digest.update(draft.as_bytes());
    }
    let confirmation_token = format!("{:x}", digest.finalize());
    let timeline = Timeline::parse(source).expect("validated segment deletion source");
    let segments = deletion
        .segments
        .iter()
        .map(|segment_id| {
            let segment = &timeline.segments[segment_id];
            SegmentDeleteImpact {
                id: segment_id.clone(),
                name: segment.name.clone().unwrap_or_else(|| segment_id.clone()),
            }
        })
        .collect();
    let drafts = draft_deletion
        .iter()
        .map(|draft_id| {
            let manifest = &manifests[draft_id];
            DraftDeleteImpact {
                id: draft_id.clone(),
                label: manifest.label.clone(),
                status: manifest.status,
            }
        })
        .collect();
    let draft_ids = draft_deletion.into_iter().collect();
    Ok(SegmentDeletePlan {
        preview: SegmentDeletePreview {
            schema: SEGMENT_DELETE_PREVIEW_SCHEMA.into(),
            id: request_id.into(),
            segments,
            goals: deletion.goals.into_iter().collect(),
            proofs: deletion.proofs,
            lineages: deletion.lineages.into_iter().collect(),
            drafts,
            confirmation_token,
        },
        deletion_roots,
        direct_draft_roots,
        path,
        original,
        replacement: deletion.replacement,
        draft_ids,
    })
}

pub(crate) fn preview_segment_deletion(
    timeline_path: &Path,
    state_root: &Path,
    id: &str,
) -> Result<SegmentDeletePreview, WorkbenchError> {
    let active = active_recordings()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let manifests = scan_draft_manifests_with_active(state_root, &active)?;
    Ok(segment_delete_plan(timeline_path, state_root, id, &manifests, &active)?.preview)
}

pub(crate) fn structural_sibling_context(
    timeline: &Timeline,
    keep_id: &str,
) -> Result<(String, Vec<String>), WorkbenchError> {
    let selected = timeline
        .segments
        .get(keep_id)
        .ok_or_else(|| WorkbenchError::new(format!("unknown checked-in segment {keep_id:?}")))?;
    let parent = selected.parent.as_deref().ok_or_else(|| {
        WorkbenchError::new("the root segment has no structural siblings to delete")
    })?;
    let roots = timeline
        .segments
        .values()
        .filter(|segment| segment.id != keep_id && segment.parent.as_deref() == Some(parent))
        .map(|segment| segment.id.clone())
        .collect::<Vec<_>>();
    Ok((parent.into(), roots))
}

pub(crate) struct SiblingDeletePlan {
    pub(crate) deletion: SegmentDeletePlan,
    pub(crate) generated: Vec<GeneratedDeleteImpact>,
    pub(crate) generated_candidate_ids: Vec<String>,
}

pub(crate) fn sibling_delete_plan(
    timeline_path: &Path,
    repository_root: &Path,
    state_root: &Path,
    keep_id: &str,
    manifests: &BTreeMap<String, DraftManifest>,
    active: &BTreeSet<String>,
) -> Result<SiblingDeletePlan, WorkbenchError> {
    let initial_path = validated_timeline_edit_path(timeline_path)?;
    let initial_source = fs::read_to_string(&initial_path).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot read timeline {}: {error}",
            initial_path.display()
        ))
    })?;
    let initial_timeline =
        Timeline::parse(&initial_source).map_err(|error| WorkbenchError::new(error.to_string()))?;
    let (parent_id, roots) = structural_sibling_context(&initial_timeline, keep_id)?;
    let direct_draft_roots = manifests
        .values()
        .filter_map(|manifest| match &manifest.parent {
            DraftParent::Segment { id, .. } if id == &parent_id => Some(manifest.id.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let mut plan = segment_delete_plan_for_roots(
        timeline_path,
        state_root,
        keep_id,
        SegmentDeleteScope {
            deletion_roots: roots.clone(),
            direct_draft_roots,
            operation_domain: b"delete-siblings",
            preferred_goal_anchor: Some(keep_id),
        },
        manifests,
        active,
    )?;

    // The roots must have been derived from the exact bytes guarded by the plan's token.
    let planned_source = std::str::from_utf8(&plan.original)
        .map_err(|_| WorkbenchError::new("timeline source is not UTF-8"))?;
    let planned_timeline =
        Timeline::parse(planned_source).map_err(|error| WorkbenchError::new(error.to_string()))?;
    if structural_sibling_context(&planned_timeline, keep_id)? != (parent_id.clone(), roots)
        || plan
            .preview
            .segments
            .iter()
            .any(|segment| segment.id == keep_id)
    {
        return Err(WorkbenchError::new(
            "timeline topology changed while planning sibling deletion; reload and retry",
        ));
    }
    let generated = visible_generated_search_projections(
        &planned_timeline,
        &repository_root.join("build/search"),
        state_root,
    )?
    .into_iter()
    .filter(|projection| projection.segment.parent.as_deref() == Some(parent_id.as_str()))
    .filter_map(|projection| {
        let generated = projection.segment.generated?;
        Some(GeneratedDeleteImpact {
            id: projection.segment.id,
            name: projection
                .segment
                .name
                .unwrap_or_else(|| generated.candidate_id.clone()),
            candidate_id: generated.candidate_id,
            run: generated.run,
        })
    })
    .collect::<Vec<_>>();
    if plan.deletion_roots.is_empty() && plan.direct_draft_roots.is_empty() && generated.is_empty()
    {
        return Err(WorkbenchError::new(format!(
            "segment {keep_id:?} has no displayed siblings to delete"
        )));
    }
    let tombstones = load_generated_search_tombstones(state_root)?;
    let mut digest = Sha256::new();
    digest.update(b"dusklight.route-workbench.displayed-sibling-delete.v1\0");
    digest.update(plan.preview.confirmation_token.as_bytes());
    digest.update(
        serde_json::to_vec(&tombstones)
            .map_err(|error| WorkbenchError::new(format!("cannot hash tombstones: {error}")))?,
    );
    for candidate in &generated {
        digest.update(candidate.candidate_id.as_bytes());
        digest.update(candidate.run.as_bytes());
    }
    plan.preview.confirmation_token = format!("{:x}", digest.finalize());
    let generated_candidate_ids = generated
        .iter()
        .map(|candidate| candidate.candidate_id.clone())
        .collect();
    Ok(SiblingDeletePlan {
        deletion: plan,
        generated,
        generated_candidate_ids,
    })
}

pub(crate) fn sibling_preview(plan: &SiblingDeletePlan) -> SiblingDeletePreview {
    let deletion = &plan.deletion;
    let root_ids = plan
        .deletion
        .deletion_roots
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    SiblingDeletePreview {
        schema: SIBLING_DELETE_PREVIEW_SCHEMA.into(),
        keep_id: deletion.preview.id.clone(),
        sibling_roots: deletion
            .preview
            .segments
            .iter()
            .filter(|segment| root_ids.contains(segment.id.as_str()))
            .cloned()
            .collect(),
        draft_roots: deletion
            .preview
            .drafts
            .iter()
            .filter(|draft| deletion.direct_draft_roots.contains(&draft.id))
            .cloned()
            .collect(),
        generated: plan.generated.clone(),
        segments: deletion.preview.segments.clone(),
        goals: deletion.preview.goals.clone(),
        proofs: deletion.preview.proofs,
        lineages: deletion.preview.lineages.clone(),
        drafts: deletion.preview.drafts.clone(),
        confirmation_token: deletion.preview.confirmation_token.clone(),
    }
}

pub(crate) fn preview_sibling_deletion(
    timeline_path: &Path,
    repository_root: &Path,
    state_root: &Path,
    keep_id: &str,
) -> Result<SiblingDeletePreview, WorkbenchError> {
    let active = active_recordings()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let manifests = scan_draft_manifests_with_active(state_root, &active)?;
    let plan = sibling_delete_plan(
        timeline_path,
        repository_root,
        state_root,
        keep_id,
        &manifests,
        &active,
    )?;
    Ok(sibling_preview(&plan))
}

pub(crate) fn rename_segment(
    timeline_path: &Path,
    request: &BrowserSegmentRenameRequest,
) -> Result<SegmentRenameResult, SegmentRenameError> {
    let name = validate_segment_name(&request.name)?;
    let _edit = timeline_edits()
        .lock()
        .map_err(|_| WorkbenchError::new("timeline label edit lock is poisoned"))?;
    let path = validated_timeline_edit_path(timeline_path)?;
    let original = fs::read(&path).map_err(|error| {
        WorkbenchError::new(format!("cannot read timeline {}: {error}", path.display()))
    })?;
    let source = String::from_utf8(original.clone())
        .map_err(|_| WorkbenchError::new("timeline source is not UTF-8"))?;
    let timeline =
        Timeline::parse(&source).map_err(|error| WorkbenchError::new(error.to_string()))?;
    let segment = timeline
        .segments
        .get(&request.id)
        .ok_or_else(|| WorkbenchError::new(format!("unknown segment {:?}", request.id)))?;
    if segment.name != request.expected_name {
        return Err(SegmentRenameError::Conflict(
            "segment name changed; reload before renaming".into(),
        ));
    }
    let replacement_source = rename_segment_in_timeline_source(&source, &request.id, &name)?;
    let replacement_timeline = Timeline::parse(&replacement_source)
        .map_err(|error| WorkbenchError::new(error.to_string()))?;
    if replacement_timeline
        .segments
        .get(&request.id)
        .and_then(|segment| segment.name.as_deref())
        != Some(name.as_str())
    {
        return Err(
            WorkbenchError::new("renamed timeline did not preserve segment identity").into(),
        );
    }

    let directory = path
        .parent()
        .ok_or_else(|| WorkbenchError::new("timeline has no parent directory"))?;
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| WorkbenchError::new("timeline filename is not UTF-8"))?;
    let nonce = random_session_token()?;
    let temporary = directory.join(format!(".{filename}.{nonce}.tmp"));
    let backup = directory.join(format!(".{filename}.{nonce}.rollback"));
    let mut temporary_file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| {
            WorkbenchError::new(format!(
                "cannot create adjacent timeline temporary file {}: {error}",
                temporary.display()
            ))
        })?;
    let mut temporary_cleanup = RemoveFileOnDrop(Some(temporary.clone()));
    temporary_file
        .write_all(replacement_source.as_bytes())
        .and_then(|()| temporary_file.sync_all())
        .map_err(|error| {
            WorkbenchError::new(format!(
                "cannot flush timeline temporary file {}: {error}",
                temporary.display()
            ))
        })?;
    drop(temporary_file);

    if validated_timeline_edit_path(timeline_path)? != path
        || fs::read(&path).ok() != Some(original.clone())
    {
        return Err(SegmentRenameError::Conflict(
            "timeline changed while preparing rename; reload and retry".into(),
        ));
    }
    fs::rename(&path, &backup).map_err(|error| {
        WorkbenchError::new(format!("cannot stage timeline rollback backup: {error}"))
    })?;
    if fs::read(&backup).ok() != Some(original) {
        fs::rename(&backup, &path).map_err(|rollback| {
            WorkbenchError::new(format!(
                "timeline changed while staging its rollback backup and could not be restored: {rollback}"
            ))
        })?;
        return Err(
            WorkbenchError::new("timeline changed while staging its rollback backup").into(),
        );
    }
    if let Err(error) = fs::rename(&temporary, &path) {
        fs::rename(&backup, &path).map_err(|rollback| {
            WorkbenchError::new(format!(
                "cannot replace timeline ({error}) or restore rollback backup ({rollback})"
            ))
        })?;
        return Err(WorkbenchError::new(format!("cannot replace timeline: {error}")).into());
    }
    temporary_cleanup.0 = None;
    let _ = fs::remove_file(backup);
    Ok(SegmentRenameResult {
        schema: SEGMENT_RENAME_RESULT_SCHEMA.into(),
        id: request.id.clone(),
        name,
    })
}
