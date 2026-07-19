use super::*;

/// Build the JSON-ready graph projection used by the visual selector.
/// Missing or unsupported artifacts remain visible with `playable: false`.
pub(super) fn drafts_root(state_root: &Path) -> Result<PathBuf, WorkbenchError> {
    let root = state_root.join("drafts");
    fs::create_dir_all(&root).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot create draft root {}: {error}",
            root.display()
        ))
    })?;
    fs::canonicalize(&root).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot resolve draft root {}: {error}",
            root.display()
        ))
    })
}

pub(super) fn validated_drafts_root(state_root: &Path) -> Result<PathBuf, WorkbenchError> {
    fs::create_dir_all(state_root).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot create state root {}: {error}",
            state_root.display()
        ))
    })?;
    let state_root = fs::canonicalize(state_root).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot resolve state root {}: {error}",
            state_root.display()
        ))
    })?;
    let expected = state_root.join("drafts");
    fs::create_dir_all(&expected).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot create draft root {}: {error}",
            expected.display()
        ))
    })?;
    let metadata = fs::symlink_metadata(&expected).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot inspect draft root {}: {error}",
            expected.display()
        ))
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(WorkbenchError::new(
            "draft root is not a contained physical directory",
        ));
    }
    let resolved = fs::canonicalize(&expected).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot resolve draft root {}: {error}",
            expected.display()
        ))
    })?;
    if resolved != expected || resolved.parent() != Some(state_root.as_path()) {
        return Err(WorkbenchError::new(
            "draft root escapes the route workbench state root",
        ));
    }
    Ok(resolved)
}

pub(super) fn scan_draft_manifests(
    state_root: &Path,
) -> Result<BTreeMap<String, DraftManifest>, WorkbenchError> {
    let active = active_recordings()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    scan_draft_manifests_with_active(state_root, &active)
}

pub(super) fn scan_draft_manifests_with_active(
    state_root: &Path,
    active: &BTreeSet<String>,
) -> Result<BTreeMap<String, DraftManifest>, WorkbenchError> {
    let root = validated_drafts_root(state_root)?;
    let mut manifests = BTreeMap::new();
    let mut entries = fs::read_dir(&root)
        .map_err(|error| WorkbenchError::new(format!("cannot scan {}: {error}", root.display())))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| WorkbenchError::new(format!("cannot scan {}: {error}", root.display())))?;
    entries.sort_by_key(|entry| entry.file_name());
    if entries.len() > MAX_DRAFTS {
        return Err(WorkbenchError::new(format!(
            "draft store has {} entries; maximum is {MAX_DRAFTS}",
            entries.len()
        )));
    }
    for entry in entries {
        let file_type = entry
            .file_type()
            .map_err(|error| WorkbenchError::new(error.to_string()))?;
        if !file_type.is_dir() {
            continue;
        }
        let directory = match fs::canonicalize(entry.path()) {
            Ok(directory) if directory.starts_with(&root) && directory != root => directory,
            _ => continue,
        };
        let id = entry.file_name().to_string_lossy().into_owned();
        if !valid_draft_id(&id) {
            continue;
        }
        let final_path = directory.join(DRAFT_FINAL_MANIFEST);
        let path = if final_path.is_file() {
            final_path
        } else {
            directory.join(DRAFT_MANIFEST)
        };
        let path = match fs::canonicalize(&path) {
            Ok(path) if path.starts_with(&directory) => path,
            _ => continue,
        };
        if !path.is_file() {
            continue;
        }
        let bytes = fs::read(&path).map_err(|error| {
            WorkbenchError::new(format!("cannot read {}: {error}", path.display()))
        })?;
        if bytes.len() > 64 * 1024 {
            continue;
        }
        let mut manifest: DraftManifest = match serde_json::from_slice(&bytes) {
            Ok(manifest) => manifest,
            Err(_) => continue,
        };
        if manifest.schema != DRAFT_SCHEMA
            || manifest.id != id
            || manifest.tape != DRAFT_TAPE
            || manifest.endpoint_kind != "manual_stop"
            || manifest.verification != "unverified"
        {
            continue;
        }
        if matches!(
            manifest.status,
            DraftStatus::Preparing | DraftStatus::Recording
        ) {
            if active.contains(&id) {
                manifest.status = DraftStatus::Recording;
                manifests.insert(id, manifest);
                continue;
            }
            let status_exists = directory
                .join(format!("{DRAFT_TAPE}.status.json"))
                .is_file();
            let launch = read_draft_launch(&directory, &manifest);
            let launch_is_live = launch
                .as_ref()
                .is_some_and(|launch| process_is_alive(launch.pid));
            match (status_exists, launch.as_ref(), launch_is_live) {
                (true, _, true) => manifest.status = DraftStatus::Recording,
                (true, _, false) => {
                    finalize_recording(&directory, &mut manifest, None);
                    let _ = write_draft_manifest(&directory, &manifest, true);
                }
                (false, Some(_), true) => manifest.status = DraftStatus::Recording,
                (false, Some(_), false) => {
                    manifest.status = DraftStatus::ProcessFailure;
                    manifest.error = Some("recording process exited without final status".into());
                    let _ = write_draft_manifest(&directory, &manifest, true);
                }
                (false, None, _) => manifest.status = DraftStatus::Orphaned,
            }
        }
        manifests.insert(id, manifest);
    }
    Ok(manifests)
}

pub(super) fn draft_descendants(
    manifests: &BTreeMap<String, DraftManifest>,
    id: &str,
) -> Result<BTreeSet<String>, WorkbenchError> {
    if !valid_draft_id(id) || !manifests.contains_key(id) {
        return Err(WorkbenchError::new(format!("unknown draft {id:?}")));
    }
    Ok(draft_descendants_from_roots(manifests, [id]))
}

pub(super) fn draft_descendants_from_roots<'a>(
    manifests: &BTreeMap<String, DraftManifest>,
    roots: impl IntoIterator<Item = &'a str>,
) -> BTreeSet<String> {
    let mut children = BTreeMap::<&str, Vec<&str>>::new();
    for manifest in manifests.values() {
        if let DraftParent::Draft { id: parent, .. } = &manifest.parent {
            children
                .entry(parent.as_str())
                .or_default()
                .push(manifest.id.as_str());
        }
    }
    let mut deletion = BTreeSet::new();
    let mut pending = roots.into_iter().collect::<Vec<_>>();
    while let Some(next) = pending.pop() {
        if !deletion.insert(next.to_owned()) {
            continue;
        }
        if let Some(descendants) = children.get(next) {
            pending.extend(descendants.iter().copied());
        }
    }
    deletion
}

