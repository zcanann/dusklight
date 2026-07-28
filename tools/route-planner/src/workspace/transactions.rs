//! Journal, recover, validate, and atomically apply workspace mutations.

use super::*;

impl WorkspaceStore {
    pub(super) fn ensure_transaction_root(&self) -> Result<(), WorkspaceError> {
        fs::create_dir_all(self.root.join(TRANSACTION_ROOT)).map_err(WorkspaceError::io)?;
        fs::create_dir_all(self.root.join(TRASH_ROOT)).map_err(WorkspaceError::io)?;
        fs::create_dir_all(self.root.join(FOLDER_TRASH_ROOT)).map_err(WorkspaceError::io)?;
        fs::create_dir_all(self.root.join(FOLDER_STAGING_ROOT)).map_err(WorkspaceError::io)
    }

    pub(super) fn recover_transactions(&self) -> Result<(), WorkspaceError> {
        let root = self.root.join(TRANSACTION_ROOT);
        let mut transactions = fs::read_dir(&root)
            .map_err(WorkspaceError::io)?
            .map(|entry| entry.map(|entry| entry.path()).map_err(WorkspaceError::io))
            .collect::<Result<Vec<_>, _>>()?;
        transactions.sort();
        for transaction_root in transactions {
            if !transaction_root.is_dir() {
                return Err(WorkspaceError::new(format!(
                    "unexpected file in transaction journal: {}",
                    transaction_root.display()
                )));
            }
            let bytes =
                fs::read(transaction_root.join("transaction.json")).map_err(WorkspaceError::io)?;
            let journal: WorkspaceTransactionJournal =
                serde_json::from_slice(&bytes).map_err(WorkspaceError::json)?;
            if journal.schema != TRANSACTION_SCHEMA
                || transaction_root.file_name().and_then(|name| name.to_str())
                    != Some(journal.id.as_str())
            {
                return Err(WorkspaceError::new(format!(
                    "workspace transaction journal {} is invalid",
                    transaction_root.display()
                )));
            }
            self.apply_transaction(&transaction_root, &journal)?;
            remove_transaction_directory(&self.root, &transaction_root)?;
        }
        Ok(())
    }

    pub(super) fn recover_folder_operations(&self) -> Result<(), WorkspaceError> {
        let trash_root = self.root.join(FOLDER_TRASH_ROOT);
        for entry in fs::read_dir(&trash_root).map_err(WorkspaceError::io)? {
            let entry = entry.map_err(WorkspaceError::io)?;
            if !entry.file_type().map_err(WorkspaceError::io)?.is_dir()
                || !entry.file_name().to_string_lossy().starts_with("folder-")
            {
                return Err(WorkspaceError::new(format!(
                    "unexpected entry in grouped folder Trash: {}",
                    entry.path().display()
                )));
            }
            let group = entry.path();
            let record = self.read_folder_trash_record(&group)?;
            let payload = group.join(FOLDER_TRASH_PAYLOAD);
            let live_exists = self
                .list_folders()?
                .iter()
                .any(|folder| folder.id == record.folder.id);
            match (payload.is_dir(), live_exists) {
                (true, false) => {}
                (false, true) => {
                    fs::remove_file(group.join(FOLDER_TRASH_RECORD_FILE))
                        .map_err(WorkspaceError::io)?;
                    fs::remove_dir(&group).map_err(WorkspaceError::io)?;
                }
                (true, true) => {
                    return Err(WorkspaceError::new(format!(
                        "grouped folder Trash recovery found both live and trashed copies of {}",
                        record.folder.id
                    )));
                }
                (false, false) => {
                    return Err(WorkspaceError::new(format!(
                        "grouped folder Trash recovery lost both copies of {}",
                        record.folder.id
                    )));
                }
            }
        }

        let staging_root = self.root.join(FOLDER_STAGING_ROOT);
        for entry in fs::read_dir(&staging_root).map_err(WorkspaceError::io)? {
            let entry = entry.map_err(WorkspaceError::io)?;
            if !entry.file_type().map_err(WorkspaceError::io)?.is_dir()
                || !entry.file_name().to_string_lossy().starts_with("folder-")
            {
                return Err(WorkspaceError::new(format!(
                    "unexpected entry in folder staging: {}",
                    entry.path().display()
                )));
            }
            let staging = entry.path();
            if staging.parent() != Some(staging_root.as_path()) {
                return Err(WorkspaceError::new(
                    "refusing to remove invalid folder staging path",
                ));
            }
            fs::remove_dir_all(staging).map_err(WorkspaceError::io)?;
        }
        Ok(())
    }

