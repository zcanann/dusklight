//! Validate references and paths, clone identities, and perform durable I/O.

use super::*;

pub(super) fn validate_asset_references(
    asset: &WorkspaceAsset,
    available: &BTreeMap<String, WorkspaceAssetKind>,
) -> Result<(), WorkspaceError> {
    for reference in &asset.references {
        let Some(actual_kind) = available.get(&reference.asset_id) else {
            return Err(WorkspaceError::new(format!(
                "{} references missing {:?} asset {}",
                asset.header.id, reference.kind, reference.asset_id
            )));
        };
        if *actual_kind != reference.kind {
            return Err(WorkspaceError::new(format!(
                "{} references {} as {:?}, but it is {:?}",
                asset.header.id, reference.asset_id, reference.kind, actual_kind
            )));
        }
    }
    Ok(())
}

pub fn dependency_issues(
    manifest: &WorkspaceManifest,
    available: &BTreeMap<(String, String), Digest>,
) -> Vec<LibraryDependencyIssue> {
    manifest
        .mounted_libraries
        .iter()
        .filter_map(|pin| {
            let key = (pin.id.clone(), pin.version.clone());
            match available.get(&key) {
                None => Some(LibraryDependencyIssue::Missing {
                    id: pin.id.clone(),
                    version: pin.version.clone(),
                    expected_sha256: pin.sha256,
                    source: pin.source.clone(),
                }),
                Some(actual) if *actual != pin.sha256 => Some(LibraryDependencyIssue::Changed {
                    id: pin.id.clone(),
                    version: pin.version.clone(),
                    expected_sha256: pin.sha256,
                    actual_sha256: *actual,
                    source: pin.source.clone(),
                }),
                Some(_) => None,
            }
        })
        .collect()
}

pub(super) fn format_dependency_issues(issues: &[LibraryDependencyIssue]) -> String {
    let details = issues
        .iter()
        .map(|issue| match issue {
            LibraryDependencyIssue::Missing {
                id,
                version,
                expected_sha256,
                source,
            } => format!(
                "{id} {version} is missing (expected {expected_sha256}; restore from {source})"
            ),
            LibraryDependencyIssue::Changed {
                id,
                version,
                expected_sha256,
                actual_sha256,
                source,
            } => format!(
                "{id} {version} changed (expected {expected_sha256}, found {actual_sha256}; restore the pinned library from {source} or explicitly upgrade the workspace)"
            ),
        })
        .collect::<Vec<_>>()
        .join("; ");
    format!("workspace library dependencies are not satisfied: {details}")
}

pub(super) fn read_manifest_and_migrate(path: &Path) -> Result<WorkspaceManifest, WorkspaceError> {
    let bytes = fs::read(path).map_err(WorkspaceError::io)?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(WorkspaceError::json)?;
    let schema = value
        .get("schema")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| WorkspaceError::new("workspace manifest has no schema"))?;
    let manifest = match schema {
        WORKSPACE_MANIFEST_SCHEMA => serde_json::from_value(value).map_err(WorkspaceError::json)?,
        LEGACY_WORKSPACE_MANIFEST_SCHEMA => {
            let legacy: LegacyWorkspaceManifest =
                serde_json::from_value(value).map_err(WorkspaceError::json)?;
            if legacy.schema != LEGACY_WORKSPACE_MANIFEST_SCHEMA {
                return Err(WorkspaceError::new("legacy workspace schema is invalid"));
            }
            let migrated = WorkspaceManifest {
                schema: WORKSPACE_MANIFEST_SCHEMA.into(),
                format_version: WORKSPACE_FORMAT_VERSION,
                id: legacy.id,
                label: legacy.label,
                mounted_libraries: legacy.mounted_libraries,
                exact_context_defaults: legacy.exact_context_defaults,
                asset_roots: legacy.asset_roots,
            };
            migrated.validate()?;
            write_atomically(path, &migrated.canonical_bytes()?)?;
            migrated
        }
        _ => {
            return Err(WorkspaceError::new(format!(
                "workspace schema {schema} is unsupported; migrate it with a compatible application version"
            )));
        }
    };
    manifest.validate()?;
    Ok(manifest)
}

pub(super) fn read_asset(path: &Path) -> Result<WorkspaceAsset, WorkspaceError> {
    let bytes = fs::read(path).map_err(WorkspaceError::io)?;
    let asset: WorkspaceAsset = serde_json::from_slice(&bytes).map_err(WorkspaceError::json)?;
    asset.validate()?;
    Ok(asset)
}