pub(super) fn draft_graph_revision(
    manifests: &BTreeMap<String, DraftManifest>,
) -> Result<String, WorkbenchError> {
    let mut digest = Sha256::new();
    digest.update(b"dusklight.route-workbench.draft-graph.v2\0");
    for (id, manifest) in manifests {
        let encoded = serde_json::to_vec(manifest).map_err(|error| {
            WorkbenchError::new(format!("cannot encode draft graph revision: {error}"))
        })?;
        digest.update((id.len() as u64).to_le_bytes());
        digest.update(id.as_bytes());
        digest.update((encoded.len() as u64).to_le_bytes());
        digest.update(encoded);
    }
    Ok(format!("{:x}", digest.finalize()))
}

pub(super) fn draft_delete_confirmation_token(
    graph_revision: &str,
    deletion: &BTreeSet<String>,
) -> String {
    let mut digest = Sha256::new();
    digest.update(b"dusklight.route-workbench.draft-delete.v1\0");
    digest.update(graph_revision.as_bytes());
    for id in deletion {
        digest.update((id.len() as u64).to_le_bytes());
        digest.update(id.as_bytes());
    }
    format!("{:x}", digest.finalize())
}

pub(super) fn draft_is_active(
    directory: &Path,
    manifest: &DraftManifest,
    active: &BTreeSet<String>,
) -> bool {
    active.contains(&manifest.id)
        || matches!(
            manifest.status,
            DraftStatus::Preparing | DraftStatus::Recording
        )
        || read_draft_launch(directory, manifest).is_some_and(|launch| process_is_alive(launch.pid))
}

pub(super) fn draft_delete_preview_locked(
    state_root: &Path,
    id: &str,
    manifests: &BTreeMap<String, DraftManifest>,
    active: &BTreeSet<String>,
) -> Result<DraftDeletePreview, WorkbenchError> {
    let deletion = draft_descendants(manifests, id)?;
    let root = validated_drafts_root(state_root)?;
    for draft_id in &deletion {
        let manifest = &manifests[draft_id];
        if draft_is_active(&root.join(draft_id), manifest, active) {
            return Err(WorkbenchError::new(format!(
                "cannot delete draft {id:?}: recording {draft_id:?} is active"
            )));
        }
    }
    let graph_revision = draft_graph_revision(manifests)?;
    let confirmation_token = draft_delete_confirmation_token(&graph_revision, &deletion);
    let drafts = deletion
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
    Ok(DraftDeletePreview {
        schema: DRAFT_DELETE_PREVIEW_SCHEMA.into(),
        id: id.into(),
        graph_revision,
        drafts,
        confirmation_token,
    })
}

pub(super) fn preview_draft_deletion(
    state_root: &Path,
    id: &str,
) -> Result<DraftDeletePreview, WorkbenchError> {
    let active = active_recordings()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let manifests = scan_draft_manifests_with_active(state_root, &active)?;
    draft_delete_preview_locked(state_root, id, &manifests, &active)
}

pub(super) fn validated_draft_directory(root: &Path, id: &str) -> Result<PathBuf, WorkbenchError> {
    if !valid_draft_id(id) {
        return Err(WorkbenchError::new(format!("invalid draft id {id:?}")));
    }
    let expected = root.join(id);
    let metadata = fs::symlink_metadata(&expected)
        .map_err(|error| WorkbenchError::new(format!("cannot inspect draft {id:?}: {error}")))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(WorkbenchError::new(format!(
            "draft {id:?} is not a contained physical directory"
        )));
    }
    let resolved = fs::canonicalize(&expected)
        .map_err(|error| WorkbenchError::new(format!("cannot resolve draft {id:?}: {error}")))?;
    if resolved != expected || resolved.parent() != Some(root) {
        return Err(WorkbenchError::new(format!(
            "draft {id:?} directory escapes the draft store"
        )));
    }
    Ok(resolved)
}

#[derive(Debug)]
pub(super) enum DraftRenameError {
    Conflict(String),
    Invalid(WorkbenchError),
}

impl fmt::Display for DraftRenameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Conflict(message) => formatter.write_str(message),
            Self::Invalid(error) => error.fmt(formatter),
        }
    }
}

impl From<WorkbenchError> for DraftRenameError {
    fn from(error: WorkbenchError) -> Self {
        Self::Invalid(error)
    }
}

pub(super) fn validated_draft_manifest_path(directory: &Path) -> Result<PathBuf, WorkbenchError> {
    let final_path = directory.join(DRAFT_FINAL_MANIFEST);
    let path = match fs::symlink_metadata(&final_path) {
        Ok(_) => final_path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            directory.join(DRAFT_MANIFEST)
        }
        Err(error) => {
            return Err(WorkbenchError::new(format!(
                "cannot inspect draft manifest {}: {error}",
                final_path.display()
            )));
        }
    };
    let metadata = fs::symlink_metadata(&path).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot inspect draft manifest {}: {error}",
            path.display()
        ))
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(WorkbenchError::new(format!(
            "draft manifest {} is not a contained regular file",
            path.display()
        )));
    }
    let resolved = fs::canonicalize(&path).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot resolve draft manifest {}: {error}",
            path.display()
        ))
    })?;
    if resolved != path || resolved.parent() != Some(directory) {
        return Err(WorkbenchError::new(format!(
            "draft manifest {} escapes its draft directory",
            path.display()
        )));
    }
    Ok(resolved)
}

pub(super) fn rollback_draft_manifest(backup: &Path, target: &Path) -> Result<(), WorkbenchError> {
    if fs::symlink_metadata(target).is_ok() {
        return Err(WorkbenchError::new(format!(
            "cannot restore draft manifest backup {} because {} now exists",
            backup.display(),
            target.display()
        )));
    }
    fs::rename(backup, target).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot restore draft manifest backup {} to {}: {error}",
            backup.display(),
            target.display()
        ))
    })
}

