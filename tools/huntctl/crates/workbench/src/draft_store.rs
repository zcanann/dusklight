use super::*;

mod deletion;
mod persistence;
mod segment_edits;
pub(crate) use deletion::*;
pub(crate) use persistence::*;
pub(crate) use segment_edits::*;

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
