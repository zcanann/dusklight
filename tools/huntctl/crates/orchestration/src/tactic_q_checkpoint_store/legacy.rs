use super::*;

pub(crate) fn write_checkpoint_with_local_store(
    checkpoint: &TacticQCampaignCheckpoint,
    directory: &Path,
) -> Result<PathBuf, TacticQCampaignError> {
    write_checkpoint(checkpoint, directory, &directory.join(CONTENT_DIRECTORY))
}

pub(crate) fn write_checkpoint(
    checkpoint: &TacticQCampaignCheckpoint,
    directory: &Path,
    content_root: &Path,
) -> Result<PathBuf, TacticQCampaignError> {
    if content_root.file_name().and_then(|name| name.to_str()) != Some(CONTENT_DIRECTORY) {
        return Err(TacticQCampaignError::InvalidState(
            "checkpoint content root must use the discoverable objects directory",
        ));
    }
    let store = TacticQContentStore::initialize(content_root).map_err(checkpoint_store_error)?;
    validate_checkpoint(checkpoint)?;
    write_validated_checkpoint_to_store(checkpoint, directory, &store)
}

/// Writes a checkpoint already constructed and validated by
/// `TacticQCampaign::checkpoint`. Freshly decoded or externally supplied
/// checkpoints must use `write_checkpoint`.
fn write_validated_checkpoint_to_store(
    checkpoint: &TacticQCampaignCheckpoint,
    directory: &Path,
    store: &TacticQContentStore,
) -> Result<PathBuf, TacticQCampaignError> {
    if store
        .store
        .root()
        .file_name()
        .and_then(|name| name.to_str())
        != Some(CONTENT_DIRECTORY)
    {
        return Err(TacticQCampaignError::InvalidState(
            "checkpoint content root must use the discoverable objects directory",
        ));
    }
    fs::create_dir_all(directory).map_err(|error| TacticQCampaignError::Io(error.to_string()))?;
    let manifest = store_checkpoint_manifest(checkpoint, store)?;
    let raw = serde_cbor::to_vec(&manifest)
        .map_err(|error| TacticQCampaignError::Serialization(error.to_string()))?;
    let envelope = encode_checkpoint_envelope(&raw)?;
    let final_path = directory.join(format!(
        "tactic-q-{}.{}",
        checkpoint.content_sha256, TACTIC_Q_CHECKPOINT_EXTENSION
    ));
    install_binary_artifact(&final_path, &envelope)?;
    Ok(final_path)
}