pub(super) fn replace_draft_manifest(
    path: &Path,
    expected: &[u8],
    replacement: &[u8],
) -> Result<(), WorkbenchError> {
    let directory = path
        .parent()
        .ok_or_else(|| WorkbenchError::new("draft manifest has no parent directory"))?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| WorkbenchError::new("draft manifest filename is not UTF-8"))?;
    let nonce = random_session_token()?;
    let temporary = directory.join(format!(".{name}.{nonce}.tmp"));
    let backup = directory.join(format!(".{name}.{nonce}.rollback"));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| {
            WorkbenchError::new(format!(
                "cannot create adjacent draft manifest temporary file {}: {error}",
                temporary.display()
            ))
        })?;
    let mut cleanup = RemoveFileOnDrop(Some(temporary.clone()));
    file.write_all(replacement)
        .and_then(|()| file.sync_all())
        .map_err(|error| {
            WorkbenchError::new(format!(
                "cannot flush draft manifest temporary file {}: {error}",
                temporary.display()
            ))
        })?;
    drop(file);

    let metadata = fs::symlink_metadata(path).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot revalidate draft manifest {}: {error}",
            path.display()
        ))
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(WorkbenchError::new(
            "draft manifest changed to a non-regular file during rename",
        ));
    }
    if fs::canonicalize(path).ok().as_deref() != Some(path)
        || !fs::read(path).is_ok_and(|bytes| bytes == expected)
    {
        return Err(WorkbenchError::new(
            "draft manifest changed while preparing rename; reload the graph",
        ));
    }

    fs::rename(path, &backup).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot stage draft manifest rollback backup {}: {error}",
            backup.display()
        ))
    })?;
    let moved_matches = fs::symlink_metadata(&backup)
        .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
        && fs::read(&backup).is_ok_and(|bytes| bytes == expected);
    if !moved_matches {
        rollback_draft_manifest(&backup, path)?;
        return Err(WorkbenchError::new(
            "draft manifest changed while staging its rollback backup",
        ));
    }
    if let Err(error) = fs::rename(&temporary, path) {
        rollback_draft_manifest(&backup, path)?;
        return Err(WorkbenchError::new(format!(
            "cannot replace draft manifest {}: {error}",
            path.display()
        )));
    }
    cleanup.0 = None;
    let _ = fs::remove_file(backup);
    Ok(())
}

pub(super) fn rename_draft_label(
    state_root: &Path,
    request: &BrowserDraftRenameRequest,
) -> Result<DraftRenameResult, DraftRenameError> {
    let label = validate_draft_label(&request.label)?;
    let active = active_recordings()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let manifests = scan_draft_manifests_with_active(state_root, &active)?;
    let revision = draft_graph_revision(&manifests)?;
    if revision != request.expected_graph_revision {
        return Err(DraftRenameError::Conflict(
            "draft graph changed; reload before renaming".into(),
        ));
    }
    let manifest = manifests
        .get(&request.id)
        .ok_or_else(|| WorkbenchError::new(format!("unknown draft {:?}", request.id)))?;
    let root = validated_drafts_root(state_root)?;
    let directory = validated_draft_directory(&root, &request.id)?;
    if draft_is_active(&directory, manifest, &active) {
        return Err(DraftRenameError::Conflict(format!(
            "cannot rename draft {:?} while its recording is active",
            request.id
        )));
    }
    let path = validated_draft_manifest_path(&directory)?;
    let original = fs::read(&path).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot read draft manifest {}: {error}",
            path.display()
        ))
    })?;
    let mut disk_manifest: DraftManifest = serde_json::from_slice(&original).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot decode draft manifest {}: {error}",
            path.display()
        ))
    })?;
    if disk_manifest.schema != DRAFT_SCHEMA || disk_manifest.id != request.id {
        return Err(
            WorkbenchError::new("draft manifest identity changed while preparing rename").into(),
        );
    }
    disk_manifest.label = label.clone();
    let replacement = serde_json::to_vec(&disk_manifest)
        .map_err(|error| WorkbenchError::new(format!("cannot encode draft manifest: {error}")))?;

    let latest = scan_draft_manifests_with_active(state_root, &active)?;
    if draft_graph_revision(&latest)? != request.expected_graph_revision {
        return Err(DraftRenameError::Conflict(
            "draft graph changed while preparing rename; reload the graph".into(),
        ));
    }
    replace_draft_manifest(&path, &original, &replacement)?;
    let updated = scan_draft_manifests_with_active(state_root, &active)?;
    Ok(DraftRenameResult {
        schema: DRAFT_RENAME_RESULT_SCHEMA.into(),
        id: request.id.clone(),
        label,
        graph_revision: draft_graph_revision(&updated)?,
    })
}

#[derive(Debug)]
pub(super) enum SegmentRenameError {
    Conflict(String),
    Invalid(WorkbenchError),
}

impl fmt::Display for SegmentRenameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Conflict(message) => formatter.write_str(message),
            Self::Invalid(error) => error.fmt(formatter),
        }
    }
}

impl From<WorkbenchError> for SegmentRenameError {
    fn from(error: WorkbenchError) -> Self {
        Self::Invalid(error)
    }
}

pub(super) fn validate_segment_name(name: &str) -> Result<String, WorkbenchError> {
    let name = name.trim();
    if name.is_empty()
        || name.len() > 160
        || name.chars().any(char::is_control)
        || name.contains(['"', '\\'])
    {
        return Err(WorkbenchError::new(
            "segment name must be 1 to 160 UTF-8 bytes without controls, quotes, or backslashes",
        ));
    }
    Ok(name.to_owned())
}

pub(super) fn timeline_line_ending(line: &str) -> &str {
    if line.ends_with("\r\n") {
        "\r\n"
    } else if line.ends_with('\n') {
        "\n"
    } else {
        ""
    }
}