    pub(super) fn validate_mutations(
        &self,
        mutations: &[WorkspaceMutation],
    ) -> Result<(), WorkspaceError> {
        let listings = self.list_assets()?;
        let mut paths = BTreeSet::new();
        let deleted_paths = mutations
            .iter()
            .filter_map(|mutation| match mutation {
                WorkspaceMutation::Delete { relative_path, .. } => Some(relative_path),
                WorkspaceMutation::Put { .. } | WorkspaceMutation::Archive { .. } => None,
            })
            .collect::<BTreeSet<_>>();
        let mut available = listings
            .iter()
            .filter(|listing| !deleted_paths.contains(&listing.relative_path))
            .map(|listing| (listing.id.clone(), listing.kind))
            .collect::<BTreeMap<_, _>>();
        let mut put_ids = BTreeSet::new();
        for mutation in mutations {
            let (relative_path, expected_revision) = match mutation {
                WorkspaceMutation::Put {
                    relative_path,
                    expected_revision_sha256,
                    asset,
                } => {
                    asset.validate()?;
                    self.validate_asset_path(relative_path, asset.header.kind)?;
                    if !put_ids.insert(&asset.header.id) {
                        return Err(WorkspaceError::new(format!(
                            "transaction writes asset identity {} more than once",
                            asset.header.id
                        )));
                    }
                    available.insert(asset.header.id.clone(), asset.header.kind);
                    if let Some(existing) = listings
                        .iter()
                        .find(|listing| listing.id == asset.header.id)
                        && existing.relative_path != *relative_path
                        && !deleted_paths.contains(&existing.relative_path)
                    {
                        return Err(WorkspaceError::new(format!(
                            "asset identity {} already exists at another path",
                            asset.header.id
                        )));
                    }
                    (relative_path, *expected_revision_sha256)
                }
                WorkspaceMutation::Delete {
                    relative_path,
                    expected_revision_sha256,
                } => {
                    validate_relative_path("asset path", relative_path)?;
                    (relative_path, Some(*expected_revision_sha256))
                }
                WorkspaceMutation::Archive {
                    trash_relative_path,
                    asset,
                } => {
                    asset.validate()?;
                    validate_trash_path(trash_relative_path)?;
                    (trash_relative_path, None)
                }
            };
            if !paths.insert(relative_path) {
                return Err(WorkspaceError::new(format!(
                    "transaction mutates {} more than once",
                    relative_path.display()
                )));
            }
            let current_revision = self.revision_at(relative_path)?;
            if current_revision != expected_revision {
                return Err(WorkspaceError::new(format!(
                    "asset revision conflict at {}: expected {}, current {}",
                    relative_path.display(),
                    display_digest(expected_revision),
                    display_digest(current_revision)
                )));
            }
        }
        for asset in mutations.iter().filter_map(|mutation| match mutation {
            WorkspaceMutation::Put { asset, .. } => Some(asset),
            WorkspaceMutation::Delete { .. } | WorkspaceMutation::Archive { .. } => None,
        }) {
            validate_asset_references(asset, &available)?;
        }
        Ok(())
    }

    pub(super) fn validate_asset_path(
        &self,
        relative_path: &Path,
        kind: WorkspaceAssetKind,
    ) -> Result<(), WorkspaceError> {
        validate_relative_path("asset path", relative_path)?;
        if relative_path.extension().and_then(|value| value.to_str()) != Some("json") {
            return Err(WorkspaceError::new("asset path must end in .json"));
        }
        let expected_root = self
            .manifest
            .asset_roots
            .get(&kind)
            .expect("validated manifest has every root");
        if !relative_path.starts_with(expected_root) {
            return Err(WorkspaceError::new(format!(
                "{kind:?} asset must be stored under {expected_root}"
            )));
        }
        Ok(())
    }

    pub(super) fn revision_at(
        &self,
        relative_path: &Path,
    ) -> Result<Option<Digest>, WorkspaceError> {
        let path = self.root.join(relative_path);
        path.is_file()
            .then(|| read_asset(&path).and_then(|asset| asset.digest()))
            .transpose()
    }

    pub(super) fn apply_transaction(
        &self,
        transaction_root: &Path,
        journal: &WorkspaceTransactionJournal,
    ) -> Result<(), WorkspaceError> {
        for operation in &journal.operations {
            match operation {
                JournalOperation::Put {
                    relative_path,
                    staged_file,
                    expected_revision_sha256,
                    new_revision_sha256,
                } => {
                    let relative_path = Path::new(relative_path);
                    validate_relative_path("transaction target", relative_path)?;
                    validate_relative_path("transaction staged file", Path::new(staged_file))?;
                    let current = self.revision_at(relative_path)?;
                    if current == Some(*new_revision_sha256) {
                        continue;
                    }
                    if current != *expected_revision_sha256 {
                        return Err(WorkspaceError::new(format!(
                            "cannot recover transaction {}: {} expected {}, current {}",
                            journal.id,
                            relative_path.display(),
                            display_digest(*expected_revision_sha256),
                            display_digest(current)
                        )));
                    }
                    let bytes =
                        fs::read(transaction_root.join(staged_file)).map_err(WorkspaceError::io)?;
                    let staged_asset: WorkspaceAsset =
                        serde_json::from_slice(&bytes).map_err(WorkspaceError::json)?;
                    if staged_asset.digest()? != *new_revision_sha256 {
                        return Err(WorkspaceError::new(format!(
                            "transaction {} staged asset digest changed",
                            journal.id
                        )));
                    }
                    let target = self.root.join(relative_path);
                    if let Some(parent) = target.parent() {
                        fs::create_dir_all(parent).map_err(WorkspaceError::io)?;
                    }
                    write_atomically(&target, &bytes)?;
                }
                JournalOperation::Delete {
                    relative_path,
                    expected_revision_sha256,
                } => {
                    let relative_path = Path::new(relative_path);
                    validate_relative_path("transaction target", relative_path)?;
                    let Some(current) = self.revision_at(relative_path)? else {
                        continue;
                    };
                    if current != *expected_revision_sha256 {
                        return Err(WorkspaceError::new(format!(
                            "cannot recover transaction {}: {} expected {}, current {}",
                            journal.id,
                            relative_path.display(),
                            expected_revision_sha256,
                            current
                        )));
                    }
                    fs::remove_file(self.root.join(relative_path)).map_err(WorkspaceError::io)?;
                }
            }
        }
        Ok(())
    }

    pub(super) fn asset_root(&self, kind: WorkspaceAssetKind) -> Result<PathBuf, WorkspaceError> {
        let relative = self
            .manifest
            .asset_roots
            .get(&kind)
            .ok_or_else(|| WorkspaceError::new(format!("missing {kind:?} asset root")))?;
        Ok(self.root.join(relative))
    }
}
