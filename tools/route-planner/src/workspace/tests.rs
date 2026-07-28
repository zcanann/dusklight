
use super::*;
use dusklight_route_planner::graph::PLANNER_GRAPH_SCHEMA;

fn temporary_directory(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "dusklight-route-planner-{label}-{}-{}",
        std::process::id(),
        NEXT_TEMPORARY_ID.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

fn graph_asset(id: &str, label: &str) -> WorkspaceAsset {
    WorkspaceAsset {
        schema: WORKSPACE_ASSET_SCHEMA.into(),
        header: WorkspaceAssetHeader {
            id: id.into(),
            label: label.into(),
            kind: WorkspaceAssetKind::RouteGraph,
            version: 1,
            origin: None,
        },
        references: Vec::new(),
        payload: WorkspaceAssetPayload::RouteGraph {
            graph: PlannerGraph {
                schema: PLANNER_GRAPH_SCHEMA.into(),
                fact_catalog_sha256: Digest([1; 32]),
                mechanics_catalog_sha256: Digest([2; 32]),
                refinement_stack_sha256: None,
                route_book_sha256: None,
                nodes: Vec::new(),
                edges: Vec::new(),
                regions: Vec::new(),
            },
        },
    }
}

#[test]
fn manifest_is_small_and_defines_fixed_typed_roots() {
    let mut manifest = WorkspaceManifest::new("workspace.test", "Test workspace").unwrap();
    let value = serde_json::to_value(&manifest).unwrap();
    assert_eq!(manifest.asset_roots.len(), WorkspaceAssetKind::ALL.len());
    for kind in WorkspaceAssetKind::ALL {
        assert_eq!(manifest.asset_roots[&kind], kind.root_name());
    }
    for forbidden in ["catalog", "graph", "snapshot", "layout", "route_book"] {
        assert!(value.get(forbidden).is_none());
    }
    assert!(manifest.canonical_bytes().unwrap().ends_with(b"\n"));
    manifest
        .asset_roots
        .insert(WorkspaceAssetKind::RouteGraph, "user-selected-root".into());
    assert!(
        manifest
            .validate()
            .unwrap_err()
            .to_string()
            .contains("asset root is fixed at route-graphs")
    );
}

#[test]
fn semantic_and_layout_identity_are_independent() {
    let semantic = graph_asset("graph.ordon", "Ordon route");
    let semantic_digest = semantic.digest().unwrap();
    let mut layout = WorkspaceAsset {
        schema: WORKSPACE_ASSET_SCHEMA.into(),
        header: WorkspaceAssetHeader {
            id: "layout.ordon".into(),
            label: "Ordon layout".into(),
            kind: WorkspaceAssetKind::Layout,
            version: 1,
            origin: None,
        },
        references: vec![WorkspaceAssetReference {
            asset_id: semantic.header.id.clone(),
            kind: WorkspaceAssetKind::RouteGraph,
        }],
        payload: WorkspaceAssetPayload::Layout(LayoutAsset {
            semantic_asset_id: semantic.header.id.clone(),
            positions: BTreeMap::from([("node.start".into(), LayoutPoint { x: 10.0, y: 20.0 })]),
            viewport: None,
        }),
    };
    layout.validate().unwrap();
    let first_layout_digest = layout.digest().unwrap();
    let WorkspaceAssetPayload::Layout(layout_payload) = &mut layout.payload else {
        unreachable!()
    };
    layout_payload.positions.get_mut("node.start").unwrap().x = 500.0;
    assert_ne!(first_layout_digest, layout.digest().unwrap());
    assert_eq!(semantic_digest, semantic.digest().unwrap());
}

#[test]
fn store_keeps_identity_when_asset_path_changes() {
    let root = temporary_directory("stable-identity");
    let manifest = WorkspaceManifest::new("workspace.test", "Test workspace").unwrap();
    let store = WorkspaceStore::create(&root, manifest).unwrap();
    let asset = graph_asset("graph.ordon", "Ordon route");
    let first = Path::new("route-graphs/ordon/route.json");
    store.save_asset(first, None, &asset).unwrap();
    let moved = Path::new("route-graphs/routes/ordon.json");
    fs::create_dir_all(root.join("route-graphs/routes")).unwrap();
    fs::rename(root.join(first), root.join(moved)).unwrap();
    let (loaded, relative_path) = store.load_asset("graph.ordon").unwrap();
    assert_eq!(loaded.header.id, "graph.ordon");
    assert_eq!(relative_path, moved);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn open_reports_missing_and_changed_library_pins_actionably() {
    let root = temporary_directory("library-pins");
    let mut manifest = WorkspaceManifest::new("workspace.test", "Test workspace").unwrap();
    manifest.mounted_libraries.push(MountedLibrary {
        id: "library.gz2e01".into(),
        version: "1.0.0".into(),
        sha256: Digest([7; 32]),
        source: "libraries/gz2e01.json".into(),
    });
    WorkspaceStore::create(&root, manifest).unwrap();
    let missing = WorkspaceStore::open(&root, &BTreeMap::new()).unwrap_err();
    assert!(missing.to_string().contains("is missing"));
    assert!(missing.to_string().contains("libraries/gz2e01.json"));
    let changed = WorkspaceStore::open(
        &root,
        &BTreeMap::from([(("library.gz2e01".into(), "1.0.0".into()), Digest([8; 32]))]),
    )
    .unwrap_err();
    assert!(changed.to_string().contains("changed"));
    assert!(changed.to_string().contains("explicitly upgrade"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn save_is_canonical_revision_checked_and_path_confined() {
    let root = temporary_directory("asset-save");
    let manifest = WorkspaceManifest::new("workspace.test", "Test workspace").unwrap();
    let store = WorkspaceStore::create(&root, manifest).unwrap();
    let mut asset = graph_asset("graph.ordon", "Ordon route");
    let path = Path::new("route-graphs/ordon/routes/ordon.json");
    let first_revision = store.save_asset(path, None, &asset).unwrap();
    assert!(root.join("route-graphs/ordon/routes").is_dir());
    assert_eq!(
        fs::read(root.join(path)).unwrap(),
        asset.canonical_bytes().unwrap()
    );
    asset.header.label = "Renamed route".into();
    assert!(
        store
            .save_asset(path, None, &asset)
            .unwrap_err()
            .to_string()
            .contains("revision conflict")
    );
    let second_revision = store
        .save_asset(path, Some(first_revision), &asset)
        .unwrap();
    assert_ne!(first_revision, second_revision);
    assert!(
        store
            .save_asset(Path::new("../escape.json"), None, &asset)
            .unwrap_err()
            .to_string()
            .contains("without traversal")
    );
    let wrong_root = Path::new("custom-nodes/ordon.json");
    assert!(
        store
            .save_asset(wrong_root, None, &asset)
            .unwrap_err()
            .to_string()
            .contains("must be stored under route-graphs")
    );
    assert!(!root.join(wrong_root).exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn unsupported_schemas_require_migration_instead_of_best_effort_loading() {
    let root = temporary_directory("migration");
    let manifest = WorkspaceManifest::new("workspace.test", "Test workspace").unwrap();
    WorkspaceStore::create(&root, manifest).unwrap();
    let path = root.join(MANIFEST_FILE);
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["schema"] = "dusklight.route-planner.workspace/v99".into();
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    let error = WorkspaceStore::open(&root, &BTreeMap::new()).unwrap_err();
    assert!(error.to_string().contains("migrate"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn known_manifest_schema_migrates_automatically_and_canonically() {
    let root = temporary_directory("known-migration");
    let manifest = WorkspaceManifest::new("workspace.test", "Test workspace").unwrap();
    WorkspaceStore::create(&root, manifest).unwrap();
    let path = root.join(MANIFEST_FILE);
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["schema"] = LEGACY_WORKSPACE_MANIFEST_SCHEMA.into();
    value.as_object_mut().unwrap().remove("format_version");
    fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();

    let opened = WorkspaceStore::open(&root, &BTreeMap::new()).unwrap();
    assert_eq!(opened.manifest().schema, WORKSPACE_MANIFEST_SCHEMA);
    assert_eq!(opened.manifest().format_version, WORKSPACE_FORMAT_VERSION);
    assert_eq!(
        fs::read(&path).unwrap(),
        opened.manifest().canonical_bytes().unwrap()
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn move_is_one_recoverable_transaction_and_preserves_identity() {
    let root = temporary_directory("transactional-move");
    let manifest = WorkspaceManifest::new("workspace.test", "Test workspace").unwrap();
    let store = WorkspaceStore::create(&root, manifest).unwrap();
    let asset = graph_asset("graph.ordon", "Ordon route");
    let source = Path::new("route-graphs/ordon.json");
    let revision = store.save_asset(source, None, &asset).unwrap();
    let destination = Path::new("route-graphs/routes/ordon.json");
    store
        .move_asset("graph.ordon", destination, revision)
        .unwrap();

    assert!(!root.join(source).exists());
    assert!(root.join(destination).is_file());
    let (loaded, path) = store.load_asset("graph.ordon").unwrap();
    assert_eq!(loaded.header.id, "graph.ordon");
    assert_eq!(path, destination);
    assert_eq!(
        fs::read_dir(root.join(TRANSACTION_ROOT)).unwrap().count(),
        0
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn open_finishes_a_transaction_interrupted_between_operations() {
    let root = temporary_directory("transaction-recovery");
    let manifest = WorkspaceManifest::new("workspace.test", "Test workspace").unwrap();
    let store = WorkspaceStore::create(&root, manifest).unwrap();
    let asset = graph_asset("graph.ordon", "Ordon route");
    let source = Path::new("route-graphs/ordon.json");
    let source_revision = store.save_asset(source, None, &asset).unwrap();
    let destination = Path::new("route-graphs/routes/ordon.json");
    fs::create_dir_all(root.join("route-graphs/routes")).unwrap();

    let id = "transaction-interrupted";
    let transaction_root = root.join(TRANSACTION_ROOT).join(id);
    fs::create_dir(&transaction_root).unwrap();
    let staged_file = "asset-0000.json";
    let bytes = asset.canonical_bytes().unwrap();
    fs::write(transaction_root.join(staged_file), &bytes).unwrap();
    let journal = WorkspaceTransactionJournal {
        schema: TRANSACTION_SCHEMA.into(),
        id: id.into(),
        operations: vec![
            JournalOperation::Put {
                relative_path: path_to_slashes(destination).unwrap(),
                staged_file: staged_file.into(),
                expected_revision_sha256: None,
                new_revision_sha256: asset.digest().unwrap(),
            },
            JournalOperation::Delete {
                relative_path: path_to_slashes(source).unwrap(),
                expected_revision_sha256: source_revision,
            },
        ],
    };
    fs::write(
        transaction_root.join("transaction.json"),
        canonical_json(&journal).unwrap(),
    )
    .unwrap();
    // Simulate a crash after the first replacement but before source delete.
    fs::write(root.join(destination), &bytes).unwrap();
    drop(store);

    let recovered = WorkspaceStore::open(&root, &BTreeMap::new()).unwrap();
    assert!(!root.join(source).exists());
    assert!(root.join(destination).is_file());
    assert_eq!(recovered.load_asset("graph.ordon").unwrap().1, destination);
    assert!(!transaction_root.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn asset_crud_preserves_ids_references_and_recoverable_trash() {
    let root = temporary_directory("asset-crud");
    let manifest = WorkspaceManifest::new("workspace.test", "Test workspace").unwrap();
    let store = WorkspaceStore::create(&root, manifest).unwrap();
    let mut graph = graph_asset("graph.ordon", "Ordon route");
    let graph_path = Path::new("route-graphs/ordon.json");
    let first_revision = store.save_asset(graph_path, None, &graph).unwrap();

    let layout = WorkspaceAsset {
        schema: WORKSPACE_ASSET_SCHEMA.into(),
        header: WorkspaceAssetHeader {
            id: "layout.ordon".into(),
            label: "Ordon layout".into(),
            kind: WorkspaceAssetKind::Layout,
            version: 1,
            origin: None,
        },
        references: vec![WorkspaceAssetReference {
            asset_id: graph.header.id.clone(),
            kind: WorkspaceAssetKind::RouteGraph,
        }],
        payload: WorkspaceAssetPayload::Layout(LayoutAsset {
            semantic_asset_id: graph.header.id.clone(),
            positions: BTreeMap::new(),
            viewport: None,
        }),
    };
    let mut missing_reference = layout.clone();
    missing_reference.header.id = "layout.missing-reference".into();
    missing_reference.references[0].asset_id = "graph.absent".into();
    let missing_path = Path::new("layouts/missing-reference.json");
    assert!(
        store
            .save_asset(missing_path, None, &missing_reference)
            .unwrap_err()
            .to_string()
            .contains("references missing RouteGraph asset graph.absent")
    );
    assert!(!root.join(missing_path).exists());
    let mut wrong_kind = layout.clone();
    wrong_kind.header.id = "layout.wrong-kind".into();
    wrong_kind.references[0].kind = WorkspaceAssetKind::StateSeed;
    let wrong_kind_path = Path::new("layouts/wrong-kind.json");
    assert!(
        store
            .save_asset(wrong_kind_path, None, &wrong_kind)
            .unwrap_err()
            .to_string()
            .contains("as StateSeed, but it is RouteGraph")
    );
    assert!(!root.join(wrong_kind_path).exists());
    store
        .save_asset(Path::new("layouts/ordon.json"), None, &layout)
        .unwrap();
    assert_eq!(
        store.inbound_references("graph.ordon").unwrap()[0].id,
        "layout.ordon"
    );
    let error = store
        .delete_to_trash("graph.ordon", first_revision, false)
        .unwrap_err();
    assert!(error.to_string().contains("layout.ordon"));

    store
        .delete_to_trash("graph.ordon", first_revision, true)
        .unwrap();
    assert!(store.load_asset("graph.ordon").is_err());
    let trash = store.list_trash().unwrap();
    assert_eq!(trash[0].original_relative_path, graph_path);
    assert_eq!(
        store.inbound_references("graph.ordon").unwrap()[0].id,
        "layout.ordon"
    );
    store
        .restore_from_trash("graph.ordon", trash[0].revision_sha256)
        .unwrap();

    let restored_revision = store.load_asset("graph.ordon").unwrap().0.digest().unwrap();
    let renamed_revision = store
        .rename_asset("graph.ordon", "Renamed route", restored_revision)
        .unwrap();
    graph.header.id = "graph.ordon-copy".into();
    graph.header.label = "Ordon route copy".into();
    store
        .duplicate_asset(
            "graph.ordon",
            "graph.ordon-copy",
            "Ordon route copy",
            Path::new("route-graphs/ordon-copy.json"),
        )
        .unwrap();
    assert_eq!(store.list_assets().unwrap().len(), 3);
    store
        .move_asset(
            "graph.ordon",
            Path::new("route-graphs/routes/renamed.json"),
            renamed_revision,
        )
        .unwrap();
    assert_eq!(
        store.load_asset("graph.ordon").unwrap().1,
        Path::new("route-graphs/routes/renamed.json")
    );

    let moved_revision = store.load_asset("graph.ordon").unwrap().0.digest().unwrap();
    store
        .delete_to_trash("graph.ordon", moved_revision, true)
        .unwrap();
    let trash = store.list_trash().unwrap();
    let trashed = trash
        .iter()
        .find(|listing| listing.id == "graph.ordon")
        .unwrap();
    store
        .permanently_delete_from_trash("graph.ordon", trashed.revision_sha256)
        .unwrap();
    assert!(
        store
            .list_trash()
            .unwrap()
            .iter()
            .all(|listing| listing.id != "graph.ordon")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn folder_crud_preserves_identity_remaps_clones_and_trashes_the_subtree() {
    let root = temporary_directory("folder-crud");
    let manifest = WorkspaceManifest::new("workspace.test", "Test workspace").unwrap();
    let store = WorkspaceStore::create(&root, manifest).unwrap();
    let parent_revision = store
        .create_folder(
            "folder.routes",
            "Routes",
            WorkspaceAssetKind::RouteGraph,
            None,
            "routes",
        )
        .unwrap();
    store
        .create_folder(
            "folder.routes.ordon",
            "Ordon",
            WorkspaceAssetKind::RouteGraph,
            Some("folder.routes"),
            "ordon",
        )
        .unwrap();

    let child = graph_asset("graph.child", "Child graph");
    store
        .save_asset(
            Path::new("route-graphs/routes/ordon/child.json"),
            None,
            &child,
        )
        .unwrap();
    let mut parent = graph_asset("graph.parent", "Parent graph");
    parent.references.push(WorkspaceAssetReference {
        asset_id: child.header.id.clone(),
        kind: WorkspaceAssetKind::RouteGraph,
    });
    store
        .save_asset(Path::new("route-graphs/routes/parent.json"), None, &parent)
        .unwrap();
    let layout = WorkspaceAsset {
        schema: WORKSPACE_ASSET_SCHEMA.into(),
        header: WorkspaceAssetHeader {
            id: "layout.external".into(),
            label: "External layout".into(),
            kind: WorkspaceAssetKind::Layout,
            version: 1,
            origin: None,
        },
        references: vec![WorkspaceAssetReference {
            asset_id: parent.header.id.clone(),
            kind: WorkspaceAssetKind::RouteGraph,
        }],
        payload: WorkspaceAssetPayload::Layout(LayoutAsset {
            semantic_asset_id: parent.header.id.clone(),
            positions: BTreeMap::new(),
            viewport: None,
        }),
    };
    store
        .save_asset(Path::new("layouts/external.json"), None, &layout)
        .unwrap();

    let renamed_revision = store
        .rename_folder(
            "folder.routes",
            "Main routes",
            "main-routes",
            parent_revision,
        )
        .unwrap();
    assert_ne!(renamed_revision, parent_revision);
    assert_eq!(
        store.load_folder("folder.routes").unwrap().1,
        Path::new("route-graphs/main-routes")
    );

    store
        .duplicate_folder(
            "folder.routes",
            "folder.routes-copy",
            "Routes copy",
            None,
            "routes-copy",
        )
        .unwrap();
    let cloned_assets = store
        .list_assets()
        .unwrap()
        .into_iter()
        .filter(|asset| asset.relative_path.starts_with("route-graphs/routes-copy"))
        .collect::<Vec<_>>();
    assert_eq!(cloned_assets.len(), 2);
    let cloned_parent = cloned_assets
        .iter()
        .find(|asset| asset.label == "Parent graph")
        .unwrap();
    let cloned_child = cloned_assets
        .iter()
        .find(|asset| asset.label == "Child graph")
        .unwrap();
    let cloned_parent_asset = store.load_asset(&cloned_parent.id).unwrap().0;
    assert_eq!(cloned_parent_asset.references[0].asset_id, cloned_child.id);
    assert_ne!(cloned_parent.id, parent.header.id);

    let error = store
        .delete_folder_to_trash("folder.routes", renamed_revision, false)
        .unwrap_err();
    assert!(error.to_string().contains("layout.external"));
    store
        .delete_folder_to_trash("folder.routes", renamed_revision, true)
        .unwrap();
    assert!(store.load_folder("folder.routes").is_err());
    assert!(store.load_asset("graph.parent").is_err());
    let trash = store.list_folder_trash().unwrap();
    assert_eq!(trash.len(), 1);
    assert_eq!(trash[0].folder_count, 2);
    assert_eq!(trash[0].asset_count, 2);
    store
        .restore_folder_from_trash("folder.routes", trash[0].revision_sha256)
        .unwrap();
    assert_eq!(
        store.load_asset("graph.child").unwrap().1,
        Path::new("route-graphs/main-routes/ordon/child.json")
    );

    let restored_revision = store
        .load_folder("folder.routes")
        .unwrap()
        .0
        .digest()
        .unwrap();
    store
        .delete_folder_to_trash("folder.routes", restored_revision, true)
        .unwrap();
    let trash = store.list_folder_trash().unwrap();
    store
        .permanently_delete_folder_from_trash("folder.routes", trash[0].revision_sha256)
        .unwrap();
    assert!(store.list_folder_trash().unwrap().is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn folder_move_rejects_cross_type_and_descendant_parents() {
    let root = temporary_directory("folder-move-validation");
    let manifest = WorkspaceManifest::new("workspace.test", "Test workspace").unwrap();
    let store = WorkspaceStore::create(&root, manifest).unwrap();
    let parent_revision = store
        .create_folder(
            "folder.parent",
            "Parent",
            WorkspaceAssetKind::RouteGraph,
            None,
            "parent",
        )
        .unwrap();
    store
        .create_folder(
            "folder.child",
            "Child",
            WorkspaceAssetKind::RouteGraph,
            Some("folder.parent"),
            "child",
        )
        .unwrap();
    store
        .create_folder(
            "folder.layouts",
            "Layouts",
            WorkspaceAssetKind::Layout,
            None,
            "layouts",
        )
        .unwrap();

    assert!(
        store
            .move_folder("folder.parent", Some("folder.child"), parent_revision)
            .unwrap_err()
            .to_string()
            .contains("descendant")
    );
    assert!(
        store
            .move_folder("folder.parent", Some("folder.layouts"), parent_revision)
            .unwrap_err()
            .to_string()
            .contains("cannot be placed")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn trashed_child_restores_below_its_stable_parent_after_parent_move() {
    let root = temporary_directory("folder-restore-parent-move");
    let manifest = WorkspaceManifest::new("workspace.test", "Test workspace").unwrap();
    let store = WorkspaceStore::create(&root, manifest).unwrap();
    let parent_revision = store
        .create_folder(
            "folder.parent",
            "Parent",
            WorkspaceAssetKind::RouteGraph,
            None,
            "parent",
        )
        .unwrap();
    let child_revision = store
        .create_folder(
            "folder.child",
            "Child",
            WorkspaceAssetKind::RouteGraph,
            Some("folder.parent"),
            "child",
        )
        .unwrap();
    store
        .create_folder(
            "folder.archive",
            "Archive",
            WorkspaceAssetKind::RouteGraph,
            None,
            "archive",
        )
        .unwrap();
    store
        .delete_folder_to_trash("folder.child", child_revision, false)
        .unwrap();
    store
        .move_folder("folder.parent", Some("folder.archive"), parent_revision)
        .unwrap();
    let trash = store.list_folder_trash().unwrap();
    store
        .restore_folder_from_trash("folder.child", trash[0].revision_sha256)
        .unwrap();

    assert_eq!(
        store.load_folder("folder.child").unwrap().1,
        Path::new("route-graphs/archive/parent/child")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn open_recovers_completed_folder_restore_and_abandoned_clone_staging() {
    let root = temporary_directory("folder-operation-recovery");
    let manifest = WorkspaceManifest::new("workspace.test", "Test workspace").unwrap();
    let store = WorkspaceStore::create(&root, manifest).unwrap();
    let revision = store
        .create_folder(
            "folder.routes",
            "Routes",
            WorkspaceAssetKind::RouteGraph,
            None,
            "routes",
        )
        .unwrap();
    store
        .delete_folder_to_trash("folder.routes", revision, false)
        .unwrap();
    let group = store.folder_trash_group("folder.routes").unwrap();
    fs::rename(
        group.join(FOLDER_TRASH_PAYLOAD),
        root.join("route-graphs/routes"),
    )
    .unwrap();
    let abandoned = root.join(FOLDER_STAGING_ROOT).join("folder-abandoned");
    fs::create_dir(&abandoned).unwrap();
    fs::write(abandoned.join(FOLDER_MARKER_FILE), b"incomplete").unwrap();
    drop(store);

    let recovered = WorkspaceStore::open(&root, &BTreeMap::new()).unwrap();
    assert!(recovered.load_folder("folder.routes").is_ok());
    assert!(recovered.list_folder_trash().unwrap().is_empty());
    assert!(!abandoned.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn registry_export_import_round_trips_independent_assets() {
    let root = temporary_directory("workspace-export-import");
    let registry = WorkspaceRegistry::open(root.join("workspaces"), BTreeMap::new()).unwrap();
    registry
        .create(WorkspaceCreateRequest {
            schema: WORKSPACE_CREATE_SCHEMA.into(),
            id: "source".into(),
            label: "Source workspace".into(),
        })
        .unwrap();
    registry
        .command_folder(
            "source",
            "folder.routes",
            WorkspaceFolderCommandRequest {
                schema: WORKSPACE_FOLDER_COMMAND_SCHEMA.into(),
                command: WorkspaceFolderCommand::Create {
                    id: "folder.routes".into(),
                    label: "Routes".into(),
                    asset_kind: WorkspaceAssetKind::RouteGraph,
                    parent_id: None,
                    directory_name: "routes".into(),
                },
            },
        )
        .unwrap();
    let asset = graph_asset("graph.ordon", "Ordon route");
    registry
        .save_asset(
            "source",
            "graph.ordon",
            WorkspaceAssetSaveRequest {
                schema: WORKSPACE_ASSET_SAVE_SCHEMA.into(),
                relative_path: "route-graphs/routes/ordon.json".into(),
                expected_revision_sha256: None,
                asset: asset.clone(),
            },
        )
        .unwrap();

    let mut bundle = registry.export("source").unwrap();
    assert_eq!(bundle.schema, WORKSPACE_EXPORT_SCHEMA);
    assert_eq!(bundle.folders.len(), 1);
    assert_eq!(bundle.folders[0].folder.id, "folder.routes");
    assert_eq!(bundle.assets.len(), 1);
    assert_eq!(
        bundle.assets[0].relative_path,
        Path::new("route-graphs/routes/ordon.json")
    );
    assert_eq!(bundle.assets[0].asset, asset);

    bundle.manifest.id = "imported".into();
    bundle.manifest.label = "Imported workspace".into();
    let imported = registry.import(bundle.clone()).unwrap();
    assert_eq!(imported.manifest.id, "imported");
    assert_eq!(imported.folders.len(), 1);
    assert_eq!(imported.folders[0].id, "folder.routes");
    assert_eq!(imported.assets.len(), 1);
    assert_eq!(imported.assets[0].id, "graph.ordon");
    assert_eq!(
        imported.assets[0].relative_path,
        Path::new("route-graphs/routes/ordon.json")
    );
    assert_eq!(
        registry
            .load_asset("imported", "graph.ordon")
            .unwrap()
            .asset,
        asset
    );
    assert!(
        registry
            .import(bundle)
            .unwrap_err()
            .to_string()
            .contains("already exists")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn registry_import_rejects_duplicate_paths_before_creating_workspace() {
    let root = temporary_directory("workspace-import-collision");
    let registry = WorkspaceRegistry::open(root.join("workspaces"), BTreeMap::new()).unwrap();
    let manifest = WorkspaceManifest::new("collision", "Collision").unwrap();
    let bundle = WorkspaceExport {
        schema: WORKSPACE_EXPORT_SCHEMA.into(),
        manifest,
        folders: Vec::new(),
        assets: vec![
            WorkspaceExportAsset {
                relative_path: "route-graphs/same.json".into(),
                asset: graph_asset("graph.first", "First"),
            },
            WorkspaceExportAsset {
                relative_path: "route-graphs/same.json".into(),
                asset: graph_asset("graph.second", "Second"),
            },
        ],
    };

    let error = registry.import(bundle).unwrap_err();
    assert!(error.to_string().contains("duplicates asset path"));
    assert!(!root.join("workspaces/collision").exists());
    fs::remove_dir_all(root).unwrap();
}