pub(super) fn rename_segment_in_timeline_source(
    source: &str,
    id: &str,
    name: &str,
) -> Result<String, WorkbenchError> {
    let lines = source.split_inclusive('\n').collect::<Vec<_>>();
    let mut segment_index = None;
    let mut label_index = None;
    for (index, line) in lines.iter().enumerate() {
        let raw = line.trim_end_matches(['\r', '\n']);
        let tokens =
            tokenize(raw, index + 1).map_err(|error| WorkbenchError::new(error.to_string()))?;
        if tokens.first().map(String::as_str) == Some("segment")
            && tokens.get(1).map(String::as_str) == Some(id)
        {
            segment_index = Some(index);
        }
        if tokens.first().map(String::as_str) == Some("label")
            && tokens.get(1).map(String::as_str) == Some(id)
        {
            label_index = Some(index);
        }
    }
    let segment_index =
        segment_index.ok_or_else(|| WorkbenchError::new(format!("unknown segment {id:?}")))?;
    let preferred_ending = if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let authored = format!("label {id} \"{name}\"");
    let mut output = String::with_capacity(source.len() + authored.len() + 4);
    for (index, line) in lines.iter().enumerate() {
        if label_index == Some(index) {
            output.push_str(&authored);
            output.push_str(timeline_line_ending(line));
            continue;
        }
        output.push_str(line);
        if label_index.is_none() && index == segment_index {
            if timeline_line_ending(line).is_empty() {
                output.push_str(preferred_ending);
            }
            output.push_str(&authored);
            output.push_str(preferred_ending);
        }
    }
    Ok(output)
}

#[derive(Debug)]
pub(super) struct SegmentSourceDeletion {
    pub(super) segments: BTreeSet<String>,
    pub(super) goals: BTreeSet<String>,
    pub(super) proofs: usize,
    pub(super) lineages: BTreeSet<String>,
    pub(super) replacement: String,
}

pub(super) fn segment_descendants_from_roots<'a>(
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
pub(super) fn delete_segment_subtree_in_timeline_source(
    source: &str,
    id: &str,
) -> Result<SegmentSourceDeletion, WorkbenchError> {
    delete_segment_subtrees_in_timeline_source(source, [id])
}

#[cfg(test)]
pub(super) fn delete_segment_subtrees_in_timeline_source<'a>(
    source: &str,
    roots: impl IntoIterator<Item = &'a str>,
) -> Result<SegmentSourceDeletion, WorkbenchError> {
    delete_segment_subtrees_in_timeline_source_preferring(source, roots, None)
}

pub(super) fn delete_segment_subtrees_in_timeline_source_preferring<'a>(
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

pub(super) fn validated_timeline_edit_path(path: &Path) -> Result<PathBuf, WorkbenchError> {
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

pub(super) struct SegmentDeletePlan {
    preview: SegmentDeletePreview,
    deletion_roots: Vec<String>,
    direct_draft_roots: Vec<String>,
    path: PathBuf,
    original: Vec<u8>,
    replacement: String,
    draft_ids: Vec<String>,
}

pub(super) struct SegmentDeleteScope<'a> {
    deletion_roots: Vec<String>,
    direct_draft_roots: Vec<String>,
    operation_domain: &'static [u8],
    preferred_goal_anchor: Option<&'a str>,
}

pub(super) fn segment_delete_plan(
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

pub(super) fn segment_delete_plan_for_roots(
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

pub(super) fn preview_segment_deletion(
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

pub(super) fn structural_sibling_context(
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

pub(super) struct SiblingDeletePlan {
    deletion: SegmentDeletePlan,
    generated: Vec<GeneratedDeleteImpact>,
    generated_candidate_ids: Vec<String>,
}

pub(super) fn sibling_delete_plan(
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

pub(super) fn sibling_preview(plan: &SiblingDeletePlan) -> SiblingDeletePreview {
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

pub(super) fn preview_sibling_deletion(
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

pub(super) fn rename_segment(
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

pub(super) fn draft_trash_root(state_root: &Path) -> Result<PathBuf, WorkbenchError> {
    fs::create_dir_all(state_root).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot create state root {}: {error}",
            state_root.display()
        ))
    })?;
    let state_root = fs::canonicalize(state_root).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot resolve state root {}: {error}",
            state_root.display()
        ))
    })?;
    let trash = state_root.join(DRAFT_TRASH_DIRECTORY).join("drafts");
    fs::create_dir_all(&trash).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot create draft trash {}: {error}",
            trash.display()
        ))
    })?;
    let trash = fs::canonicalize(&trash).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot resolve draft trash {}: {error}",
            trash.display()
        ))
    })?;
    if !trash.starts_with(&state_root) || trash == state_root {
        return Err(WorkbenchError::new("draft trash escapes the state root"));
    }
    Ok(trash)
}

pub(super) struct DraftTrashMove {
    root: PathBuf,
    transaction: PathBuf,
    moved: Vec<(String, PathBuf)>,
}

impl DraftTrashMove {
    fn rollback(&mut self) -> Result<(), WorkbenchError> {
        let mut failures = Vec::new();
        for (draft_id, moved_path) in self.moved.iter().rev() {
            if let Err(error) = fs::rename(moved_path, self.root.join(draft_id)) {
                failures.push(format!("{draft_id}: {error}"));
            }
        }
        if failures.is_empty() {
            self.moved.clear();
            let _ = fs::remove_dir(&self.transaction);
            Ok(())
        } else {
            Err(WorkbenchError::new(format!(
                "cannot restore drafts after failed timeline edit: {}",
                failures.join(", ")
            )))
        }
    }
}

pub(super) fn move_draft_set_to_trash(
    state_root: &Path,
    draft_ids: &[String],
    token: &str,
) -> Result<Option<DraftTrashMove>, WorkbenchError> {
    if draft_ids.is_empty() {
        return Ok(None);
    }
    let root = validated_drafts_root(state_root)?;
    let mut sources = Vec::with_capacity(draft_ids.len());
    for draft_id in draft_ids {
        sources.push((
            draft_id.clone(),
            validated_draft_directory(&root, draft_id)?,
        ));
    }
    let trash = draft_trash_root(state_root)?;
    let nonce = random_session_token()?;
    let transaction = trash.join(format!(
        "{}-{}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
        &token[..16],
        nonce
    ));
    fs::create_dir(&transaction).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot create draft trash transaction {}: {error}",
            transaction.display()
        ))
    })?;
    let transaction = fs::canonicalize(&transaction).map_err(|error| {
        WorkbenchError::new(format!("cannot resolve draft trash transaction: {error}"))
    })?;
    if transaction.parent() != Some(trash.as_path()) {
        return Err(WorkbenchError::new(
            "draft trash transaction escapes the trash root",
        ));
    }

    let mut moved = Vec::new();
    for (draft_id, source) in &sources {
        let destination = transaction.join(draft_id);
        if let Err(error) = fs::rename(source, &destination) {
            let mut transaction_state = DraftTrashMove {
                root,
                transaction,
                moved,
            };
            let rollback = transaction_state.rollback().err();
            let suffix = rollback
                .map(|error| format!("; {error}"))
                .unwrap_or_default();
            return Err(WorkbenchError::new(format!(
                "cannot move draft {draft_id:?} into recoverable trash: {error}{suffix}"
            )));
        }
        moved.push((draft_id.clone(), destination));
    }
    Ok(Some(DraftTrashMove {
        root,
        transaction,
        moved,
    }))
}