pub(super) fn read_folder(path: &Path) -> Result<WorkspaceFolder, WorkspaceError> {
    let bytes = fs::read(path).map_err(WorkspaceError::io)?;
    let folder: WorkspaceFolder = serde_json::from_slice(&bytes).map_err(WorkspaceError::json)?;
    folder.validate()?;
    Ok(folder)
}

pub(super) fn read_folder_trash_record(
    group: &Path,
) -> Result<WorkspaceFolderTrashRecord, WorkspaceError> {
    let bytes = fs::read(group.join(FOLDER_TRASH_RECORD_FILE)).map_err(WorkspaceError::io)?;
    let record: WorkspaceFolderTrashRecord =
        serde_json::from_slice(&bytes).map_err(WorkspaceError::json)?;
    if record.schema != FOLDER_TRASH_RECORD_SCHEMA
        || record.folder.digest()? != record.revision_sha256
        || record.folder_count == 0
    {
        return Err(WorkspaceError::new(format!(
            "grouped folder Trash record {} is invalid",
            group.display()
        )));
    }
    validate_relative_path(
        "grouped folder Trash original path",
        &record.original_relative_path,
    )?;
    Ok(record)
}

pub(super) fn collect_folder_markers(
    root: &Path,
    visit: &mut impl FnMut(&Path, &Path) -> Result<(), WorkspaceError>,
) -> Result<(), WorkspaceError> {
    if !root.is_dir() {
        return Err(WorkspaceError::new(format!(
            "asset root {} does not exist",
            root.display()
        )));
    }
    let marker = root.join(FOLDER_MARKER_FILE);
    if marker.is_file() {
        visit(root, &marker)?;
    }
    for entry in fs::read_dir(root).map_err(WorkspaceError::io)? {
        let entry = entry.map_err(WorkspaceError::io)?;
        if entry.file_type().map_err(WorkspaceError::io)?.is_dir() {
            collect_folder_markers(&entry.path(), visit)?;
        }
    }
    Ok(())
}

pub(super) fn validate_folder_subtree_files(root: &Path) -> Result<(), WorkspaceError> {
    for entry in fs::read_dir(root).map_err(WorkspaceError::io)? {
        let entry = entry.map_err(WorkspaceError::io)?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(WorkspaceError::io)?;
        if file_type.is_dir() {
            validate_folder_subtree_files(&path)?;
        } else if !file_type.is_file()
            || (path.file_name().and_then(|name| name.to_str()) != Some(FOLDER_MARKER_FILE)
                && path.extension().and_then(|extension| extension.to_str()) != Some("json"))
        {
            return Err(WorkspaceError::new(format!(
                "folder subtree contains unsupported file {}; only typed workspace JSON files can be duplicated",
                path.display()
            )));
        }
    }
    Ok(())
}

pub(super) fn validate_folder_relative_path(
    manifest: &WorkspaceManifest,
    kind: WorkspaceAssetKind,
    relative_path: &Path,
) -> Result<(), WorkspaceError> {
    validate_relative_path("workspace folder path", relative_path)?;
    let root = manifest
        .asset_roots
        .get(&kind)
        .expect("validated manifest has every asset root");
    if relative_path == Path::new(root) || !relative_path.starts_with(root) {
        return Err(WorkspaceError::new(format!(
            "{kind:?} folder must be below the fixed {root} root"
        )));
    }
    Ok(())
}

pub(super) fn validate_export_folder_hierarchy(
    manifest: &WorkspaceManifest,
    folders: &[WorkspaceExportFolder],
) -> Result<(), WorkspaceError> {
    let by_id = folders
        .iter()
        .map(|folder| (folder.folder.id.as_str(), folder))
        .collect::<BTreeMap<_, _>>();
    for record in folders {
        let expected_parent = record
            .relative_path
            .parent()
            .ok_or_else(|| WorkspaceError::new("import folder has no parent path"))?;
        match &record.folder.parent_id {
            Some(parent_id) => {
                let parent = by_id.get(parent_id.as_str()).ok_or_else(|| {
                    WorkspaceError::new(format!(
                        "import folder {} references missing parent {parent_id}",
                        record.folder.id
                    ))
                })?;
                if parent.folder.kind != record.folder.kind
                    || parent.relative_path != expected_parent
                {
                    return Err(WorkspaceError::new(format!(
                        "import folder {} parent identity does not match its path",
                        record.folder.id
                    )));
                }
            }
            None => {
                let root = manifest
                    .asset_roots
                    .get(&record.folder.kind)
                    .expect("validated manifest has every asset root");
                if expected_parent != Path::new(root) {
                    return Err(WorkspaceError::new(format!(
                        "import folder {} without a parent must be directly below {root}",
                        record.folder.id
                    )));
                }
            }
        }
    }
    Ok(())
}

