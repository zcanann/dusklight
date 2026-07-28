//! Manage workspace folder trees and recoverable folder trash.

use super::*;

impl WorkspaceStore {
    pub fn create(
        root: impl Into<PathBuf>,
        manifest: WorkspaceManifest,
    ) -> Result<Self, WorkspaceError> {
        manifest.validate()?;
        let root = root.into();
        if root.exists()
            && fs::read_dir(&root)
                .map_err(WorkspaceError::io)?
                .next()
                .is_some()
        {
            return Err(WorkspaceError::new(format!(
                "workspace folder {} is not empty",
                root.display()
            )));
        }
        fs::create_dir_all(&root).map_err(WorkspaceError::io)?;
        for asset_root in manifest.asset_roots.values() {
            fs::create_dir_all(root.join(asset_root)).map_err(WorkspaceError::io)?;
        }
        write_atomically(&root.join(MANIFEST_FILE), &manifest.canonical_bytes()?)?;
        let root = root.canonicalize().map_err(WorkspaceError::io)?;
        let store = Self { root, manifest };
        store.ensure_transaction_root()?;
        Ok(store)
    }

    pub fn open(
        root: impl Into<PathBuf>,
        available_libraries: &BTreeMap<(String, String), Digest>,
    ) -> Result<Self, WorkspaceError> {
        let root = root.into().canonicalize().map_err(WorkspaceError::io)?;
        let manifest = read_manifest_and_migrate(&root.join(MANIFEST_FILE))?;
        let issues = dependency_issues(&manifest, available_libraries);
        if !issues.is_empty() {
            return Err(WorkspaceError::new(format_dependency_issues(&issues)));
        }
        let store = Self { root, manifest };
        store.ensure_transaction_root()?;
        store.recover_transactions()?;
        store.recover_folder_operations()?;
        Ok(store)
    }

    pub fn manifest(&self) -> &WorkspaceManifest {
        &self.manifest
    }