#[derive(Debug)]
pub(super) enum SegmentDeleteError {
    Conflict(String),
    Invalid(WorkbenchError),
}

impl fmt::Display for SegmentDeleteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Conflict(message) => formatter.write_str(message),
            Self::Invalid(error) => error.fmt(formatter),
        }
    }
}

impl From<WorkbenchError> for SegmentDeleteError {
    fn from(error: WorkbenchError) -> Self {
        Self::Invalid(error)
    }
}

pub(super) fn rollback_draft_move(moved: &mut Option<DraftTrashMove>) -> String {
    moved
        .as_mut()
        .and_then(|transaction| transaction.rollback().err())
        .map(|error| format!("; {error}"))
        .unwrap_or_default()
}

pub(super) struct AppliedSegmentDeletion {
    segments: Vec<String>,
    drafts: Vec<String>,
    trash_transaction: Option<PathBuf>,
}

pub(super) fn apply_segment_delete_plan(
    timeline_path: &Path,
    state_root: &Path,
    plan: SegmentDeletePlan,
) -> Result<AppliedSegmentDeletion, SegmentDeleteError> {
    let directory = plan
        .path
        .parent()
        .ok_or_else(|| WorkbenchError::new("timeline has no parent directory"))?;
    let filename = plan
        .path
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
        .write_all(plan.replacement.as_bytes())
        .and_then(|()| temporary_file.sync_all())
        .map_err(|error| {
            WorkbenchError::new(format!(
                "cannot flush timeline temporary file {}: {error}",
                temporary.display()
            ))
        })?;
    drop(temporary_file);

    if validated_timeline_edit_path(timeline_path)? != plan.path
        || fs::read(&plan.path).ok() != Some(plan.original.clone())
    {
        return Err(SegmentDeleteError::Conflict(
            "timeline changed while preparing deletion; reload and retry".into(),
        ));
    }

    let mut moved = move_draft_set_to_trash(
        state_root,
        &plan.draft_ids,
        &plan.preview.confirmation_token,
    )?;
    if let Err(error) = fs::rename(&plan.path, &backup) {
        let rollback = rollback_draft_move(&mut moved);
        return Err(WorkbenchError::new(format!(
            "cannot stage timeline rollback backup: {error}{rollback}"
        ))
        .into());
    }
    if fs::read(&backup).ok() != Some(plan.original.clone()) {
        let restore = fs::rename(&backup, &plan.path).err();
        let rollback = rollback_draft_move(&mut moved);
        let restore = restore
            .map(|error| format!("; cannot restore timeline: {error}"))
            .unwrap_or_default();
        return Err(WorkbenchError::new(format!(
            "timeline changed while staging its rollback backup{restore}{rollback}"
        ))
        .into());
    }
    if let Err(error) = fs::rename(&temporary, &plan.path) {
        let restore = fs::rename(&backup, &plan.path).err();
        let rollback = rollback_draft_move(&mut moved);
        let restore = restore
            .map(|restore| format!("; cannot restore timeline: {restore}"))
            .unwrap_or_default();
        return Err(WorkbenchError::new(format!(
            "cannot replace timeline: {error}{restore}{rollback}"
        ))
        .into());
    }
    temporary_cleanup.0 = None;
    let _ = fs::remove_file(backup);

    Ok(AppliedSegmentDeletion {
        segments: plan
            .preview
            .segments
            .into_iter()
            .map(|segment| segment.id)
            .collect(),
        drafts: plan.draft_ids,
        trash_transaction: moved.map(|transaction| transaction.transaction),
    })
}

pub(super) fn apply_segment_deletion(
    timeline_path: &Path,
    state_root: &Path,
    request: &BrowserSegmentDeleteApplyRequest,
) -> Result<SegmentDeleteResult, SegmentDeleteError> {
    let _edit = timeline_edits()
        .lock()
        .map_err(|_| WorkbenchError::new("timeline edit lock is poisoned"))?;
    let active = active_recordings()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let manifests = scan_draft_manifests_with_active(state_root, &active)?;
    let plan = segment_delete_plan(timeline_path, state_root, &request.id, &manifests, &active)?;
    if request.confirmation_token != plan.preview.confirmation_token {
        return Err(SegmentDeleteError::Conflict(
            "timeline or attached drafts changed after preview; reload and confirm deletion again"
                .into(),
        ));
    }
    let result = apply_segment_delete_plan(timeline_path, state_root, plan)?;
    Ok(SegmentDeleteResult {
        schema: SEGMENT_DELETE_RESULT_SCHEMA.into(),
        id: request.id.clone(),
        segments: result.segments,
        drafts: result.drafts,
        trash_transaction: result.trash_transaction,
    })
}

pub(super) struct AppliedTombstoneEdit {
    target: PathBuf,
    backup: Option<PathBuf>,
    had_original: bool,
    active: bool,
}

impl AppliedTombstoneEdit {
    fn rollback(&mut self) -> Result<(), WorkbenchError> {
        if !self.active {
            return Ok(());
        }
        if self.target.exists() {
            fs::remove_file(&self.target).map_err(|error| {
                WorkbenchError::new(format!("cannot roll back search tombstones: {error}"))
            })?;
        }
        if self.had_original {
            let backup = self.backup.as_ref().ok_or_else(|| {
                WorkbenchError::new("search tombstone rollback backup is missing")
            })?;
            fs::rename(backup, &self.target).map_err(|error| {
                WorkbenchError::new(format!("cannot restore search tombstones: {error}"))
            })?;
        }
        self.active = false;
        Ok(())
    }

    fn commit(mut self) {
        if let Some(backup) = self.backup.take() {
            let _ = fs::remove_file(backup);
        }
        self.active = false;
    }
}

impl Drop for AppliedTombstoneEdit {
    fn drop(&mut self) {
        let _ = self.rollback();
    }
}

