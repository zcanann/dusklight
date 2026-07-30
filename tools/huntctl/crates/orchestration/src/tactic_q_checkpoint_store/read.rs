use super::*;

#[derive(Deserialize)]
struct StoredCheckpointManifestSchema {
    schema: String,
}

pub(crate) fn read_checkpoint(
    path: &Path,
) -> Result<TacticQCampaignCheckpoint, TacticQCampaignError> {
    let metadata =
        fs::symlink_metadata(path).map_err(|error| TacticQCampaignError::Io(error.to_string()))?;
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() < CHECKPOINT_HEADER_SIZE as u64
        || metadata.len() > MAXIMUM_CHECKPOINT_MANIFEST_BYTES + CHECKPOINT_HEADER_SIZE as u64
    {
        return Err(TacticQCampaignError::InvalidState(
            "checkpoint path is not a bounded physical binary envelope",
        ));
    }
    let raw = decode_checkpoint_envelope(
        &fs::read(path).map_err(|error| TacticQCampaignError::Io(error.to_string()))?,
    )?;
    let manifest_schema: StoredCheckpointManifestSchema =
        decode_cbor(&raw).map_err(checkpoint_store_error)?;
    if manifest_schema.schema == v6::manifest_schema() {
        let manifest = v6::decode_manifest(&raw)?;
        let parent = path.parent().ok_or(TacticQCampaignError::InvalidState(
            "checkpoint path has no parent",
        ))?;
        for ancestor in parent.ancestors() {
            let content_root = ancestor.join(CONTENT_DIRECTORY);
            let Ok(store) = TacticQContentStore::open(&content_root) else {
                continue;
            };
            if let Ok(checkpoint) = v6::load_checkpoint(&manifest, &store) {
                return Ok(checkpoint);
            }
        }
        return Err(TacticQCampaignError::InvalidState(
            "checkpoint v6 content objects are unavailable or invalid",
        ));
    }
    let manifest: StoredCheckpointManifest = decode_cbor(&raw).map_err(checkpoint_store_error)?;
    let parent = path.parent().ok_or(TacticQCampaignError::InvalidState(
        "checkpoint path has no parent",
    ))?;
    for ancestor in parent.ancestors() {
        let content_root = ancestor.join(CONTENT_DIRECTORY);
        let Ok(store) = TacticQContentStore::open(&content_root) else {
            continue;
        };
        if let Ok(checkpoint) = load_checkpoint_manifest(&manifest, &store) {
            validate_checkpoint(&checkpoint)?;
            return Ok(checkpoint);
        }
    }
    Err(TacticQCampaignError::InvalidState(
        "checkpoint content objects are unavailable or invalid",
    ))
}