pub(super) fn validate_directory_name(value: &str) -> Result<(), WorkspaceError> {
    let path = Path::new(value);
    if value.is_empty()
        || value.len() > 128
        || path.components().count() != 1
        || !matches!(path.components().next(), Some(Component::Normal(_)))
        || value.chars().any(char::is_control)
        || matches!(value, "." | "..")
    {
        return Err(WorkspaceError::new(
            "folder directory name must be one safe printable path segment",
        ));
    }
    Ok(())
}

pub(super) fn clone_identity_map(
    source_ids: &[&String],
    namespace: &str,
    category: &str,
    existing: &BTreeSet<String>,
    root_id: Option<&str>,
) -> Result<BTreeMap<String, String>, WorkspaceError> {
    let mut mapping = BTreeMap::new();
    let mut allocated = existing.clone();
    let mut index = 0_usize;
    for source in source_ids {
        let candidate = if root_id == Some(source.as_str()) {
            namespace.to_owned()
        } else {
            loop {
                let candidate = format!("{namespace}.{category}-{index:03}");
                index += 1;
                if !allocated.contains(&candidate) {
                    break candidate;
                }
            }
        };
        validate_stable_id("duplicated stable identity", &candidate)?;
        if !allocated.insert(candidate.clone()) {
            return Err(WorkspaceError::new(format!(
                "duplicated stable identity {candidate} collides"
            )));
        }
        mapping.insert((*source).clone(), candidate);
    }
    Ok(mapping)
}

pub(super) fn remap_cloned_asset(asset: &mut WorkspaceAsset, mapping: &BTreeMap<String, String>) {
    asset.header.id = mapping
        .get(&asset.header.id)
        .expect("every cloned asset has a new identity")
        .clone();
    asset.header.version = 1;
    for reference in &mut asset.references {
        if let Some(remapped) = mapping.get(&reference.asset_id) {
            reference.asset_id = remapped.clone();
        }
    }
    match &mut asset.payload {
        WorkspaceAssetPayload::Scenario(scenario) => {
            if let Some(remapped) = mapping.get(&scenario.route_graph_id) {
                scenario.route_graph_id = remapped.clone();
            }
            if let Some(id) = &mut scenario.state_seed_id
                && let Some(remapped) = mapping.get(id)
            {
                *id = remapped.clone();
            }
            if let Some(id) = &mut scenario.route_book_id
                && let Some(remapped) = mapping.get(id)
            {
                *id = remapped.clone();
            }
            if let ScenarioAnchor::StateSeed { state_seed_id } = &mut scenario.anchor
                && let Some(remapped) = mapping.get(state_seed_id)
            {
                *state_seed_id = remapped.clone();
            }
        }
        WorkspaceAssetPayload::Layout(layout) => {
            if let Some(remapped) = mapping.get(&layout.semantic_asset_id) {
                layout.semantic_asset_id = remapped.clone();
            }
        }
        WorkspaceAssetPayload::RouteGraph { .. }
        | WorkspaceAssetPayload::ReusableSubgraph { .. }
        | WorkspaceAssetPayload::CustomNodeDefinition(_)
        | WorkspaceAssetPayload::StateSeed { .. }
        | WorkspaceAssetPayload::QueryGoal(_)
        | WorkspaceAssetPayload::RouteBook { .. } => {}
    }
}

pub(super) fn collect_asset_files(
    root: &Path,
    visit: &mut impl FnMut(&Path) -> Result<(), WorkspaceError>,
) -> Result<(), WorkspaceError> {
    if !root.is_dir() {
        return Err(WorkspaceError::new(format!(
            "asset root {} does not exist",
            root.display()
        )));
    }
    for entry in fs::read_dir(root).map_err(WorkspaceError::io)? {
        let entry = entry.map_err(WorkspaceError::io)?;
        let path = entry.path();
        if entry.file_type().map_err(WorkspaceError::io)?.is_dir() {
            collect_asset_files(&path, visit)?;
        } else if path.file_name().and_then(|value| value.to_str()) != Some(FOLDER_MARKER_FILE)
            && path.extension().and_then(|value| value.to_str()) == Some("json")
        {
            visit(&path)?;
        }
    }
    Ok(())
}

pub(super) fn validate_pins(field: &str, pins: &[CustomNodePin]) -> Result<(), WorkspaceError> {
    let mut ids = BTreeSet::new();
    for pin in pins {
        validate_stable_id(field, &pin.id)?;
        validate_label(field, &pin.label)?;
        validate_stable_id(field, &pin.value_type)?;
        if !ids.insert(&pin.id) {
            return Err(WorkspaceError::new(format!(
                "{field} contains duplicate {}",
                pin.id
            )));
        }
    }
    Ok(())
}