pub(super) fn apply_generated_search_tombstones(
    state_root: &Path,
    candidate_ids: &[String],
) -> Result<Option<AppliedTombstoneEdit>, WorkbenchError> {
    if candidate_ids.is_empty() {
        return Ok(None);
    }
    fs::create_dir_all(state_root).map_err(|error| {
        WorkbenchError::new(format!("cannot create state root for tombstones: {error}"))
    })?;
    let root = fs::canonicalize(state_root)
        .map_err(|error| WorkbenchError::new(format!("cannot resolve state root: {error}")))?;
    let target = root.join(GENERATED_SEARCH_TOMBSTONES);
    let original = match fs::symlink_metadata(&target) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(WorkbenchError::new(
                    "generated search tombstones are not a physical file",
                ));
            }
            Some(fs::read(&target).map_err(|error| {
                WorkbenchError::new(format!("cannot read search tombstones: {error}"))
            })?)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(WorkbenchError::new(format!(
                "cannot inspect search tombstones: {error}"
            )));
        }
    };
    let mut tombstones = load_generated_search_tombstones(&root)?;
    tombstones
        .candidate_ids
        .extend(candidate_ids.iter().cloned());
    let replacement = serde_json::to_vec_pretty(&tombstones)
        .map_err(|error| WorkbenchError::new(format!("cannot encode tombstones: {error}")))?;
    let nonce = random_session_token()?;
    let temporary = root.join(format!(".{GENERATED_SEARCH_TOMBSTONES}.{nonce}.tmp"));
    let backup = original
        .as_ref()
        .map(|_| root.join(format!(".{GENERATED_SEARCH_TOMBSTONES}.{nonce}.rollback")));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| WorkbenchError::new(format!("cannot stage tombstones: {error}")))?;
    file.write_all(&replacement)
        .and_then(|()| file.sync_all())
        .map_err(|error| WorkbenchError::new(format!("cannot flush tombstones: {error}")))?;
    drop(file);
    let mut temporary_cleanup = RemoveFileOnDrop(Some(temporary.clone()));
    if fs::read(&target).ok() != original {
        return Err(WorkbenchError::new(
            "generated search tombstones changed while preparing deletion",
        ));
    }
    if let Some(backup) = &backup {
        fs::rename(&target, backup).map_err(|error| {
            WorkbenchError::new(format!("cannot stage search tombstone rollback: {error}"))
        })?;
    }
    if let Err(error) = fs::rename(&temporary, &target) {
        if let Some(backup) = &backup {
            let _ = fs::rename(backup, &target);
        }
        return Err(WorkbenchError::new(format!(
            "cannot install generated search tombstones: {error}"
        )));
    }
    temporary_cleanup.0 = None;
    Ok(Some(AppliedTombstoneEdit {
        target,
        backup,
        had_original: original.is_some(),
        active: true,
    }))
}

pub(super) fn apply_sibling_deletion(
    timeline_path: &Path,
    repository_root: &Path,
    state_root: &Path,
    request: &BrowserSiblingDeleteApplyRequest,
) -> Result<SiblingDeleteResult, SegmentDeleteError> {
    let _edit = timeline_edits()
        .lock()
        .map_err(|_| WorkbenchError::new("timeline edit lock is poisoned"))?;
    let active = active_recordings()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let manifests = scan_draft_manifests_with_active(state_root, &active)?;
    let plan = sibling_delete_plan(
        timeline_path,
        repository_root,
        state_root,
        &request.keep_id,
        &manifests,
        &active,
    )?;
    if request.confirmation_token != plan.deletion.preview.confirmation_token {
        return Err(SegmentDeleteError::Conflict(
            "timeline or attached drafts changed after preview; reload and confirm sibling deletion again"
                .into(),
        ));
    }
    let sibling_roots = plan.deletion.deletion_roots.clone();
    let draft_roots = plan.deletion.direct_draft_roots.clone();
    let generated_candidates = plan.generated_candidate_ids.clone();
    let mut tombstone_edit =
        apply_generated_search_tombstones(state_root, &plan.generated_candidate_ids)?;
    let result = match apply_segment_delete_plan(timeline_path, state_root, plan.deletion) {
        Ok(result) => result,
        Err(error) => {
            if let Some(edit) = tombstone_edit.as_mut() {
                edit.rollback()?;
            }
            return Err(error);
        }
    };
    if let Some(edit) = tombstone_edit {
        edit.commit();
    }
    Ok(SiblingDeleteResult {
        schema: SIBLING_DELETE_RESULT_SCHEMA.into(),
        keep_id: request.keep_id.clone(),
        sibling_roots,
        draft_roots,
        generated_candidates,
        segments: result.segments,
        drafts: result.drafts,
        trash_transaction: result.trash_transaction,
    })
}

pub(super) fn apply_draft_deletion(
    state_root: &Path,
    request: &BrowserDraftDeleteApplyRequest,
) -> Result<DraftDeleteResult, WorkbenchError> {
    let active = active_recordings()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let manifests = scan_draft_manifests_with_active(state_root, &active)?;
    let preview = draft_delete_preview_locked(state_root, &request.id, &manifests, &active)?;
    if request.confirmation_token != preview.confirmation_token {
        return Err(WorkbenchError::new(
            "draft graph changed after preview; request a new deletion preview",
        ));
    }

    let draft_ids = preview
        .drafts
        .iter()
        .map(|draft| draft.id.clone())
        .collect::<Vec<_>>();
    let moved = move_draft_set_to_trash(state_root, &draft_ids, &preview.confirmation_token)?
        .expect("a draft deletion always moves at least one draft");

    Ok(DraftDeleteResult {
        schema: DRAFT_DELETE_RESULT_SCHEMA.into(),
        id: request.id.clone(),
        graph_revision: preview.graph_revision,
        drafts: preview.drafts.into_iter().map(|draft| draft.id).collect(),
        trash_transaction: moved.transaction,
    })
}

pub(super) fn read_draft_launch(directory: &Path, manifest: &DraftManifest) -> Option<DraftLaunch> {
    let bytes = fs::read(directory.join(DRAFT_LAUNCH)).ok()?;
    let launch: DraftLaunch = serde_json::from_slice(&bytes).ok()?;
    (launch.schema == "dusklight.route-workbench.launch.v2"
        && launch.id == manifest.id
        && launch.session_token == manifest.session_token)
        .then_some(launch)
}

