//! Load, save, move, duplicate, and trash typed workspace assets.

use super::*;

impl WorkspaceStore {
    pub fn load_asset(&self, id: &str) -> Result<(WorkspaceAsset, PathBuf), WorkspaceError> {
        validate_stable_id("asset id", id)?;
        let listing = self
            .list_assets()?
            .into_iter()
            .find(|listing| listing.id == id)
            .ok_or_else(|| WorkspaceError::new(format!("asset {id} does not exist")))?;
        let path = self.root.join(&listing.relative_path);
        Ok((read_asset(&path)?, listing.relative_path))
    }

    pub fn save_asset(
        &self,
        relative_path: &Path,
        expected_revision_sha256: Option<Digest>,
        asset: &WorkspaceAsset,
    ) -> Result<Digest, WorkspaceError> {
        asset.validate()?;
        self.validate_asset_path(relative_path, asset.header.kind)?;
        if self
            .list_assets()?
            .iter()
            .any(|listing| listing.id == asset.header.id && listing.relative_path != relative_path)
        {
            return Err(WorkspaceError::new(format!(
                "asset identity {} already exists at another path",
                asset.header.id
            )));
        }
        let mut available = self
            .list_assets()?
            .into_iter()
            .map(|listing| (listing.id, listing.kind))
            .collect::<BTreeMap<_, _>>();
        available.insert(asset.header.id.clone(), asset.header.kind);
        validate_asset_references(asset, &available)?;
        let path = self.root.join(relative_path);
        let current_revision = path
            .is_file()
            .then(|| read_asset(&path).and_then(|current| current.digest()))
            .transpose()?;
        if current_revision != expected_revision_sha256 {
            return Err(WorkspaceError::new(format!(
                "asset revision conflict: expected {}, current {}",
                display_digest(expected_revision_sha256),
                display_digest(current_revision)
            )));
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(WorkspaceError::io)?;
        }
        write_atomically(&path, &asset.canonical_bytes()?)?;
        asset.digest()
    }

    /// Durably commits a set of asset writes and deletes as one recoverable unit.
    ///
    /// A crash can interrupt the visible filesystem updates, but the prepared
    /// journal is replayed on the next open before assets are returned.
    pub fn transact(&self, mutations: &[WorkspaceMutation]) -> Result<(), WorkspaceError> {
        if mutations.is_empty() {
            return Err(WorkspaceError::new(
                "workspace transaction must contain at least one mutation",
            ));
        }
        self.validate_mutations(mutations)?;
        let id = format!(
            "transaction-{}-{}",
            std::process::id(),
            NEXT_TEMPORARY_ID.fetch_add(1, Ordering::Relaxed)
        );
        let transaction_root = self.root.join(TRANSACTION_ROOT).join(&id);
        fs::create_dir(&transaction_root).map_err(WorkspaceError::io)?;
        let mut operations = Vec::with_capacity(mutations.len());
        for (index, mutation) in mutations.iter().enumerate() {
            match mutation {
                WorkspaceMutation::Put {
                    relative_path,
                    expected_revision_sha256,
                    asset,
                } => {
                    let staged_file = format!("asset-{index:04}.json");
                    let bytes = asset.canonical_bytes()?;
                    write_new_synced(&transaction_root.join(&staged_file), &bytes)?;
                    operations.push(JournalOperation::Put {
                        relative_path: path_to_slashes(relative_path)?,
                        staged_file,
                        expected_revision_sha256: *expected_revision_sha256,
                        new_revision_sha256: asset.digest()?,
                    });
                }
                WorkspaceMutation::Delete {
                    relative_path,
                    expected_revision_sha256,
                } => operations.push(JournalOperation::Delete {
                    relative_path: path_to_slashes(relative_path)?,
                    expected_revision_sha256: *expected_revision_sha256,
                }),
                WorkspaceMutation::Archive {
                    trash_relative_path,
                    asset,
                } => {
                    let staged_file = format!("asset-{index:04}.json");
                    let bytes = asset.canonical_bytes()?;
                    write_new_synced(&transaction_root.join(&staged_file), &bytes)?;
                    operations.push(JournalOperation::Put {
                        relative_path: path_to_slashes(trash_relative_path)?,
                        staged_file,
                        expected_revision_sha256: None,
                        new_revision_sha256: asset.digest()?,
                    });
                }
            }
        }
        let journal = WorkspaceTransactionJournal {
            schema: TRANSACTION_SCHEMA.into(),
            id,
            operations,
        };
        write_atomically(
            &transaction_root.join("transaction.json"),
            &canonical_json(&journal)?,
        )?;
        self.apply_transaction(&transaction_root, &journal)?;
        remove_transaction_directory(&self.root, &transaction_root)
    }

    pub fn move_asset(
        &self,
        id: &str,
        destination: &Path,
        expected_revision_sha256: Digest,
    ) -> Result<(), WorkspaceError> {
        let (asset, source) = self.load_asset(id)?;
        if source == destination {
            return Ok(());
        }
        self.transact(&[
            WorkspaceMutation::Put {
                relative_path: destination.to_path_buf(),
                expected_revision_sha256: None,
                asset,
            },
            WorkspaceMutation::Delete {
                relative_path: source,
                expected_revision_sha256,
            },
        ])
    }

