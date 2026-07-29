use super::*;

fn temporary_repository(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let repository = std::env::temp_dir().join(format!(
        "dusklight-workspace-{name}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(repository.join("routes")).unwrap();
    repository
}

#[test]
fn checked_workspace_projects_real_folders_and_hides_route_internals() {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../..")
        .canonicalize()
        .unwrap();
    let catalog = load_project_catalog(&repository).unwrap();
    assert!(
        catalog
            .entries
            .contains_key("routes/Glitch Exhibition/intro")
    );
    assert_eq!(
        catalog.entries["routes/Glitch Exhibition/intro"].kind,
        ProjectKind::Timeline
    );
    assert!(
        !catalog
            .entries
            .contains_key("routes/Glitch Exhibition/intro/segments/tolink/01-title-ready")
    );
    assert!(catalog.groups.contains_key("routes"));
    assert!(catalog.groups.contains_key("routes/Glitch Exhibition"));
    assert!(
        !catalog
            .groups
            .contains_key("routes/Glitch Exhibition/intro")
    );
}

#[test]
fn route_private_storage_is_hidden_and_rejected_by_workspace_crud() {
    let repository = temporary_repository("private-route-storage");
    fs::write(
        repository.join("routes/private.timeline"),
        "timeline private\n",
    )
    .unwrap();
    fs::create_dir_all(repository.join("routes/private/segments")).unwrap();
    fs::create_dir_all(repository.join("routes/private/variants")).unwrap();

    let catalog = load_project_catalog(&repository).unwrap();
    assert!(catalog.entries.contains_key("routes/private"));
    assert!(!catalog.groups.contains_key("routes/private"));
    assert!(!catalog.groups.contains_key("routes/private/segments"));
    assert!(!catalog.groups.contains_key("routes/private/variants"));

    let error = workspace_node_sources(
        &repository,
        "routes/private/segments",
        WorkspaceNodeKind::Folder,
    )
    .unwrap_err();
    assert!(error.to_string().contains("private workspace folder"));
    let error = create_workspace_folder(
        &repository,
        &BrowserWorkspaceFolderCreateRequest {
            parent: "routes/private".into(),
            name: "forged-child".into(),
        },
    )
    .unwrap_err();
    assert!(error.to_string().contains("private workspace folder"));

    fs::remove_dir_all(repository).unwrap();
}

#[test]
fn every_workspace_entry_loads_or_parses() {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../..")
        .canonicalize()
        .unwrap();
    let catalog = load_project_catalog(&repository).unwrap();
    for entry in catalog.entries.values() {
        match entry.kind {
            ProjectKind::Timeline => {
                load_authoritative_timeline(&repository.join(&entry.artifact)).unwrap();
            }
            ProjectKind::Tas | ProjectKind::Tape => {
                load_project_tape(&repository, entry).unwrap();
                load_project_native_oracle(&repository, entry).unwrap();
            }
        }
    }
}

#[test]
fn boot_overrides_and_workspace_crud_move_artifacts_as_one_git_unit() {
    let repository = temporary_repository("crud");
    fs::create_dir_all(repository.join("routes/qa")).unwrap();
    fs::create_dir_all(repository.join("routes/empty")).unwrap();
    fs::write(
        repository.join("routes/qa/canary.tape"),
        InputTape::default().encode().unwrap(),
    )
    .unwrap();
    fs::write(
        repository.join("routes/qa/canary.launch"),
        "dusklaunch 1\noracle eye_shredder\n",
    )
    .unwrap();

    let catalog = load_project_catalog(&repository).unwrap();
    assert!(catalog.groups.contains_key("routes/empty"));
    assert!(catalog.entries.contains_key("routes/qa/canary"));

    let stage_boot = TapeBoot::Stage {
        stage: "F_SP103".into(),
        room: 1,
        point: 0,
        layer: -1,
        save_slot: None,
        fixture: None,
    };
    update_boot_override(
        &repository,
        &BrowserBootOverrideUpdateRequest {
            project: "routes/qa/canary".into(),
            enabled: true,
            boot: stage_boot.clone(),
        },
    )
    .unwrap();
    let materialized = project_materialized_playback(&repository, "routes/qa/canary").unwrap();
    assert!(matches!(
        materialized.tape.boot,
        TapeBoot::Stage { ref stage, .. } if stage == "F_SP103"
    ));
    assert_eq!(
        materialized.native_oracle,
        NativePlaybackOracle::EyeShredder
    );

    create_workspace_folder(
        &repository,
        &BrowserWorkspaceFolderCreateRequest {
            parent: "routes".into(),
            name: "moved".into(),
        },
    )
    .unwrap();
    let mut active_timeline = repository.join("not-active.timeline");
    let moved = move_workspace_node(
        &repository,
        &mut active_timeline,
        &BrowserWorkspaceMoveRequest {
            id: "routes/qa/canary".into(),
            kind: WorkspaceNodeKind::Project,
            destination: "routes/moved".into(),
        },
    )
    .unwrap();
    assert_eq!(moved.id, "routes/moved/canary");
    assert_eq!(moved.destination.as_deref(), Some("routes/moved"));
    assert!(repository.join("routes/moved/canary.tape").is_file());
    assert!(repository.join("routes/moved/canary.boot.json").is_file());
    assert!(repository.join("routes/moved/canary.launch").is_file());
    assert!(!repository.join("routes/qa/canary.tape").exists());

    let state_root = repository.join("state");
    let deletion = delete_workspace_node(
        &repository,
        &repository.join("not-active.timeline"),
        &state_root,
        &BrowserWorkspaceDeleteRequest {
            id: "routes/moved/canary".into(),
            kind: WorkspaceNodeKind::Project,
        },
    )
    .unwrap();
    let trash = deletion.trash.unwrap();
    assert!(trash.join("canary.tape").is_file());
    assert!(trash.join("canary.boot.json").is_file());
    assert!(trash.join("canary.launch").is_file());
    assert!(!repository.join("routes/moved/canary.tape").exists());
    fs::remove_dir_all(repository).unwrap();
}

#[test]
fn workspace_rejects_moving_a_folder_into_its_descendant() {
    let repository = temporary_repository("cycle");
    fs::create_dir_all(repository.join("routes/a/b")).unwrap();
    let mut active_timeline = repository.join("not-active.timeline");
    let error = move_workspace_node(
        &repository,
        &mut active_timeline,
        &BrowserWorkspaceMoveRequest {
            id: "routes/a".into(),
            kind: WorkspaceNodeKind::Folder,
            destination: "routes/a/b".into(),
        },
    )
    .unwrap_err();
    assert!(error.to_string().contains("cannot be moved into itself"));
    fs::remove_dir_all(repository).unwrap();
}

#[test]
fn workspace_moves_a_folder_to_an_existing_sibling_and_returns_its_new_identity() {
    let repository = temporary_repository("move-folder");
    fs::create_dir_all(repository.join("routes/source/nested")).unwrap();
    fs::create_dir_all(repository.join("routes/destination")).unwrap();
    fs::write(
        repository.join("routes/source/nested/canary.tas"),
        NEW_TAPE_SOURCE,
    )
    .unwrap();

    let mut active_timeline = repository.join("not-active.timeline");
    let moved = move_workspace_node(
        &repository,
        &mut active_timeline,
        &BrowserWorkspaceMoveRequest {
            id: "routes/source".into(),
            kind: WorkspaceNodeKind::Folder,
            destination: "routes/destination".into(),
        },
    )
    .unwrap();

    assert_eq!(moved.id, "routes/destination/source");
    assert_eq!(moved.destination.as_deref(), Some("routes/destination"));
    assert!(
        repository
            .join("routes/destination/source/nested/canary.tas")
            .is_file()
    );
    assert!(!repository.join("routes/source").exists());
    fs::remove_dir_all(repository).unwrap();
}

#[test]
fn workspace_moves_the_open_timeline_and_updates_the_live_server_path() {
    let repository = temporary_repository("move-active-timeline");
    fs::create_dir_all(repository.join("routes/destination")).unwrap();
    fs::create_dir_all(repository.join("routes/active/segments")).unwrap();
    fs::write(
        repository.join("routes/active.timeline"),
        "timeline active\n",
    )
    .unwrap();
    fs::write(
        repository.join("routes/active/segments/marker"),
        b"private route data",
    )
    .unwrap();
    let mut active_timeline = repository.join("routes/active.timeline");

    let moved = move_workspace_node(
        &repository,
        &mut active_timeline,
        &BrowserWorkspaceMoveRequest {
            id: "routes/active".into(),
            kind: WorkspaceNodeKind::Project,
            destination: "routes/destination".into(),
        },
    )
    .unwrap();

    assert_eq!(moved.id, "routes/destination/active");
    assert_eq!(
        active_timeline,
        fs::canonicalize(repository.join("routes/destination/active.timeline")).unwrap()
    );
    assert!(active_timeline.is_file());
    assert!(
        repository
            .join("routes/destination/active/segments/marker")
            .is_file()
    );
    assert!(!repository.join("routes/active.timeline").exists());
    fs::remove_dir_all(repository).unwrap();
}

#[test]
fn workspace_creates_playable_tapes_and_clones_every_sidecar() {
    let repository = temporary_repository("create-clone");
    fs::create_dir_all(repository.join("routes/QA")).unwrap();
    let created = create_workspace_tape(
        &repository,
        &BrowserWorkspaceTapeCreateRequest {
            parent: "routes/QA".into(),
            name: "Blank Boot".into(),
        },
    )
    .unwrap();
    assert_eq!(created.id, "routes/QA/Blank Boot");
    let catalog = load_project_catalog(&repository).unwrap();
    let entry = &catalog.entries[&created.id];
    let tape = load_project_tape(&repository, entry).unwrap();
    assert_eq!(tape.frames.len(), 1);

    let invalid_boot: TapeBoot = serde_json::from_value(serde_json::json!({
        "kind": "stage",
        "stage": "F_SP103",
        "room": 1,
        "point": 0,
        "layer": -1,
        "fixture": {
            "schema": "dusklight-scenario-fixture/v1",
            "name": "invalid native slot",
            "inventory": [{"slot": 24, "item": 64, "quantity": 1}]
        }
    }))
    .unwrap();
    let error = update_boot_override(
        &repository,
        &BrowserBootOverrideUpdateRequest {
            project: created.id.clone(),
            enabled: true,
            boot: invalid_boot,
        },
    )
    .unwrap_err();
    assert!(error.to_string().contains("native save limits"));

    let boot = br#"{"schema":"dusklight.route-workbench.boot-override.v1","enabled":false,"boot":{"kind":"process"}}"#;
    let launch = b"dusklaunch 1\noracle eye_shredder\n";
    fs::write(repository.join("routes/QA/Blank Boot.boot.json"), boot).unwrap();
    fs::write(repository.join("routes/QA/Blank Boot.launch"), launch).unwrap();
    let cloned = clone_workspace_tape(
        &repository,
        &BrowserWorkspaceTapeCloneRequest {
            source: created.id,
            destination: "routes/QA".into(),
            name: "Configured Copy".into(),
        },
    )
    .unwrap();
    assert_eq!(cloned.id, "routes/QA/Configured Copy");
    assert_eq!(
        fs::read(repository.join("routes/QA/Configured Copy.boot.json")).unwrap(),
        boot
    );
    assert_eq!(
        fs::read(repository.join("routes/QA/Configured Copy.launch")).unwrap(),
        launch
    );
    assert!(repository.join("routes/QA/Configured Copy.tas").is_file());

    fs::remove_dir_all(repository).unwrap();
}

#[test]
fn workspace_folder_names_preserve_human_casing_and_spaces() {
    for valid in ["QA", "Intro Segments", "Glitch-Hunt_01", "Élite Routes"] {
        validate_workspace_name(valid).unwrap();
    }
    for invalid in [
        "",
        ".",
        "..",
        " trailing ",
        "trailing.",
        "a/b",
        r"a\b",
        "CON",
        "con.txt",
        "LPT9",
    ] {
        assert!(
            validate_workspace_name(invalid).is_err(),
            "accepted unsafe workspace folder {invalid:?}"
        );
    }
}