pub(super) fn graph_drafts_from_manifests(
    timeline: &Timeline,
    repository_root: &Path,
    state_root: &Path,
    manifests: BTreeMap<String, DraftManifest>,
) -> Result<Vec<GraphDraft>, WorkbenchError> {
    let root = drafts_root(state_root)?;
    let mut memo = BTreeMap::new();
    let mut anchor_digests = BTreeMap::new();
    for id in manifests.keys() {
        let _ = validate_draft_structure(
            id,
            &manifests,
            timeline,
            repository_root,
            &root,
            &mut memo,
            &mut anchor_digests,
        );
    }
    Ok(manifests
        .into_values()
        .map(|manifest| {
            let mut error = manifest.error.clone();
            if error.is_none() && manifest.status == DraftStatus::Ready {
                error = memo
                    .get(&manifest.id)
                    .and_then(|result| result.clone().err());
            }
            let playable = manifest.status == DraftStatus::Ready && error.is_none();
            GraphDraft {
                id: manifest.id,
                label: manifest.label,
                parent: manifest.parent,
                created_unix_ms: manifest.created_unix_ms,
                status: manifest.status,
                frame_count: manifest.frames,
                playable,
                endpoint_kind: manifest.endpoint_kind,
                verification: manifest.verification,
                tape_sha256: manifest.tape_sha256,
                result_tape_sha256: manifest.result_tape_sha256,
                tape_bytes: manifest.tape_bytes,
                thumbnail: None,
                error,
            }
        })
        .collect())
}

pub(super) fn validate_draft_structure(
    id: &str,
    manifests: &BTreeMap<String, DraftManifest>,
    timeline: &Timeline,
    repository_root: &Path,
    drafts_root: &Path,
    memo: &mut BTreeMap<String, Result<(), String>>,
    anchor_digests: &mut BTreeMap<(String, usize), Result<String, String>>,
) -> Result<(), String> {
    if let Some(result) = memo.get(id) {
        return result.clone();
    }
    memo.insert(
        id.to_owned(),
        Err("draft parent graph contains a cycle".into()),
    );
    let result = (|| {
        let manifest = manifests
            .get(id)
            .ok_or_else(|| "parent draft is missing".to_owned())?;
        if manifest.status != DraftStatus::Ready {
            return Err(format!("parent draft {id:?} is not ready"));
        }
        for (name, digest) in [
            ("parent", Some(manifest.parent_tape_sha256.as_str())),
            ("continuation", manifest.tape_sha256.as_deref()),
            ("result", manifest.result_tape_sha256.as_deref()),
        ] {
            if !digest.is_some_and(valid_sha256) {
                return Err(format!("draft {id:?} has invalid {name} tape digest"));
            }
        }
        if manifest.tape_bytes.is_none() || manifest.frames.is_none() {
            return Err(format!("draft {id:?} lacks finalized tape metadata"));
        }
        let draft_directory = fs::canonicalize(drafts_root.join(id))
            .map_err(|_| format!("draft {id:?} directory is missing"))?;
        let tape_path = fs::canonicalize(draft_directory.join(DRAFT_TAPE))
            .map_err(|_| format!("draft {id:?} continuation tape is missing"))?;
        if !draft_directory.starts_with(drafts_root) || !tape_path.starts_with(&draft_directory) {
            return Err(format!("draft {id:?} continuation tape escapes the store"));
        }
        let tape_bytes = fs::read(&tape_path)
            .map_err(|_| format!("draft {id:?} continuation tape is unreadable"))?;
        if manifest.tape_bytes != Some(tape_bytes.len() as u64)
            || manifest.tape_sha256.as_deref()
                != Some(format!("{:x}", Sha256::digest(&tape_bytes)).as_str())
        {
            return Err(format!("draft {id:?} continuation tape was tampered"));
        }
        let continuation = InputTape::decode(&tape_bytes)
            .map_err(|_| format!("draft {id:?} continuation tape is invalid"))?
            .tape;
        if continuation.frames.is_empty()
            || manifest.frames != Some(continuation.frames.len() as u64)
        {
            return Err(format!(
                "draft {id:?} continuation frame metadata is inconsistent"
            ));
        }
        match &manifest.parent {
            DraftParent::Milestone {
                id,
                program_sha256,
                definition_sha256,
                boundary_fingerprint,
            } => {
                let program = milestone_program_projection(timeline, repository_root)
                    .map_err(|error| error.to_string())?
                    .ok_or_else(|| "Boot parent has no authored milestone program".to_owned())?;
                let definition = program
                    .definitions
                    .iter()
                    .find(|definition| definition.name == *id)
                    .ok_or_else(|| "Boot parent milestone no longer exists".to_owned())?;
                if program.program_sha256 != *program_sha256
                    || definition.definition_sha256 != *definition_sha256
                    || !is_exact_boot_boundary_predicate(definition)
                    || !manifest.start_boundary_verified
                    || !boundary_fingerprint
                        .as_deref()
                        .is_some_and(native_fingerprint)
                    || manifest.parent_tape_sha256
                        != tape_digest(&InputTape::default()).map_err(|error| error.to_string())?
                {
                    return Err("Boot milestone parent proof is missing or stale".into());
                }
            }
            DraftParent::Segment {
                id: segment_id,
                terminal_milestone: _,
                boundary_fingerprint,
            } => {
                let segment = timeline
                    .segments
                    .get(segment_id)
                    .ok_or_else(|| "parent segment no longer exists".to_owned())?;
                if segment.end_fingerprint != *boundary_fingerprint
                    || !manifest.start_boundary_verified
                {
                    return Err("curated segment anchor is no longer exact".into());
                }
                let key = (segment_id.clone(), 0);
                let parent_digest = if let Some(result) = anchor_digests.get(&key) {
                    result.clone()?
                } else {
                    let result = materialize_segment_chain(timeline, repository_root, segment_id)
                        .map_err(|error| error.to_string())
                        .and_then(|materialized| {
                            tape_digest(&materialized.tape).map_err(|error| error.to_string())
                        });
                    anchor_digests.insert(key, result.clone());
                    result?
                };
                if manifest.parent_tape_sha256 != parent_digest {
                    return Err("curated parent tape digest changed".into());
                }
            }
            DraftParent::Draft {
                id: parent_id,
                parent_tape_sha256,
            } => {
                validate_draft_structure(
                    parent_id,
                    manifests,
                    timeline,
                    repository_root,
                    drafts_root,
                    memo,
                    anchor_digests,
                )?;
                let parent = &manifests[parent_id];
                if parent.result_tape_sha256.as_deref() != Some(parent_tape_sha256)
                    || manifest.parent_tape_sha256 != *parent_tape_sha256
                {
                    return Err("draft parent chain digest mismatch".into());
                }
            }
        }
        Ok(())
    })();
    memo.insert(id.to_owned(), result.clone());
    result
}