pub(super) fn validate_stable_id(field: &str, value: &str) -> Result<(), WorkspaceError> {
    if value.is_empty()
        || value.len() > 128
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'.' | b'_' | b'-' | b'/' | b':')
        })
    {
        return Err(WorkspaceError::new(format!(
            "{field} must use 1-128 lowercase ASCII letters, digits, '.', '_', '-', '/', or ':'"
        )));
    }
    Ok(())
}

pub(super) fn stable_fragment(value: &str) -> String {
    let fragment = value
        .chars()
        .map(|character| {
            if character.is_ascii_lowercase() || character.is_ascii_digit() {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    fragment.trim_matches('-').to_owned()
}

pub(super) fn folder_trash_directory_name(id: &str) -> String {
    format!("folder-{}", Digest(Sha256::digest(id.as_bytes()).into()))
}

pub(super) fn validate_label(field: &str, value: &str) -> Result<(), WorkspaceError> {
    if value.trim().is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        return Err(WorkspaceError::new(format!(
            "{field} must be nonempty printable text of at most 256 characters"
        )));
    }
    Ok(())
}

pub(super) fn validate_relative_path(field: &str, path: &Path) -> Result<(), WorkspaceError> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            !matches!(component, Component::Normal(_))
                || component
                    .as_os_str()
                    .to_str()
                    .is_none_or(|value| value.is_empty() || value.chars().any(char::is_control))
        })
    {
        return Err(WorkspaceError::new(format!(
            "{field} must be a nonempty relative path without traversal"
        )));
    }
    Ok(())
}

pub(super) fn validate_trash_path(path: &Path) -> Result<(), WorkspaceError> {
    validate_relative_path("trash path", path)?;
    if !path.starts_with(TRASH_ROOT)
        || path == Path::new(TRASH_ROOT)
        || path.extension().and_then(|value| value.to_str()) != Some("json")
    {
        return Err(WorkspaceError::new(
            "trash asset path must be a JSON file below the workspace trash root",
        ));
    }
    Ok(())
}

pub(super) fn canonical_json<T: Serialize>(value: &T) -> Result<Vec<u8>, WorkspaceError> {
    let mut bytes = serde_json::to_vec(value).map_err(WorkspaceError::json)?;
    bytes.push(b'\n');
    Ok(bytes)
}

pub(super) fn path_to_slashes(path: &Path) -> Result<String, WorkspaceError> {
    validate_relative_path("asset path", path)?;
    Ok(path
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/"))
}

pub(super) fn write_new_synced(path: &Path, bytes: &[u8]) -> Result<(), WorkspaceError> {
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(WorkspaceError::io)?;
    output.write_all(bytes).map_err(WorkspaceError::io)?;
    output.sync_all().map_err(WorkspaceError::io)
}

pub(super) fn remove_transaction_directory(
    root: &Path,
    transaction: &Path,
) -> Result<(), WorkspaceError> {
    let expected_parent = root.join(TRANSACTION_ROOT);
    if transaction.parent() != Some(expected_parent.as_path())
        || transaction.file_name().is_none()
        || !transaction.is_dir()
    {
        return Err(WorkspaceError::new(
            "refusing to remove an invalid transaction directory",
        ));
    }
    fs::remove_dir_all(transaction).map_err(WorkspaceError::io)
}

pub(super) fn display_digest(value: Option<Digest>) -> String {
    value
        .map(|digest| digest.to_string())
        .unwrap_or_else(|| "none".into())
}

pub(super) fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), WorkspaceError> {
    let temporary = path.with_extension(format!(
        "tmp-{}-{}",
        std::process::id(),
        NEXT_TEMPORARY_ID.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(WorkspaceError::io)?;
        output.write_all(bytes).map_err(WorkspaceError::io)?;
        output.sync_all().map_err(WorkspaceError::io)?;
        drop(output);
        replace_file(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(not(windows))]
pub(super) fn replace_file(source: &Path, destination: &Path) -> Result<(), WorkspaceError> {
    fs::rename(source, destination).map_err(WorkspaceError::io)
}

#[cfg(windows)]
pub(super) fn replace_file(source: &Path, destination: &Path) -> Result<(), WorkspaceError> {
    use std::os::windows::ffi::OsStrExt;
    const MOVEFILE_REPLACE_EXISTING: u32 = 0x0000_0001;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x0000_0008;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(existing: *const u16, replacement: *const u16, flags: u32) -> i32;
    }
    let source = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        Err(WorkspaceError::io(std::io::Error::last_os_error()))
    } else {
        Ok(())
    }
}
