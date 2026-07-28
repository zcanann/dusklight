//! Discover, create, import, and summarize workspace roots.

use super::*;

impl WorkspaceRegistry {
    pub fn open(
        root: impl Into<PathBuf>,
        available_libraries: BTreeMap<(String, String), Digest>,
    ) -> Result<Self, WorkspaceError> {
        let root = root.into();
        fs::create_dir_all(&root).map_err(WorkspaceError::io)?;
        let root = root.canonicalize().map_err(WorkspaceError::io)?;
        Ok(Self {
            root,
            available_libraries,
        })
    }

    pub fn list(&self) -> Result<WorkspaceList, WorkspaceError> {
        let mut workspaces = Vec::new();
        for entry in fs::read_dir(&self.root).map_err(WorkspaceError::io)? {
            let entry = entry.map_err(WorkspaceError::io)?;
            if !entry.file_type().map_err(WorkspaceError::io)?.is_dir() {
                continue;
            }
            let path = entry.path();
            if !path.join(MANIFEST_FILE).is_file() {
                continue;
            }
            let manifest = read_manifest_and_migrate(&path.join(MANIFEST_FILE))?;
            let directory_id = path.file_name().and_then(|name| name.to_str());
            if directory_id != Some(manifest.id.as_str()) {
                return Err(WorkspaceError::new(format!(
                    "workspace folder {} does not match stable identity {}",
                    path.display(),
                    manifest.id
                )));
            }
            let issues = dependency_issues(&manifest, &self.available_libraries);
            let dependency_error = (!issues.is_empty()).then(|| format_dependency_issues(&issues));
            let asset_count = if dependency_error.is_none() {
                WorkspaceStore::open(&path, &self.available_libraries)?
                    .list_assets()?
                    .len()
            } else {
                0
            };
            workspaces.push(WorkspaceSummary {
                id: manifest.id,
                label: manifest.label,
                asset_count,
                dependency_error,
            });
        }
        workspaces.sort_by(|left, right| {
            left.label
                .cmp(&right.label)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(WorkspaceList {
            schema: WORKSPACE_LIST_SCHEMA.into(),
            workspaces,
        })
    }

    pub fn create(
        &self,
        request: WorkspaceCreateRequest,
    ) -> Result<WorkspaceRecord, WorkspaceError> {
        if request.schema != WORKSPACE_CREATE_SCHEMA {
            return Err(WorkspaceError::new(
                "workspace create request schema is unsupported",
            ));
        }
        let manifest = WorkspaceManifest::new(request.id, request.label)?;
        let path = self.workspace_path(&manifest.id)?;
        if path.exists() {
            return Err(WorkspaceError::new(format!(
                "workspace {} already exists",
                manifest.id
            )));
        }
        let store = WorkspaceStore::create(path, manifest)?;
        workspace_record(&store)
    }

    pub fn load(&self, id: &str) -> Result<WorkspaceRecord, WorkspaceError> {
        let path = self.workspace_path(id)?;
        if !path.is_dir() {
            return Err(WorkspaceError::new(format!(
                "workspace {id} does not exist"
            )));
        }
        let store = WorkspaceStore::open(path, &self.available_libraries)?;
        workspace_record(&store)
    }

    pub fn export(&self, id: &str) -> Result<WorkspaceExport, WorkspaceError> {
        let store = self.open_workspace(id)?;
        let folders = store
            .list_folders()?
            .into_iter()
            .map(|listing| {
                Ok(WorkspaceExportFolder {
                    relative_path: listing.relative_path.clone(),
                    folder: store.load_folder(&listing.id)?.0,
                })
            })
            .collect::<Result<Vec<_>, WorkspaceError>>()?;
        let assets = store
            .list_assets()?
            .into_iter()
            .map(|listing| {
                let (asset, relative_path) = store.load_asset(&listing.id)?;
                Ok(WorkspaceExportAsset {
                    relative_path,
                    asset,
                })
            })
            .collect::<Result<Vec<_>, WorkspaceError>>()?;
        Ok(WorkspaceExport {
            schema: WORKSPACE_EXPORT_SCHEMA.into(),
            manifest: store.manifest().clone(),
            folders,
            assets,
        })
    }

    pub fn import(&self, bundle: WorkspaceExport) -> Result<WorkspaceRecord, WorkspaceError> {
        if bundle.schema != WORKSPACE_EXPORT_SCHEMA
            && bundle.schema != LEGACY_WORKSPACE_EXPORT_SCHEMA
        {
            return Err(WorkspaceError::new(
                "workspace export schema is unsupported",
            ));
        }
        bundle.manifest.validate()?;
        let issues = dependency_issues(&bundle.manifest, &self.available_libraries);
        if !issues.is_empty() {
            return Err(WorkspaceError::new(format_dependency_issues(&issues)));
        }
        let root = self.workspace_path(&bundle.manifest.id)?;
        if root.exists() {
            return Err(WorkspaceError::new(format!(
                "workspace {} already exists",
                bundle.manifest.id
            )));
        }
        let mut folder_ids = BTreeSet::new();
        let mut folder_paths = BTreeSet::new();
        for record in &bundle.folders {
            record.folder.validate()?;
            validate_folder_relative_path(
                &bundle.manifest,
                record.folder.kind,
                &record.relative_path,
            )?;
            if !folder_ids.insert(record.folder.id.clone()) {
                return Err(WorkspaceError::new(format!(
                    "workspace import duplicates folder identity {}",
                    record.folder.id
                )));
            }
            if !folder_paths.insert(record.relative_path.clone()) {
                return Err(WorkspaceError::new(format!(
                    "workspace import duplicates folder path {}",
                    record.relative_path.display()
                )));
            }
        }
        validate_export_folder_hierarchy(&bundle.manifest, &bundle.folders)?;
        let mut ids = BTreeSet::new();
        let mut paths = BTreeSet::new();
        for record in &bundle.assets {
            record.asset.validate()?;
            validate_relative_path("workspace import asset path", &record.relative_path)?;
            let expected_root = bundle
                .manifest
                .asset_roots
                .get(&record.asset.header.kind)
                .expect("validated manifest contains every asset root");
            if !record.relative_path.starts_with(expected_root)
                || record
                    .relative_path
                    .extension()
                    .and_then(|value| value.to_str())
                    != Some("json")
            {
                return Err(WorkspaceError::new(format!(
                    "{:?} import asset must be a JSON file below {}",
                    record.asset.header.kind, expected_root
                )));
            }
            if !ids.insert(record.asset.header.id.clone()) {
                return Err(WorkspaceError::new(format!(
                    "workspace import duplicates asset identity {}",
                    record.asset.header.id
                )));
            }
            if !paths.insert(record.relative_path.clone()) {
                return Err(WorkspaceError::new(format!(
                    "workspace import duplicates asset path {}",
                    record.relative_path.display()
                )));
            }
        }
        let store = WorkspaceStore::create(&root, bundle.manifest)?;
        if let Err(error) = store.import_folders(&bundle.folders) {
            let cleanup = fs::remove_dir_all(&root);
            return match cleanup {
                Ok(()) => Err(error),
                Err(cleanup) => Err(WorkspaceError::new(format!(
                    "{error}; failed to remove incomplete imported workspace: {cleanup}"
                ))),
            };
        }
        let mutations = bundle
            .assets
            .into_iter()
            .map(|record| WorkspaceMutation::Put {
                relative_path: record.relative_path,
                expected_revision_sha256: None,
                asset: record.asset,
            })
            .collect::<Vec<_>>();
        if let Err(error) = store.transact(&mutations) {
            let cleanup = fs::remove_dir_all(&root);
            return match cleanup {
                Ok(()) => Err(error),
                Err(cleanup) => Err(WorkspaceError::new(format!(
                    "{error}; failed to remove incomplete imported workspace: {cleanup}"
                ))),
            };
        }
        workspace_record(&store)
    }

    pub fn command_folder(
        &self,
        workspace_id: &str,
        folder_id: &str,
        request: WorkspaceFolderCommandRequest,
    ) -> Result<WorkspaceRecord, WorkspaceError> {
        if request.schema != WORKSPACE_FOLDER_COMMAND_SCHEMA {
            return Err(WorkspaceError::new(
                "workspace folder command schema is unsupported",
            ));
        }
        let store = self.open_workspace(workspace_id)?;
        match request.command {
            WorkspaceFolderCommand::Create {
                id,
                label,
                asset_kind,
                parent_id,
                directory_name,
            } => {
                if id != folder_id {
                    return Err(WorkspaceError::new(
                        "URL folder id does not match create command",
                    ));
                }
                store.create_folder(
                    id,
                    label,
                    asset_kind,
                    parent_id.as_deref(),
                    &directory_name,
                )?;
            }
            WorkspaceFolderCommand::Rename {
                expected_revision_sha256,
                label,
                directory_name,
            } => {
                store.rename_folder(folder_id, label, &directory_name, expected_revision_sha256)?;
            }
            WorkspaceFolderCommand::Move {
                expected_revision_sha256,
                parent_id,
            } => {
                store.move_folder(folder_id, parent_id.as_deref(), expected_revision_sha256)?;
            }
            WorkspaceFolderCommand::Duplicate {
                new_id,
                new_label,
                parent_id,
                directory_name,
            } => {
                store.duplicate_folder(
                    folder_id,
                    new_id,
                    new_label,
                    parent_id.as_deref(),
                    &directory_name,
                )?;
            }
            WorkspaceFolderCommand::DeleteToTrash {
                expected_revision_sha256,
                allow_broken_references,
            } => {
                store.delete_folder_to_trash(
                    folder_id,
                    expected_revision_sha256,
                    allow_broken_references,
                )?;
            }
        }
        workspace_record(&store)
    }

    pub fn list_folder_trash(
        &self,
        workspace_id: &str,
    ) -> Result<Vec<WorkspaceFolderTrashListing>, WorkspaceError> {
        self.open_workspace(workspace_id)?.list_folder_trash()
    }

    pub fn command_folder_trash(
        &self,
        workspace_id: &str,
        folder_id: &str,
        request: WorkspaceFolderTrashCommandRequest,
    ) -> Result<WorkspaceRecord, WorkspaceError> {
        if request.schema != WORKSPACE_FOLDER_TRASH_COMMAND_SCHEMA {
            return Err(WorkspaceError::new(
                "workspace folder trash command schema is unsupported",
            ));
        }
        let store = self.open_workspace(workspace_id)?;
        match request.command {
            WorkspaceTrashCommand::Restore => {
                store.restore_folder_from_trash(folder_id, request.expected_revision_sha256)?;
            }
            WorkspaceTrashCommand::PermanentlyDelete => {
                store.permanently_delete_folder_from_trash(
                    folder_id,
                    request.expected_revision_sha256,
                )?;
            }
        }
        workspace_record(&store)
    }

    pub fn load_asset(
        &self,
        workspace_id: &str,
        asset_id: &str,
    ) -> Result<WorkspaceAssetRecord, WorkspaceError> {
        let store = self.open_workspace(workspace_id)?;
        let (asset, relative_path) = store.load_asset(asset_id)?;
        let revision_sha256 = asset.digest()?;
        Ok(WorkspaceAssetRecord {
            schema: WORKSPACE_ASSET_RECORD_SCHEMA.into(),
            relative_path,
            revision_sha256,
            asset,
        })
    }

    pub fn save_asset(
        &self,
        workspace_id: &str,
        asset_id: &str,
        request: WorkspaceAssetSaveRequest,
    ) -> Result<WorkspaceAssetRecord, WorkspaceError> {
        if request.schema != WORKSPACE_ASSET_SAVE_SCHEMA {
            return Err(WorkspaceError::new(
                "workspace asset save request schema is unsupported",
            ));
        }
        if request.asset.header.id != asset_id {
            return Err(WorkspaceError::new(
                "URL asset id does not match the document",
            ));
        }
        let store = self.open_workspace(workspace_id)?;
        let revision_sha256 = store.save_asset(
            &request.relative_path,
            request.expected_revision_sha256,
            &request.asset,
        )?;
        Ok(WorkspaceAssetRecord {
            schema: WORKSPACE_ASSET_RECORD_SCHEMA.into(),
            relative_path: request.relative_path,
            revision_sha256,
            asset: request.asset,
        })
    }

    pub fn save_route_graph(
        &self,
        workspace_id: &str,
        graph_id: &str,
        request: WorkspaceRouteGraphSaveRequest,
        catalog: &ComposedPlannerCatalog,
    ) -> Result<WorkspaceRouteGraphEditRecord, WorkspaceError> {
        if request.schema != WORKSPACE_ROUTE_GRAPH_SAVE_SCHEMA {
            return Err(WorkspaceError::new(
                "workspace route-graph save request schema is unsupported",
            ));
        }
        catalog.validate()?;
        request.route_book.validate_against_composed(catalog)?;
        let store = self.open_workspace(workspace_id)?;
        let (mut graph_asset, graph_path) = store.load_asset(graph_id)?;
        if graph_asset.header.kind != WorkspaceAssetKind::RouteGraph {
            return Err(WorkspaceError::new(format!(
                "asset {graph_id} is not a route graph"
            )));
        }
        if graph_asset.digest()? != request.expected_graph_revision_sha256 {
            return Err(WorkspaceError::new(
                "route graph revision conflict before save",
            ));
        }
        if !graph_asset.references.iter().any(|reference| {
            reference.asset_id == request.route_book_id
                && reference.kind == WorkspaceAssetKind::RouteBook
        }) {
            return Err(WorkspaceError::new(format!(
                "route graph {graph_id} does not reference route book {}",
                request.route_book_id
            )));
        }
        let (mut route_book_asset, route_book_path) = store.load_asset(&request.route_book_id)?;
        if route_book_asset.header.kind != WorkspaceAssetKind::RouteBook {
            return Err(WorkspaceError::new(format!(
                "asset {} is not a route book",
                request.route_book_id
            )));
        }
        if route_book_asset.digest()? != request.expected_route_book_revision_sha256 {
            return Err(WorkspaceError::new(
                "route book revision conflict before save",
            ));
        }
        graph_asset.payload = WorkspaceAssetPayload::RouteGraph {
            graph: PlannerGraph::project_composed_with_route_book(catalog, &request.route_book)?,
        };
        route_book_asset.payload = WorkspaceAssetPayload::RouteBook {
            route_book: request.route_book,
        };

        let mut mutations = vec![
            WorkspaceMutation::Put {
                relative_path: graph_path.clone(),
                expected_revision_sha256: Some(request.expected_graph_revision_sha256),
                asset: graph_asset.clone(),
            },
            WorkspaceMutation::Put {
                relative_path: route_book_path.clone(),
                expected_revision_sha256: Some(request.expected_route_book_revision_sha256),
                asset: route_book_asset.clone(),
            },
        ];
        let mut saved_layout = None;
        if let Some(edit) = request.layout {
            let (mut asset, path) = store.load_asset(&edit.asset_id)?;
            if asset.digest()? != edit.expected_revision_sha256 {
                return Err(WorkspaceError::new("layout revision conflict before save"));
            }
            let WorkspaceAssetPayload::Layout(layout) = &mut asset.payload else {
                return Err(WorkspaceError::new(format!(
                    "asset {} is not a layout",
                    edit.asset_id
                )));
            };
            if layout.semantic_asset_id != graph_id {
                return Err(WorkspaceError::new(format!(
                    "layout {} presents {}, not route graph {graph_id}",
                    edit.asset_id, layout.semantic_asset_id
                )));
            }
            layout.positions = edit.positions;
            layout.viewport = edit.viewport;
            mutations.push(WorkspaceMutation::Put {
                relative_path: path.clone(),
                expected_revision_sha256: Some(edit.expected_revision_sha256),
                asset: asset.clone(),
            });
            saved_layout = Some((asset, path));
        }
        store.transact(&mutations)?;

        let asset_record = |asset: WorkspaceAsset,
                            relative_path: PathBuf|
         -> Result<WorkspaceAssetRecord, WorkspaceError> {
            Ok(WorkspaceAssetRecord {
                schema: WORKSPACE_ASSET_RECORD_SCHEMA.into(),
                revision_sha256: asset.digest()?,
                relative_path,
                asset,
            })
        };
        Ok(WorkspaceRouteGraphEditRecord {
            schema: WORKSPACE_ROUTE_GRAPH_EDIT_RECORD_SCHEMA.into(),
            workspace: workspace_record(&store)?,
            graph: asset_record(graph_asset, graph_path)?,
            route_book: asset_record(route_book_asset, route_book_path)?,
            layout: saved_layout
                .map(|(asset, path)| asset_record(asset, path))
                .transpose()?,
        })
    }

    pub fn command_asset(
        &self,
        workspace_id: &str,
        asset_id: &str,
        request: WorkspaceAssetCommandRequest,
    ) -> Result<WorkspaceRecord, WorkspaceError> {
        if request.schema != WORKSPACE_ASSET_COMMAND_SCHEMA {
            return Err(WorkspaceError::new(
                "workspace asset command schema is unsupported",
            ));
        }
        let store = self.open_workspace(workspace_id)?;
        match request.command {
            WorkspaceAssetCommand::Rename {
                expected_revision_sha256,
                label,
            } => {
                store.rename_asset(asset_id, label, expected_revision_sha256)?;
            }
            WorkspaceAssetCommand::Move {
                expected_revision_sha256,
                relative_path,
            } => {
                store.move_asset(asset_id, &relative_path, expected_revision_sha256)?;
            }
            WorkspaceAssetCommand::Duplicate {
                new_id,
                new_label,
                relative_path,
            } => {
                store.duplicate_asset(asset_id, new_id, new_label, &relative_path)?;
            }
            WorkspaceAssetCommand::DeleteToTrash {
                expected_revision_sha256,
                allow_broken_references,
            } => {
                store.delete_to_trash(
                    asset_id,
                    expected_revision_sha256,
                    allow_broken_references,
                )?;
            }
        }
        workspace_record(&store)
    }

    pub fn list_trash(
        &self,
        workspace_id: &str,
    ) -> Result<Vec<WorkspaceTrashListing>, WorkspaceError> {
        self.open_workspace(workspace_id)?.list_trash()
    }

    pub fn command_trash(
        &self,
        workspace_id: &str,
        asset_id: &str,
        request: WorkspaceTrashCommandRequest,
    ) -> Result<WorkspaceRecord, WorkspaceError> {
        if request.schema != WORKSPACE_TRASH_COMMAND_SCHEMA {
            return Err(WorkspaceError::new(
                "workspace trash command schema is unsupported",
            ));
        }
        let store = self.open_workspace(workspace_id)?;
        match request.command {
            WorkspaceTrashCommand::Restore => {
                store.restore_from_trash(asset_id, request.expected_revision_sha256)?;
            }
            WorkspaceTrashCommand::PermanentlyDelete => {
                store.permanently_delete_from_trash(asset_id, request.expected_revision_sha256)?;
            }
        }
        workspace_record(&store)
    }

    pub fn create_scenario_from_library(
        &self,
        workspace_id: &str,
        project: &PlannerWebProject,
        library_sha256: Digest,
    ) -> Result<WorkspaceRecord, WorkspaceError> {
        let mut store = self.open_workspace(workspace_id)?;
        store.import_project_template(project, library_sha256)?;
        workspace_record(&store)
    }

    pub fn create_blank_scenario(
        &self,
        workspace_id: &str,
        project: &PlannerWebProject,
        library_sha256: Digest,
        request: WorkspaceScenarioCreateRequest,
    ) -> Result<WorkspaceRecord, WorkspaceError> {
        if request.schema != WORKSPACE_SCENARIO_CREATE_SCHEMA {
            return Err(WorkspaceError::new(
                "workspace scenario create schema is unsupported",
            ));
        }
        if request.library_id != project.id {
            return Err(WorkspaceError::new(
                "scenario Library identity does not match the selected exact source",
            ));
        }
        let mut store = self.open_workspace(workspace_id)?;
        store.import_blank_scenario(
            project,
            library_sha256,
            &request.namespace,
            &request.label,
            &request.goal_id,
        )?;
        workspace_record(&store)
    }

    pub fn add_library_reference(
        &self,
        workspace_id: &str,
        project: &PlannerWebProject,
        library_sha256: Digest,
    ) -> Result<WorkspaceRecord, WorkspaceError> {
        project
            .validate()
            .map_err(|error| WorkspaceError::new(error.to_string()))?;
        if library_sha256 == Digest::ZERO {
            return Err(WorkspaceError::new("library digest must be nonzero"));
        }
        let exact_context = project
            .start_state
            .as_ref()
            .ok_or_else(|| WorkspaceError::new("Library has no exact context to reference"))?
            .snapshot
            .environment
            .runtime_configuration
            .exact_context()?;
        let mut store = self.open_workspace(workspace_id)?;
        store.mount_library(
            MountedLibrary {
                id: project.id.clone(),
                version: BUILTIN_LIBRARY_VERSION.into(),
                sha256: library_sha256,
                source: format!("builtin:{}", project.id),
            },
            exact_context,
        )?;
        workspace_record(&store)
    }

    pub fn fork_library(
        &self,
        workspace_id: &str,
        project: &PlannerWebProject,
        library_sha256: Digest,
        request: WorkspaceLibraryForkRequest,
    ) -> Result<WorkspaceRecord, WorkspaceError> {
        if request.schema != WORKSPACE_LIBRARY_FORK_SCHEMA {
            return Err(WorkspaceError::new(
                "workspace Library fork schema is unsupported",
            ));
        }
        let mut store = self.open_workspace(workspace_id)?;
        store.fork_project_template(project, library_sha256, &request.namespace)?;
        workspace_record(&store)
    }

    fn open_workspace(&self, id: &str) -> Result<WorkspaceStore, WorkspaceError> {
        let path = self.workspace_path(id)?;
        if !path.is_dir() {
            return Err(WorkspaceError::new(format!(
                "workspace {id} does not exist"
            )));
        }
        WorkspaceStore::open(path, &self.available_libraries)
    }

    fn workspace_path(&self, id: &str) -> Result<PathBuf, WorkspaceError> {
        validate_stable_id("workspace id", id)?;
        if id.contains('/') || id.contains(':') {
            return Err(WorkspaceError::new(
                "workspace id used as a folder cannot contain '/' or ':'",
            ));
        }
        Ok(self.root.join(id))
    }
}

fn workspace_record(store: &WorkspaceStore) -> Result<WorkspaceRecord, WorkspaceError> {
    Ok(WorkspaceRecord {
        schema: WORKSPACE_RECORD_SCHEMA.into(),
        manifest: store.manifest().clone(),
        folders: store.list_folders()?,
        assets: store.list_assets()?,
    })
}