    pub fn rename_asset(
        &self,
        id: &str,
        label: impl Into<String>,
        expected_revision_sha256: Digest,
    ) -> Result<Digest, WorkspaceError> {
        let (mut asset, relative_path) = self.load_asset(id)?;
        if asset.digest()? != expected_revision_sha256 {
            return Err(WorkspaceError::new("asset revision conflict before rename"));
        }
        asset.header.label = label.into();
        self.save_asset(&relative_path, Some(expected_revision_sha256), &asset)
    }

    pub fn duplicate_asset(
        &self,
        source_id: &str,
        new_id: impl Into<String>,
        new_label: impl Into<String>,
        destination: &Path,
    ) -> Result<Digest, WorkspaceError> {
        let (mut asset, _) = self.load_asset(source_id)?;
        asset.header.id = new_id.into();
        asset.header.label = new_label.into();
        asset.header.version = 1;
        self.save_asset(destination, None, &asset)
    }

    pub fn inbound_references(
        &self,
        id: &str,
    ) -> Result<Vec<WorkspaceAssetListing>, WorkspaceError> {
        validate_stable_id("asset id", id)?;
        let mut inbound = Vec::new();
        for listing in self.list_assets()? {
            let asset = read_asset(&self.root.join(&listing.relative_path))?;
            if asset
                .references
                .iter()
                .any(|reference| reference.asset_id == id)
            {
                inbound.push(listing);
            }
        }
        Ok(inbound)
    }

    pub fn delete_to_trash(
        &self,
        id: &str,
        expected_revision_sha256: Digest,
        allow_broken_references: bool,
    ) -> Result<(), WorkspaceError> {
        let inbound = self.inbound_references(id)?;
        if !allow_broken_references && !inbound.is_empty() {
            return Err(WorkspaceError::new(format!(
                "asset {id} is referenced by {}; confirm deletion to preserve these as broken stable-ID references",
                inbound
                    .iter()
                    .map(|listing| listing.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
        let (asset, source) = self.load_asset(id)?;
        if asset.digest()? != expected_revision_sha256 {
            return Err(WorkspaceError::new("asset revision conflict before delete"));
        }
        let trash_relative_path = Path::new(TRASH_ROOT).join(&source);
        self.transact(&[
            WorkspaceMutation::Archive {
                trash_relative_path,
                asset,
            },
            WorkspaceMutation::Delete {
                relative_path: source,
                expected_revision_sha256,
            },
        ])
    }

    pub fn list_trash(&self) -> Result<Vec<WorkspaceTrashListing>, WorkspaceError> {
        let root = self.root.join(TRASH_ROOT);
        let mut trash = Vec::new();
        collect_asset_files(&root, &mut |path| {
            let asset = read_asset(path)?;
            let relative = path
                .strip_prefix(&root)
                .map_err(|_| WorkspaceError::new("trash asset escaped trash root"))?
                .to_path_buf();
            trash.push(WorkspaceTrashListing {
                id: asset.header.id.clone(),
                label: asset.header.label.clone(),
                kind: asset.header.kind,
                original_relative_path: relative,
                revision_sha256: asset.digest()?,
            });
            Ok(())
        })?;
        trash.sort_by(|left, right| {
            left.kind
                .cmp(&right.kind)
                .then_with(|| left.label.cmp(&right.label))
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(trash)
    }

    pub fn restore_from_trash(
        &self,
        id: &str,
        expected_revision_sha256: Digest,
    ) -> Result<(), WorkspaceError> {
        let listing = self
            .list_trash()?
            .into_iter()
            .find(|listing| listing.id == id)
            .ok_or_else(|| WorkspaceError::new(format!("trashed asset {id} does not exist")))?;
        if listing.revision_sha256 != expected_revision_sha256 {
            return Err(WorkspaceError::new(
                "trash revision conflict before restore",
            ));
        }
        let trash_relative_path = Path::new(TRASH_ROOT).join(&listing.original_relative_path);
        let asset = read_asset(&self.root.join(&trash_relative_path))?;
        self.transact(&[
            WorkspaceMutation::Put {
                relative_path: listing.original_relative_path,
                expected_revision_sha256: None,
                asset,
            },
            WorkspaceMutation::Delete {
                relative_path: trash_relative_path,
                expected_revision_sha256,
            },
        ])
    }

    pub fn permanently_delete_from_trash(
        &self,
        id: &str,
        expected_revision_sha256: Digest,
    ) -> Result<(), WorkspaceError> {
        let listing = self
            .list_trash()?
            .into_iter()
            .find(|listing| listing.id == id)
            .ok_or_else(|| WorkspaceError::new(format!("trashed asset {id} does not exist")))?;
        if listing.revision_sha256 != expected_revision_sha256 {
            return Err(WorkspaceError::new(
                "trash revision conflict before permanent delete",
            ));
        }
        let path = self
            .root
            .join(TRASH_ROOT)
            .join(listing.original_relative_path);
        fs::remove_file(path).map_err(WorkspaceError::io)
    }
}
