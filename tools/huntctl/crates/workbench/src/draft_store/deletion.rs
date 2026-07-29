use super::*;

pub(crate) fn draft_trash_root(state_root: &Path) -> Result<PathBuf, WorkbenchError> {
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

pub(crate) struct DraftTrashMove {
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

pub(crate) fn move_draft_set_to_trash(
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
pub(crate) enum SegmentDeleteError {
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

pub(crate) fn rollback_draft_move(moved: &mut Option<DraftTrashMove>) -> String {
    moved
        .as_mut()
        .and_then(|transaction| transaction.rollback().err())
        .map(|error| format!("; {error}"))
        .unwrap_or_default()
}

pub(crate) struct AppliedSegmentDeletion {
    segments: Vec<String>,
    drafts: Vec<String>,
    trash_transaction: Option<PathBuf>,
}

pub(crate) fn apply_segment_delete_plan(
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

pub(crate) fn apply_segment_deletion(
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

pub(crate) struct AppliedTombstoneEdit {
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

pub(crate) fn apply_generated_search_tombstones(
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

pub(crate) fn apply_sibling_deletion(
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

pub(crate) fn apply_draft_deletion(
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
