//! Seal, persist, and resume the native goal-learning journal.

use super::*;

pub(super) fn record_identity(
    record: &NativeGoalLearningLoopRecord,
) -> Result<Digest, NativeGoalLearningLoopError> {
    let mut canonical = record.clone();
    canonical.record_sha256 = Digest::ZERO;
    let domain = match record.schema.as_str() {
        NATIVE_GOAL_LEARNING_LOOP_RECORD_SCHEMA_V2 => {
            b"dusklight.native-goal-learning-loop-record/v2\0".as_slice()
        }
        _ => b"dusklight.native-goal-learning-loop-record/v3\0".as_slice(),
    };
    canonical_digest(domain, &canonical)
}

pub(super) fn record_bytes(
    record: &NativeGoalLearningLoopRecord,
) -> Result<Vec<u8>, NativeGoalLearningLoopError> {
    let mut bytes = serde_json::to_vec(record).map_err(loop_error)?;
    bytes.push(b'\n');
    Ok(bytes)
}

pub(super) fn validate_artifact_shape(
    label: &str,
    reference: &ArtifactReference,
) -> Result<(), NativeGoalLearningLoopError> {
    validate_relative_path(label, &reference.path)?;
    if reference.sha256 == Digest::ZERO {
        return Err(loop_message(format!("{label} has a zero digest")));
    }
    Ok(())
}

pub(super) fn validate_relative_path(
    label: &str,
    value: &str,
) -> Result<(), NativeGoalLearningLoopError> {
    let path = Path::new(value);
    if value.is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(loop_message(format!(
            "{label} path is not repository relative"
        )));
    }
    Ok(())
}

pub(super) fn read_reference(
    root: &Path,
    reference: &ArtifactReference,
) -> Result<Vec<u8>, NativeGoalLearningLoopError> {
    validate_artifact_shape("artifact", reference)?;
    let bytes = fs::read(root.join(&reference.path)).map_err(NativeGoalLearningLoopError::io)?;
    if sha256(&bytes) != reference.sha256 {
        return Err(loop_message("learning-loop artifact digest differs"));
    }
    Ok(bytes)
}

pub(super) fn referenced_path(
    root: &Path,
    reference: &ArtifactReference,
) -> Result<PathBuf, NativeGoalLearningLoopError> {
    read_reference(root, reference)?;
    Ok(root.join(&reference.path))
}

pub(super) fn output_path(
    root: &Path,
    relative: &str,
) -> Result<PathBuf, NativeGoalLearningLoopError> {
    validate_relative_path("learning-loop output", relative)?;
    Ok(root.join(relative))
}

pub(super) fn canonical_root(root: &Path) -> Result<PathBuf, NativeGoalLearningLoopError> {
    root.canonicalize().map_err(NativeGoalLearningLoopError::io)
}

pub(super) fn create_parent(path: &Path) -> Result<(), NativeGoalLearningLoopError> {
    let parent = path
        .parent()
        .ok_or_else(|| loop_message("learning-loop output has no parent"))?;
    fs::create_dir_all(parent).map_err(NativeGoalLearningLoopError::io)
}

pub(super) fn write_state_atomically(
    path: &Path,
    state: &NativeGoalLearningLoopState,
) -> Result<(), NativeGoalLearningLoopError> {
    create_parent(path)?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(loop_error)?
        .as_nanos();
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| loop_message("learning-loop state filename is invalid"))?;
    let temporary =
        path.with_file_name(format!(".{file_name}.{}.{}.tmp", std::process::id(), nonce));
    let bytes = state.to_pretty_json()?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(NativeGoalLearningLoopError::io)?;
    output
        .write_all(&bytes)
        .map_err(NativeGoalLearningLoopError::io)?;
    output.sync_all().map_err(NativeGoalLearningLoopError::io)?;
    fs::rename(&temporary, path).map_err(NativeGoalLearningLoopError::io)?;
    sync_parent(path)
}

pub(super) fn sync_parent(path: &Path) -> Result<(), NativeGoalLearningLoopError> {
    fs::File::open(
        path.parent()
            .ok_or_else(|| loop_message("learning-loop output has no parent"))?,
    )
    .and_then(|directory| directory.sync_all())
    .map_err(NativeGoalLearningLoopError::io)
}

pub(super) fn pretty_json(value: &impl Serialize) -> Result<Vec<u8>, NativeGoalLearningLoopError> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(loop_error)?;
    bytes.push(b'\n');
    Ok(bytes)
}

pub(super) fn canonical_digest(
    domain: &[u8],
    value: &impl Serialize,
) -> Result<Digest, NativeGoalLearningLoopError> {
    let bytes = serde_json::to_vec(value).map_err(loop_error)?;
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(bytes);
    Ok(Digest(hasher.finalize().into()))
}

pub(super) fn sha256(bytes: &[u8]) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Digest(hasher.finalize().into())
}

pub(super) fn lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