    pub fn list_assets(&self) -> Result<Vec<WorkspaceAssetListing>, WorkspaceError> {
        let mut listings = Vec::new();
        let mut identities = BTreeMap::new();
        for kind in WorkspaceAssetKind::ALL {
            let root = self.asset_root(kind)?;
            collect_asset_files(&root, &mut |path| {
                let asset = read_asset(path)?;
                if asset.header.kind != kind {
                    return Err(WorkspaceError::new(format!(
                        "{} contains {:?} asset {} under the {:?} root",
                        path.display(),
                        asset.header.kind,
                        asset.header.id,
                        kind
                    )));
                }
                if let Some(first) = identities.insert(asset.header.id.clone(), path.to_path_buf())
                {
                    return Err(WorkspaceError::new(format!(
                        "asset identity {} is duplicated at {} and {}",
                        asset.header.id,
                        first.display(),
                        path.display()
                    )));
                }
                listings.push(WorkspaceAssetListing {
                    id: asset.header.id.clone(),
                    label: asset.header.label.clone(),
                    kind,
                    relative_path: path
                        .strip_prefix(&self.root)
                        .map_err(|_| WorkspaceError::new("asset escaped workspace root"))?
                        .to_path_buf(),
                    revision_sha256: asset.digest()?,
                });
                Ok(())
            })?;
        }
        listings.sort_by(|left, right| {
            left.kind
                .cmp(&right.kind)
                .then_with(|| left.label.cmp(&right.label))
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(listings)
    }

    pub fn list_folders(&self) -> Result<Vec<WorkspaceFolderListing>, WorkspaceError> {
        let mut folders = Vec::new();
        let mut identities = BTreeMap::new();
        let mut paths = BTreeMap::new();
        for kind in WorkspaceAssetKind::ALL {
            let root = self.asset_root(kind)?;
            collect_folder_markers(&root, &mut |directory, marker| {
                let folder = read_folder(marker)?;
                if folder.kind != kind {
                    return Err(WorkspaceError::new(format!(
                        "{} declares {:?} below the {:?} root",
                        marker.display(),
                        folder.kind,
                        kind
                    )));
                }
                let relative_path = directory
                    .strip_prefix(&self.root)
                    .map_err(|_| WorkspaceError::new("folder escaped workspace root"))?
                    .to_path_buf();
                if let Some(first) = identities.insert(folder.id.clone(), relative_path.clone()) {
                    return Err(WorkspaceError::new(format!(
                        "folder identity {} is duplicated at {} and {}",
                        folder.id,
                        first.display(),
                        relative_path.display()
                    )));
                }
                paths.insert(relative_path.clone(), folder.id.clone());
                let revision_sha256 = folder.digest()?;
                folders.push(WorkspaceFolderListing {
                    id: folder.id,
                    label: folder.label,
                    kind,
                    parent_id: folder.parent_id,
                    relative_path,
                    revision_sha256,
                });
                Ok(())
            })?;
        }
        for folder in &folders {
            let expected_parent_path = folder
                .relative_path
                .parent()
                .ok_or_else(|| WorkspaceError::new("folder has no parent directory"))?;
            let asset_root = Path::new(
                self.manifest
                    .asset_roots
                    .get(&folder.kind)
                    .expect("validated manifest has every root"),
            );
            match &folder.parent_id {
                Some(parent_id) => {
                    let actual_parent_id = paths.get(expected_parent_path).ok_or_else(|| {
                        WorkspaceError::new(format!(
                            "folder {} names parent {parent_id}, but {} has no folder marker",
                            folder.id,
                            expected_parent_path.display()
                        ))
                    })?;
                    if actual_parent_id != parent_id {
                        return Err(WorkspaceError::new(format!(
                            "folder {} names parent {parent_id}, but its containing folder is {actual_parent_id}",
                            folder.id
                        )));
                    }
                }
                None if expected_parent_path != asset_root => {
                    return Err(WorkspaceError::new(format!(
                        "folder {} has no parent identity but is not directly below its fixed root",
                        folder.id
                    )));
                }
                None => {}
            }
        }
        folders.sort_by(|left, right| {
            left.kind
                .cmp(&right.kind)
                .then_with(|| left.relative_path.cmp(&right.relative_path))
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(folders)
    }

    pub fn load_folder(&self, id: &str) -> Result<(WorkspaceFolder, PathBuf), WorkspaceError> {
        validate_stable_id("folder id", id)?;
        let listing = self
            .list_folders()?
            .into_iter()
            .find(|listing| listing.id == id)
            .ok_or_else(|| WorkspaceError::new(format!("folder {id} does not exist")))?;
        Ok((
            read_folder(
                &self
                    .root
                    .join(&listing.relative_path)
                    .join(FOLDER_MARKER_FILE),
            )?,
            listing.relative_path,
        ))
    }

    pub fn create_folder(
        &self,
        id: impl Into<String>,
        label: impl Into<String>,
        kind: WorkspaceAssetKind,
        parent_id: Option<&str>,
        directory_name: &str,
    ) -> Result<Digest, WorkspaceError> {
        let folder = WorkspaceFolder {
            schema: WORKSPACE_FOLDER_SCHEMA.into(),
            id: id.into(),
            label: label.into(),
            kind,
            parent_id: parent_id.map(str::to_owned),
        };
        folder.validate()?;
        if self
            .list_folders()?
            .iter()
            .any(|existing| existing.id == folder.id)
        {
            return Err(WorkspaceError::new(format!(
                "folder identity {} already exists",
                folder.id
            )));
        }
        validate_directory_name(directory_name)?;
        let parent = self.folder_parent_path(kind, parent_id)?;
        let destination = parent.join(directory_name);
        if destination.exists() {
            return Err(WorkspaceError::new(format!(
                "folder path {} already exists",
                destination.display()
            )));
        }
        let staging = self.root.join(FOLDER_STAGING_ROOT).join(format!(
            "folder-{}-{}",
            std::process::id(),
            NEXT_TEMPORARY_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&staging).map_err(WorkspaceError::io)?;
        let result = (|| {
            write_new_synced(
                &staging.join(FOLDER_MARKER_FILE),
                &folder.canonical_bytes()?,
            )?;
            fs::rename(&staging, &destination).map_err(WorkspaceError::io)
        })();
        if let Err(error) = result {
            if staging.is_dir() {
                let _ = fs::remove_dir_all(&staging);
            }
            return Err(error);
        }
        folder.digest()
    }

    pub fn rename_folder(
        &self,
        id: &str,
        label: impl Into<String>,
        directory_name: &str,
        expected_revision_sha256: Digest,
    ) -> Result<Digest, WorkspaceError> {
        let (mut folder, path) = self.load_folder(id)?;
        if folder.digest()? != expected_revision_sha256 {
            return Err(WorkspaceError::new(
                "folder revision conflict before rename",
            ));
        }
        folder.label = label.into();
        validate_directory_name(directory_name)?;
        let bytes = folder.canonical_bytes()?;
        let source = self.root.join(path);
        let destination = source
            .parent()
            .ok_or_else(|| WorkspaceError::new("folder path has no parent"))?
            .join(directory_name);
        if source == destination {
            write_atomically(&source.join(FOLDER_MARKER_FILE), &bytes)?;
        } else {
            if destination.exists() {
                return Err(WorkspaceError::new(format!(
                    "folder path {} already exists",
                    destination.display()
                )));
            }
            fs::rename(&source, &destination).map_err(WorkspaceError::io)?;
            if let Err(error) = write_atomically(&destination.join(FOLDER_MARKER_FILE), &bytes) {
                let _ = fs::rename(&destination, &source);
                return Err(error);
            }
        }
        folder.digest()
    }

    pub fn move_folder(
        &self,
        id: &str,
        parent_id: Option<&str>,
        expected_revision_sha256: Digest,
    ) -> Result<Digest, WorkspaceError> {
        let (mut folder, source_relative) = self.load_folder(id)?;
        if folder.digest()? != expected_revision_sha256 {
            return Err(WorkspaceError::new("folder revision conflict before move"));
        }
        if parent_id == Some(id) {
            return Err(WorkspaceError::new("folder cannot be moved into itself"));
        }
        let destination_parent = self.folder_parent_path(folder.kind, parent_id)?;
        let source = self.root.join(&source_relative);
        if destination_parent.starts_with(&source) {
            return Err(WorkspaceError::new(
                "folder cannot be moved into its own descendant",
            ));
        }
        let name = source
            .file_name()
            .ok_or_else(|| WorkspaceError::new("folder path has no directory name"))?;
        let destination = destination_parent.join(name);
        if destination == source {
            return folder.digest();
        }
        if destination.exists() {
            return Err(WorkspaceError::new(format!(
                "folder path {} already exists",
                destination.display()
            )));
        }
        fs::rename(&source, &destination).map_err(WorkspaceError::io)?;
        folder.parent_id = parent_id.map(str::to_owned);
        if let Err(error) = write_atomically(
            &destination.join(FOLDER_MARKER_FILE),
            &folder.canonical_bytes()?,
        ) {
            let _ = fs::rename(&destination, &source);
            return Err(error);
        }
        folder.digest()
    }

    pub fn duplicate_folder(
        &self,
        id: &str,
        new_id: impl Into<String>,
        new_label: impl Into<String>,
        parent_id: Option<&str>,
        directory_name: &str,
    ) -> Result<Digest, WorkspaceError> {
        let (source_folder, source_relative) = self.load_folder(id)?;
        let new_id = new_id.into();
        let new_label = new_label.into();
        validate_stable_id("duplicated folder id", &new_id)?;
        validate_label("duplicated folder label", &new_label)?;
        validate_directory_name(directory_name)?;
        if self
            .list_folders()?
            .iter()
            .any(|folder| folder.id == new_id)
        {
            return Err(WorkspaceError::new(format!(
                "folder identity {new_id} already exists"
            )));
        }
        let destination_parent = self.folder_parent_path(source_folder.kind, parent_id)?;
        let destination = destination_parent.join(directory_name);
        if destination.exists() {
            return Err(WorkspaceError::new(format!(
                "folder path {} already exists",
                destination.display()
            )));
        }
        validate_folder_subtree_files(&self.root.join(&source_relative))?;

        let source_folders = self
            .list_folders()?
            .into_iter()
            .filter(|folder| folder.relative_path.starts_with(&source_relative))
            .collect::<Vec<_>>();
        let source_assets = self
            .list_assets()?
            .into_iter()
            .filter(|asset| asset.relative_path.starts_with(&source_relative))
            .collect::<Vec<_>>();
        let existing_folder_ids = self
            .list_folders()?
            .into_iter()
            .map(|folder| folder.id)
            .collect::<BTreeSet<_>>();
        let existing_asset_ids = self
            .list_assets()?
            .into_iter()
            .map(|asset| asset.id)
            .collect::<BTreeSet<_>>();
        let folder_id_map = clone_identity_map(
            &source_folders
                .iter()
                .map(|folder| &folder.id)
                .collect::<Vec<_>>(),
            &new_id,
            "folder",
            &existing_folder_ids,
            Some(id),
        )?;
        let asset_id_map = clone_identity_map(
            &source_assets
                .iter()
                .map(|asset| &asset.id)
                .collect::<Vec<_>>(),
            &new_id,
            "asset",
            &existing_asset_ids,
            None,
        )?;
        let staging_root = self.root.join(FOLDER_STAGING_ROOT);
        fs::create_dir_all(&staging_root).map_err(WorkspaceError::io)?;
        let staging = staging_root.join(format!(
            "folder-{}-{}",
            std::process::id(),
            NEXT_TEMPORARY_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let mut cloned_assets = Vec::new();
        for listing in &source_assets {
            let (mut asset, _) = self.load_asset(&listing.id)?;
            remap_cloned_asset(&mut asset, &asset_id_map);
            cloned_assets.push((listing, asset));
        }
        let mut available = self
            .list_assets()?
            .into_iter()
            .map(|asset| (asset.id, asset.kind))
            .collect::<BTreeMap<_, _>>();
        available.extend(
            cloned_assets
                .iter()
                .map(|(_, asset)| (asset.header.id.clone(), asset.header.kind)),
        );
        for (_, asset) in &cloned_assets {
            validate_asset_references(asset, &available)?;
        }
        fs::create_dir(&staging).map_err(WorkspaceError::io)?;

        let build = (|| {
            for listing in &source_folders {
                let (mut folder, _) = self.load_folder(&listing.id)?;
                folder.id = folder_id_map
                    .get(&folder.id)
                    .expect("every cloned folder has an identity")
                    .clone();
                folder.label = if listing.id == id {
                    new_label.clone()
                } else {
                    folder.label
                };
                folder.parent_id = if listing.id == id {
                    parent_id.map(str::to_owned)
                } else {
                    folder
                        .parent_id
                        .as_ref()
                        .and_then(|parent| folder_id_map.get(parent))
                        .cloned()
                };
                let relative = listing
                    .relative_path
                    .strip_prefix(&source_relative)
                    .map_err(|_| WorkspaceError::new("cloned folder escaped source"))?;
                let target = staging.join(relative);
                fs::create_dir_all(&target).map_err(WorkspaceError::io)?;
                write_new_synced(&target.join(FOLDER_MARKER_FILE), &folder.canonical_bytes()?)?;
            }
            for (listing, asset) in &cloned_assets {
                let relative = listing
                    .relative_path
                    .strip_prefix(&source_relative)
                    .map_err(|_| WorkspaceError::new("cloned asset escaped source"))?;
                let target = staging.join(relative);
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent).map_err(WorkspaceError::io)?;
                }
                write_new_synced(&target, &asset.canonical_bytes()?)?;
            }
            fs::rename(&staging, &destination).map_err(WorkspaceError::io)
        })();
        if let Err(error) = build {
            if staging.is_dir() {
                let _ = fs::remove_dir_all(&staging);
            }
            return Err(error);
        }
        let (folder, _) = self.load_folder(&new_id)?;
        folder.digest()
    }

    pub fn delete_folder_to_trash(
        &self,
        id: &str,
        expected_revision_sha256: Digest,
        allow_broken_references: bool,
    ) -> Result<(), WorkspaceError> {
        let (folder, source_relative) = self.load_folder(id)?;
        if folder.digest()? != expected_revision_sha256 {
            return Err(WorkspaceError::new(
                "folder revision conflict before delete",
            ));
        }
        let subtree_assets = self
            .list_assets()?
            .into_iter()
            .filter(|asset| asset.relative_path.starts_with(&source_relative))
            .collect::<Vec<_>>();
        let subtree_ids = subtree_assets
            .iter()
            .map(|asset| asset.id.as_str())
            .collect::<BTreeSet<_>>();
        let mut external_inbound = BTreeSet::new();
        for listing in self.list_assets()? {
            if subtree_ids.contains(listing.id.as_str()) {
                continue;
            }
            let asset = read_asset(&self.root.join(&listing.relative_path))?;
            if asset
                .references
                .iter()
                .any(|reference| subtree_ids.contains(reference.asset_id.as_str()))
            {
                external_inbound.insert(listing.id);
            }
        }
        if !allow_broken_references && !external_inbound.is_empty() {
            return Err(WorkspaceError::new(format!(
                "folder {id} contains assets referenced by {}; confirm deletion to preserve these as broken stable-ID references",
                external_inbound.into_iter().collect::<Vec<_>>().join(", ")
            )));
        }
        let folder_count = self
            .list_folders()?
            .into_iter()
            .filter(|child| child.relative_path.starts_with(&source_relative))
            .count();
        let group = self.folder_trash_group(id)?;
        if group.exists() {
            return Err(WorkspaceError::new(format!(
                "folder {id} already has a grouped Trash entry"
            )));
        }
        fs::create_dir(&group).map_err(WorkspaceError::io)?;
        let record = WorkspaceFolderTrashRecord {
            schema: FOLDER_TRASH_RECORD_SCHEMA.into(),
            folder,
            original_relative_path: source_relative.clone(),
            revision_sha256: expected_revision_sha256,
            folder_count,
            asset_count: subtree_assets.len(),
        };
        if let Err(error) = write_new_synced(
            &group.join(FOLDER_TRASH_RECORD_FILE),
            &canonical_json(&record)?,
        ) {
            let _ = fs::remove_dir(&group);
            return Err(error);
        }
        if let Err(error) = fs::rename(
            self.root.join(&source_relative),
            group.join(FOLDER_TRASH_PAYLOAD),
        ) {
            let _ = fs::remove_file(group.join(FOLDER_TRASH_RECORD_FILE));
            let _ = fs::remove_dir(&group);
            return Err(WorkspaceError::io(error));
        }
        Ok(())
    }

    pub fn list_folder_trash(&self) -> Result<Vec<WorkspaceFolderTrashListing>, WorkspaceError> {
        let root = self.root.join(FOLDER_TRASH_ROOT);
        let mut trash = Vec::new();
        for entry in fs::read_dir(&root).map_err(WorkspaceError::io)? {
            let entry = entry.map_err(WorkspaceError::io)?;
            if !entry.file_type().map_err(WorkspaceError::io)?.is_dir() {
                return Err(WorkspaceError::new(format!(
                    "unexpected file in grouped folder Trash: {}",
                    entry.path().display()
                )));
            }
            let record = self.read_folder_trash_record(&entry.path())?;
            if entry.file_name().to_string_lossy() != folder_trash_directory_name(&record.folder.id)
                || !entry.path().join(FOLDER_TRASH_PAYLOAD).is_dir()
            {
                return Err(WorkspaceError::new(format!(
                    "grouped folder Trash entry {} is invalid",
                    entry.path().display()
                )));
            }
            trash.push(WorkspaceFolderTrashListing {
                id: record.folder.id,
                label: record.folder.label,
                kind: record.folder.kind,
                original_relative_path: record.original_relative_path,
                revision_sha256: record.revision_sha256,
                folder_count: record.folder_count,
                asset_count: record.asset_count,
            });
        }
        trash.sort_by(|left, right| {
            left.kind
                .cmp(&right.kind)
                .then_with(|| left.label.cmp(&right.label))
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(trash)
    }

    pub fn restore_folder_from_trash(
        &self,
        id: &str,
        expected_revision_sha256: Digest,
    ) -> Result<(), WorkspaceError> {
        let group = self.folder_trash_group(id)?;
        let record = self.read_folder_trash_record(&group)?;
        if record.folder.id != id || record.revision_sha256 != expected_revision_sha256 {
            return Err(WorkspaceError::new(
                "folder Trash revision conflict before restore",
            ));
        }
        let destination = self.folder_restore_destination(&record)?;
        if destination.exists() {
            return Err(WorkspaceError::new(format!(
                "cannot restore folder {id}; {} already exists",
                destination
                    .strip_prefix(&self.root)
                    .unwrap_or(&destination)
                    .display()
            )));
        }
        fs::rename(group.join(FOLDER_TRASH_PAYLOAD), &destination).map_err(WorkspaceError::io)?;
        fs::remove_file(group.join(FOLDER_TRASH_RECORD_FILE)).map_err(WorkspaceError::io)?;
        fs::remove_dir(group).map_err(WorkspaceError::io)
    }

    pub fn permanently_delete_folder_from_trash(
        &self,
        id: &str,
        expected_revision_sha256: Digest,
    ) -> Result<(), WorkspaceError> {
        let group = self.folder_trash_group(id)?;
        let record = self.read_folder_trash_record(&group)?;
        if record.folder.id != id || record.revision_sha256 != expected_revision_sha256 {
            return Err(WorkspaceError::new(
                "folder Trash revision conflict before permanent delete",
            ));
        }
        let trash_root = self.root.join(FOLDER_TRASH_ROOT);
        if group.parent() != Some(trash_root.as_path()) || !group.is_dir() {
            return Err(WorkspaceError::new(
                "refusing to remove an invalid grouped folder Trash path",
            ));
        }
        fs::remove_dir_all(group).map_err(WorkspaceError::io)
    }

    pub(super) fn folder_parent_path(
        &self,
        kind: WorkspaceAssetKind,
        parent_id: Option<&str>,
    ) -> Result<PathBuf, WorkspaceError> {
        match parent_id {
            Some(id) => {
                let (parent, path) = self.load_folder(id)?;
                if parent.kind != kind {
                    return Err(WorkspaceError::new(format!(
                        "{kind:?} folder cannot be placed below {:?} folder {id}",
                        parent.kind
                    )));
                }
                Ok(self.root.join(path))
            }
            None => self.asset_root(kind),
        }
    }

    pub(super) fn folder_trash_group(&self, id: &str) -> Result<PathBuf, WorkspaceError> {
        validate_stable_id("folder id", id)?;
        Ok(self
            .root
            .join(FOLDER_TRASH_ROOT)
            .join(folder_trash_directory_name(id)))
    }

    pub(super) fn read_folder_trash_record(
        &self,
        group: &Path,
    ) -> Result<WorkspaceFolderTrashRecord, WorkspaceError> {
        let record = read_folder_trash_record(group)?;
        validate_folder_relative_path(
            &self.manifest,
            record.folder.kind,
            &record.original_relative_path,
        )?;
        Ok(record)
    }

    pub(super) fn folder_restore_destination(
        &self,
        record: &WorkspaceFolderTrashRecord,
    ) -> Result<PathBuf, WorkspaceError> {
        let directory_name = record
            .original_relative_path
            .file_name()
            .ok_or_else(|| WorkspaceError::new("trashed folder path has no directory name"))?;
        Ok(self
            .folder_parent_path(record.folder.kind, record.folder.parent_id.as_deref())?
            .join(directory_name))
    }

    pub(super) fn import_folders(
        &self,
        folders: &[WorkspaceExportFolder],
    ) -> Result<(), WorkspaceError> {
        let mut sorted = folders.to_vec();
        sorted.sort_by_key(|folder| folder.relative_path.components().count());
        for record in sorted {
            let path = self.root.join(&record.relative_path);
            fs::create_dir_all(&path).map_err(WorkspaceError::io)?;
            write_new_synced(
                &path.join(FOLDER_MARKER_FILE),
                &record.folder.canonical_bytes()?,
            )?;
        }
        self.list_folders().map(|_| ())
    }
}