pub(super) fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

pub(super) fn valid_draft_id(id: &str) -> bool {
    id.starts_with("draft-")
        && id.len() <= 80
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

#[cfg(windows)]
pub(super) fn process_is_alive(pid: u32) -> bool {
    type Handle = *mut std::ffi::c_void;
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, process_id: u32) -> Handle;
        fn GetExitCodeProcess(process: Handle, exit_code: *mut u32) -> i32;
        fn CloseHandle(object: Handle) -> i32;
    }
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const STILL_ACTIVE: u32 = 259;
    // SAFETY: handles are checked for null, the output pointer is valid for the
    // duration of the call, and every opened handle is closed exactly once.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return false;
        }
        let mut exit_code = 0;
        let ok = GetExitCodeProcess(process, &mut exit_code);
        let _ = CloseHandle(process);
        ok != 0 && exit_code == STILL_ACTIVE
    }
}

#[cfg(target_os = "linux")]
pub(super) fn process_is_alive(pid: u32) -> bool {
    Path::new("/proc").join(pid.to_string()).exists()
}

#[cfg(all(unix, not(target_os = "linux")))]
pub(super) fn process_is_alive(pid: u32) -> bool {
    unsafe extern "C" {
        fn kill(pid: i32, signal: i32) -> i32;
    }
    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };
    // SAFETY: signal zero does not deliver a signal; it only asks the kernel
    // whether this process exists and is visible to the caller.
    let visible = unsafe { kill(pid, 0) == 0 };
    visible || std::io::Error::last_os_error().kind() == std::io::ErrorKind::PermissionDenied
}

#[cfg(not(any(windows, unix)))]
pub(super) fn process_is_alive(_pid: u32) -> bool {
    false
}

pub(super) fn write_draft_manifest(
    directory: &Path,
    manifest: &DraftManifest,
    finalized: bool,
) -> Result<(), WorkbenchError> {
    let bytes = serde_json::to_vec(manifest)
        .map_err(|error| WorkbenchError::new(format!("cannot encode draft manifest: {error}")))?;
    let target = directory.join(if finalized {
        DRAFT_FINAL_MANIFEST
    } else {
        DRAFT_MANIFEST
    });
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = directory.join(format!(".draft-{nonce}.tmp"));
    let mut file = fs::File::create(&temporary).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot write draft manifest {}: {error}",
            directory.display()
        ))
    })?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| {
            WorkbenchError::new(format!("cannot flush {}: {error}", temporary.display()))
        })?;
    match fs::rename(&temporary, &target) {
        Ok(()) => Ok(()),
        Err(_) if target.is_file() => {
            let _ = fs::remove_file(&temporary);
            Ok(())
        }
        Err(error) => Err(WorkbenchError::new(format!(
            "cannot atomically install {}: {error}",
            target.display()
        ))),
    }
}

pub(super) fn write_draft_launch(
    directory: &Path,
    launch: &DraftLaunch,
) -> Result<(), WorkbenchError> {
    let bytes = serde_json::to_vec(launch)
        .map_err(|error| WorkbenchError::new(format!("cannot encode draft launch: {error}")))?;
    let target = directory.join(DRAFT_LAUNCH);
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = directory.join(format!(".launch-{nonce}.tmp"));
    let mut file = fs::File::create(&temporary)
        .map_err(|error| WorkbenchError::new(format!("cannot create launch state: {error}")))?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| WorkbenchError::new(format!("cannot flush launch state: {error}")))?;
    fs::rename(&temporary, &target)
        .map_err(|error| WorkbenchError::new(format!("cannot install launch state: {error}")))
}

pub(super) fn random_session_token() -> Result<String, WorkbenchError> {
    let mut bytes = [0_u8; 16];
    fill_random(&mut bytes)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[cfg(windows)]
pub(super) fn fill_random(output: &mut [u8]) -> Result<(), WorkbenchError> {
    #[link(name = "bcrypt")]
    unsafe extern "system" {
        fn BCryptGenRandom(
            algorithm: *mut std::ffi::c_void,
            buffer: *mut u8,
            length: u32,
            flags: u32,
        ) -> i32;
    }
    const BCRYPT_USE_SYSTEM_PREFERRED_RNG: u32 = 2;
    // SAFETY: `output` is a live writable slice and its length fits in u32.
    let status = unsafe {
        BCryptGenRandom(
            std::ptr::null_mut(),
            output.as_mut_ptr(),
            output.len() as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    if status == 0 {
        Ok(())
    } else {
        Err(WorkbenchError::new(format!(
            "system random generator failed with status {status}"
        )))
    }
}

#[cfg(not(windows))]
pub(super) fn fill_random(output: &mut [u8]) -> Result<(), WorkbenchError> {
    fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(output))
        .map_err(|error| WorkbenchError::new(format!("system random generator failed: {error}")))
}

pub(super) fn tape_digest(tape: &InputTape) -> Result<String, WorkbenchError> {
    let encoded = tape
        .encode()
        .map_err(|error| WorkbenchError::new(error.to_string()))?;
    Ok(format!("{:x}", Sha256::digest(encoded)))
}

pub(super) fn read_draft_tape(directory: &Path) -> Result<(Vec<u8>, InputTape), WorkbenchError> {
    let directory = fs::canonicalize(directory)
        .map_err(|error| WorkbenchError::new(format!("cannot resolve draft directory: {error}")))?;
    let path = fs::canonicalize(directory.join(DRAFT_TAPE))
        .map_err(|error| WorkbenchError::new(format!("cannot resolve draft tape: {error}")))?;
    if !path.starts_with(&directory) || !path.is_file() {
        return Err(WorkbenchError::new(
            "draft tape escapes its draft directory",
        ));
    }
    let bytes = fs::read(&path).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot read draft tape {}: {error}",
            path.display()
        ))
    })?;
    let decoded = InputTape::decode(&bytes).map_err(|error| {
        WorkbenchError::new(format!("invalid draft tape {}: {error}", path.display()))
    })?;
    Ok((bytes, decoded.tape))
}
