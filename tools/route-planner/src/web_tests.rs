
use super::*;
use crate::service::PlannerServiceRequest;
use dusklight_route_planner::logic::{FACT_CATALOG_SCHEMA, FactCatalog};
use dusklight_route_planner::transition::{MECHANICS_CATALOG_SCHEMA, MechanicsCatalog};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_TEST_ROOT: AtomicU64 = AtomicU64::new(0);

fn state() -> (WebState, PathBuf) {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "dusklight-route-web-{}-{nonce}-{}",
        std::process::id(),
        NEXT_TEST_ROOT.fetch_add(1, Ordering::Relaxed),
    ));
    let projects = ProjectStore::open(&root).unwrap();
    let available_libraries = builtin_library_digests(&projects).unwrap();
    let state = WebState {
        projects: Arc::new(Mutex::new(projects)),
        workspaces: Arc::new(Mutex::new(
            WorkspaceRegistry::open(root.join("workspaces"), available_libraries).unwrap(),
        )),
    };
    (state, root)
}

#[test]
fn static_assets_and_health_are_local_and_cacheless() {
    let (state, root) = state();
    let index = dispatch(
        HttpRequest {
            method: "GET".into(),
            target: "/".into(),
            body: Vec::new(),
        },
        &state,
    );
    assert_eq!(index.status, 200);
    assert_eq!(index.content_type, "text/html; charset=utf-8");
    assert!(
        index
            .body
            .windows(13)
            .any(|window| window == b"Route Planner")
    );
    let index_text = String::from_utf8(index.body).unwrap();
    assert!(index_text.contains("id=\"empty-primary\""));
    assert!(index_text.contains("id=\"folder-dialog\""));
    assert!(index_text.contains("References use this identity, not the directory path."));
    assert!(index_text.contains("Legacy file migration"));
    assert!(!index_text.contains("New legacy project"));
    let app = dispatch(
        HttpRequest {
            method: "GET".into(),
            target: "/app.js".into(),
            body: Vec::new(),
        },
        &state,
    );
    assert_eq!(app.status, 200);
    let app_text = String::from_utf8(app.body).unwrap();
    assert!(app_text.contains("evaluate_transition"));
    assert!(app_text.contains("Choose context and anchor"));
    for required in [
        "workspace-scenario-create/v1",
        "openNewScenarioDialog",
        "Empty grounded scenario created from selected context, anchor, and goal",
        "workspace-folder-command/v1",
        "workspace-folder-trash-command/v1",
        "workspaceFolderItem",
        "Folder subtree moved to Trash as one recoverable group",
        "populateFolderParentChoices",
        "Clone the subtree with new stable identities",
        "/folder-trash/",
    ] {
        assert!(
            app_text.contains(required),
            "missing browser contract {required}"
        );
    }

    let health = dispatch(
        HttpRequest {
            method: "GET".into(),
            target: "/api/health".into(),
            body: Vec::new(),
        },
        &state,
    );
    assert_eq!(health.status, 200);
    assert!(
        health
            .body
            .windows(13)
            .any(|window| window == b"\"status\":\"ok\"")
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn typed_service_rejects_an_unknown_protocol_through_http() {
    let (state, root) = state();
    let envelope = PlannerServiceEnvelope {
        schema: "dusklight.route-planner.service/v999".into(),
        request: PlannerServiceRequest::Compose {
            request_id: "web-test".into(),
            facts: Box::new(FactCatalog {
                schema: FACT_CATALOG_SCHEMA.into(),
                aliases: Vec::new(),
                derived_facts: Vec::new(),
            }),
            mechanics: Box::new(MechanicsCatalog {
                schema: MECHANICS_CATALOG_SCHEMA.into(),
                transitions: Vec::new(),
                obligations: Vec::new(),
                writers: Vec::new(),
                gates: Vec::new(),
                readers: Vec::new(),
                reconstruction_rules: Vec::new(),
                obstructions: Vec::new(),
                resolvers: Vec::new(),
                techniques: Vec::new(),
                microtraces: Vec::new(),
                goals: Vec::new(),
            }),
            packs: Vec::new(),
            route_local_overlays: Vec::new(),
            ephemeral_what_if_overlays: Vec::new(),
        },
    };
    let response = dispatch(
        HttpRequest {
            method: "POST".into(),
            target: "/api/service".into(),
            body: serde_json::to_vec(&envelope).unwrap(),
        },
        &state,
    );
    assert_eq!(response.status, 200);
    let body: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(body["outcome"]["status"], "error");
    assert_eq!(body["outcome"]["field"], "schema");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn project_http_lists_loads_saves_and_rejects_stale_revisions() {
    let (state, root) = state();
    let list = dispatch(
        HttpRequest {
            method: "GET".into(),
            target: "/api/projects".into(),
            body: Vec::new(),
        },
        &state,
    );
    assert_eq!(list.status, 200);
    let list: serde_json::Value = serde_json::from_slice(&list.body).unwrap();
    assert_eq!(list["projects"].as_array().unwrap().len(), 6);

    let template = dispatch(
        HttpRequest {
            method: "GET".into(),
            target: "/api/project-template".into(),
            body: Vec::new(),
        },
        &state,
    );
    let mut record: serde_json::Value = serde_json::from_slice(&template.body).unwrap();
    record["project"]["id"] = "http-route".into();
    record["project"]["label"] = "HTTP route".into();
    let request = serde_json::json!({
        "schema": crate::project::WEB_PROJECT_SAVE_SCHEMA,
        "expected_revision_sha256": null,
        "project": record["project"],
    });
    let saved = dispatch(
        HttpRequest {
            method: "PUT".into(),
            target: "/api/projects/http-route".into(),
            body: serde_json::to_vec(&request).unwrap(),
        },
        &state,
    );
    assert_eq!(saved.status, 200);
    let conflict = dispatch(
        HttpRequest {
            method: "PUT".into(),
            target: "/api/projects/http-route".into(),
            body: serde_json::to_vec(&request).unwrap(),
        },
        &state,
    );
    assert_eq!(conflict.status, 400);
    assert!(
        String::from_utf8(conflict.body)
            .unwrap()
            .contains("revision conflict")
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn workspace_http_creates_lists_and_loads_file_backed_workspaces() {
    let (state, root) = state();
    let create = serde_json::json!({
        "schema": crate::workspace::WORKSPACE_CREATE_SCHEMA,
        "id": "ordon-route",
        "label": "Ordon route",
    });
    let created = dispatch(
        HttpRequest {
            method: "POST".into(),
            target: "/api/workspaces".into(),
            body: serde_json::to_vec(&create).unwrap(),
        },
        &state,
    );
    assert_eq!(created.status, 200);
    let created: serde_json::Value = serde_json::from_slice(&created.body).unwrap();
    assert_eq!(created["manifest"]["id"], "ordon-route");
    assert!(created["manifest"].get("catalog").is_none());
    assert_eq!(created["assets"].as_array().unwrap().len(), 0);

    let list = dispatch(
        HttpRequest {
            method: "GET".into(),
            target: "/api/workspaces".into(),
            body: Vec::new(),
        },
        &state,
    );
    let list: serde_json::Value = serde_json::from_slice(&list.body).unwrap();
    assert_eq!(list["workspaces"].as_array().unwrap().len(), 1);
    assert_eq!(list["workspaces"][0]["label"], "Ordon route");

    let loaded = dispatch(
        HttpRequest {
            method: "GET".into(),
            target: "/api/workspaces/ordon-route".into(),
            body: Vec::new(),
        },
        &state,
    );
    assert_eq!(loaded.status, 200);
    let loaded: serde_json::Value = serde_json::from_slice(&loaded.body).unwrap();
    assert_eq!(loaded["manifest"]["id"], "ordon-route");
    assert!(root.join("workspaces/ordon-route/workspace.json").is_file());

    let referenced = dispatch(
        HttpRequest {
            method: "POST".into(),
            target: "/api/workspaces/ordon-route/library-references/demo-forest-keyed-door".into(),
            body: Vec::new(),
        },
        &state,
    );
    assert_eq!(referenced.status, 200);
    let referenced: serde_json::Value = serde_json::from_slice(&referenced.body).unwrap();
    assert_eq!(
        referenced["manifest"]["mounted_libraries"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(referenced["assets"].as_array().unwrap().is_empty());

    let imported = dispatch(
        HttpRequest {
            method: "POST".into(),
            target: "/api/workspaces/ordon-route/library-scenarios/demo-forest-keyed-door".into(),
            body: Vec::new(),
        },
        &state,
    );
    assert_eq!(
        imported.status,
        200,
        "{}",
        String::from_utf8_lossy(&imported.body)
    );
    let imported: serde_json::Value = serde_json::from_slice(&imported.body).unwrap();
    assert_eq!(
        imported["manifest"]["mounted_libraries"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    for kind in ["scenario", "route_graph", "state_seed", "layout"] {
        assert!(
            imported["assets"]
                .as_array()
                .unwrap()
                .iter()
                .any(|asset| asset["kind"] == kind),
            "missing imported {kind}"
        );
    }
    assert!(
        imported["assets"]
            .as_array()
            .unwrap()
            .iter()
            .all(|asset| asset["id"] == "custom.roll"
                || asset["kind"] == "route_book"
                || asset.get("revision_sha256").is_some())
    );
    let asset_listing = |kind: &str| {
        imported["assets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|asset| asset["kind"] == kind)
            .unwrap()
            .clone()
    };
    let graph_listing = asset_listing("route_graph");
    let route_book_listing = asset_listing("route_book");
    let layout_listing = asset_listing("layout");
    let route_book_asset = dispatch(
        HttpRequest {
            method: "GET".into(),
            target: format!(
                "/api/workspaces/ordon-route/assets/{}",
                route_book_listing["id"].as_str().unwrap()
            ),
            body: Vec::new(),
        },
        &state,
    );
    let route_book_asset: serde_json::Value =
        serde_json::from_slice(&route_book_asset.body).unwrap();
    let mut route_book = route_book_asset["asset"]["payload"]["route_book"].clone();
    route_book["manifest"]["author"] = "Browser route author".into();
    let save_route = serde_json::json!({
        "schema": crate::workspace::WORKSPACE_ROUTE_GRAPH_SAVE_SCHEMA,
        "expected_graph_revision_sha256": graph_listing["revision_sha256"],
        "route_book_id": route_book_listing["id"],
        "expected_route_book_revision_sha256": route_book_listing["revision_sha256"],
        "route_book": route_book,
        "layout": {
            "asset_id": layout_listing["id"],
            "expected_revision_sha256": layout_listing["revision_sha256"],
            "positions": {
                "node.authored": { "x": 320.0, "y": 180.0 }
            },
            "viewport": { "x": 12.0, "y": 24.0, "zoom": 1.25 }
        }
    });
    let route_saved = dispatch(
        HttpRequest {
            method: "POST".into(),
            target: format!(
                "/api/workspaces/ordon-route/route-graphs/{}",
                graph_listing["id"].as_str().unwrap()
            ),
            body: serde_json::to_vec(&save_route).unwrap(),
        },
        &state,
    );
    assert_eq!(route_saved.status, 200);
    let route_saved: serde_json::Value = serde_json::from_slice(&route_saved.body).unwrap();
    assert_eq!(
        route_saved["schema"],
        crate::workspace::WORKSPACE_ROUTE_GRAPH_EDIT_RECORD_SCHEMA
    );
    assert_eq!(
        route_saved["route_book"]["asset"]["payload"]["route_book"]["manifest"]["author"],
        "Browser route author"
    );
    assert_eq!(
        route_saved["layout"]["asset"]["payload"]["positions"]["node.authored"]["x"],
        320.0
    );
    assert!(
        route_saved["graph"]["asset"]["payload"]["graph"]["route_book_sha256"]
            .as_str()
            .is_some()
    );

    let fork = serde_json::json!({
        "schema": crate::workspace::WORKSPACE_LIBRARY_FORK_SCHEMA,
        "namespace": "forest-alternate",
    });
    let forked = dispatch(
        HttpRequest {
            method: "POST".into(),
            target: "/api/workspaces/ordon-route/library-forks/demo-forest-keyed-door".into(),
            body: serde_json::to_vec(&fork).unwrap(),
        },
        &state,
    );
    assert_eq!(forked.status, 200);
    let forked: serde_json::Value = serde_json::from_slice(&forked.body).unwrap();
    assert!(forked["assets"].as_array().unwrap().iter().any(|asset| {
        asset["id"] == "route-graph.forest-alternate"
            && asset["kind"] == "route_graph"
            && asset.get("revision_sha256").is_some()
    }));
    let forked_graph = dispatch(
        HttpRequest {
            method: "GET".into(),
            target: "/api/workspaces/ordon-route/assets/route-graph.forest-alternate".into(),
            body: Vec::new(),
        },
        &state,
    );
    assert_eq!(forked_graph.status, 200);
    let forked_graph: serde_json::Value = serde_json::from_slice(&forked_graph.body).unwrap();
    let source_library = dispatch(
        HttpRequest {
            method: "GET".into(),
            target: "/api/projects/demo-forest-keyed-door".into(),
            body: Vec::new(),
        },
        &state,
    );
    let source_library: serde_json::Value = serde_json::from_slice(&source_library.body).unwrap();
    let origin = &forked_graph["asset"]["header"]["origin"];
    assert_eq!(origin["library_id"], "demo-forest-keyed-door");
    assert_eq!(
        origin["library_version"],
        crate::workspace::BUILTIN_LIBRARY_VERSION
    );
    assert_eq!(origin["library_sha256"], source_library["revision_sha256"]);
    assert_eq!(origin["source_asset_id"], "demo-forest-keyed-door:graph");

    let goal_id = source_library["project"]["catalog"]["mechanics"]["goals"][0]["id"]
        .as_str()
        .unwrap();
    let scenario_create = serde_json::json!({
        "schema": crate::workspace::WORKSPACE_SCENARIO_CREATE_SCHEMA,
        "library_id": "demo-forest-keyed-door",
        "namespace": "blank-forest-route",
        "label": "Blank forest route",
        "goal_id": goal_id,
    });
    let scenario_created = dispatch(
        HttpRequest {
            method: "POST".into(),
            target: "/api/workspaces/ordon-route/scenarios".into(),
            body: serde_json::to_vec(&scenario_create).unwrap(),
        },
        &state,
    );
    assert_eq!(scenario_created.status, 200);
    let scenario_created: serde_json::Value =
        serde_json::from_slice(&scenario_created.body).unwrap();
    for (id, kind) in [
        ("scenario.blank-forest-route", "scenario"),
        ("route-graph.blank-forest-route", "route_graph"),
        ("state-seed.blank-forest-route", "state_seed"),
        ("route-book.blank-forest-route", "route_book"),
        ("layout.blank-forest-route", "layout"),
    ] {
        assert!(
            scenario_created["assets"]
                .as_array()
                .unwrap()
                .iter()
                .any(|asset| { asset["id"] == id && asset["kind"] == kind })
        );
    }
    let blank_route_book = dispatch(
        HttpRequest {
            method: "GET".into(),
            target: "/api/workspaces/ordon-route/assets/route-book.blank-forest-route".into(),
            body: Vec::new(),
        },
        &state,
    );
    assert_eq!(blank_route_book.status, 200);
    let blank_route_book: serde_json::Value =
        serde_json::from_slice(&blank_route_book.body).unwrap();
    assert_eq!(
        blank_route_book["asset"]["payload"]["route_book"]["goal_ids"],
        serde_json::json!([goal_id])
    );
    assert_eq!(
        blank_route_book["asset"]["payload"]["route_book"]["steps"],
        serde_json::json!([])
    );
    assert_eq!(
        blank_route_book["asset"]["payload"]["route_book"]["methods"],
        serde_json::json!([])
    );
    let blank_scenario = dispatch(
        HttpRequest {
            method: "GET".into(),
            target: "/api/workspaces/ordon-route/assets/scenario.blank-forest-route".into(),
            body: Vec::new(),
        },
        &state,
    );
    assert_eq!(blank_scenario.status, 200);
    let blank_scenario: serde_json::Value = serde_json::from_slice(&blank_scenario.body).unwrap();
    assert_eq!(
        blank_scenario["asset"]["payload"]["anchor"],
        serde_json::json!({
            "kind": "state_seed",
            "state_seed_id": "state-seed.blank-forest-route",
        })
    );
    assert_eq!(
        blank_scenario["asset"]["header"]["origin"]["library_id"],
        "demo-forest-keyed-door"
    );

    let asset = crate::workspace::WorkspaceAsset {
        schema: crate::workspace::WORKSPACE_ASSET_SCHEMA.into(),
        header: crate::workspace::WorkspaceAssetHeader {
            id: "custom.roll".into(),
            label: "Roll".into(),
            kind: crate::workspace::WorkspaceAssetKind::CustomNodeDefinition,
            version: 1,
            origin: None,
        },
        references: Vec::new(),
        payload: crate::workspace::WorkspaceAssetPayload::CustomNodeDefinition(
            crate::workspace::CustomNodeDefinitionAsset {
                inputs: Vec::new(),
                outputs: Vec::new(),
                guard: dusklight_route_planner::logic::PredicateExpression::True,
                effects: Vec::new(),
                evidence_status: crate::workspace::CustomNodeEvidenceStatus::Hypothetical,
                evidence: Vec::new(),
            },
        ),
    };
    let save = crate::workspace::WorkspaceAssetSaveRequest {
        schema: crate::workspace::WORKSPACE_ASSET_SAVE_SCHEMA.into(),
        relative_path: "custom-nodes/roll.json".into(),
        expected_revision_sha256: None,
        asset,
    };
    let saved = dispatch(
        HttpRequest {
            method: "PUT".into(),
            target: "/api/workspaces/ordon-route/assets/custom.roll".into(),
            body: serde_json::to_vec(&save).unwrap(),
        },
        &state,
    );
    assert_eq!(saved.status, 200);
    let saved: serde_json::Value = serde_json::from_slice(&saved.body).unwrap();
    assert_eq!(saved["asset"]["header"]["id"], "custom.roll");
    assert!(
        root.join("workspaces/ordon-route/custom-nodes/roll.json")
            .is_file()
    );
    let loaded_asset = dispatch(
        HttpRequest {
            method: "GET".into(),
            target: "/api/workspaces/ordon-route/assets/custom.roll".into(),
            body: Vec::new(),
        },
        &state,
    );
    assert_eq!(loaded_asset.status, 200);
    let loaded_asset: serde_json::Value = serde_json::from_slice(&loaded_asset.body).unwrap();
    assert_eq!(
        loaded_asset["asset"]["payload"]["evidence_status"],
        "hypothetical"
    );
    let revision = loaded_asset["revision_sha256"].as_str().unwrap();
    let rename = serde_json::json!({
        "schema": crate::workspace::WORKSPACE_ASSET_COMMAND_SCHEMA,
        "command": {
            "kind": "rename",
            "expected_revision_sha256": revision,
            "label": "Roll quickly",
        },
    });
    let renamed = dispatch(
        HttpRequest {
            method: "POST".into(),
            target: "/api/workspaces/ordon-route/assets/custom.roll".into(),
            body: serde_json::to_vec(&rename).unwrap(),
        },
        &state,
    );
    assert_eq!(renamed.status, 200);
    let renamed: serde_json::Value = serde_json::from_slice(&renamed.body).unwrap();
    let renamed_asset = renamed["assets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|asset| asset["id"] == "custom.roll")
        .unwrap();
    assert_eq!(renamed_asset["label"], "Roll quickly");
    let trash = serde_json::json!({
        "schema": crate::workspace::WORKSPACE_ASSET_COMMAND_SCHEMA,
        "command": {
            "kind": "delete_to_trash",
            "expected_revision_sha256": renamed_asset["revision_sha256"],
            "allow_broken_references": false,
        },
    });
    let trashed = dispatch(
        HttpRequest {
            method: "POST".into(),
            target: "/api/workspaces/ordon-route/assets/custom.roll".into(),
            body: serde_json::to_vec(&trash).unwrap(),
        },
        &state,
    );
    assert_eq!(trashed.status, 200);
    let trash_list = dispatch(
        HttpRequest {
            method: "GET".into(),
            target: "/api/workspaces/ordon-route/trash".into(),
            body: Vec::new(),
        },
        &state,
    );
    let trash_list: serde_json::Value = serde_json::from_slice(&trash_list.body).unwrap();
    assert_eq!(trash_list.as_array().unwrap().len(), 1);
    let restore = serde_json::json!({
        "schema": crate::workspace::WORKSPACE_TRASH_COMMAND_SCHEMA,
        "expected_revision_sha256": trash_list[0]["revision_sha256"],
        "command": "restore",
    });
    let restored = dispatch(
        HttpRequest {
            method: "POST".into(),
            target: "/api/workspaces/ordon-route/trash/custom.roll".into(),
            body: serde_json::to_vec(&restore).unwrap(),
        },
        &state,
    );
    assert_eq!(restored.status, 200);
    let restored: serde_json::Value = serde_json::from_slice(&restored.body).unwrap();
    let restored_asset = restored["assets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|asset| asset["id"] == "custom.roll")
        .unwrap();

    let create_folder = serde_json::json!({
        "schema": crate::workspace::WORKSPACE_FOLDER_COMMAND_SCHEMA,
        "command": {
            "kind": "create",
            "id": "folder.custom-research",
            "label": "Research",
            "asset_kind": "custom_node_definition",
            "parent_id": null,
            "directory_name": "research",
        },
    });
    let folder_created = dispatch(
        HttpRequest {
            method: "POST".into(),
            target: "/api/workspaces/ordon-route/folders/folder.custom-research".into(),
            body: serde_json::to_vec(&create_folder).unwrap(),
        },
        &state,
    );
    assert_eq!(folder_created.status, 200);
    let folder_created: serde_json::Value = serde_json::from_slice(&folder_created.body).unwrap();
    let folder = folder_created["folders"]
        .as_array()
        .unwrap()
        .iter()
        .find(|folder| folder["id"] == "folder.custom-research")
        .unwrap();
    let folder_revision = folder["revision_sha256"].clone();

    let move_asset = serde_json::json!({
        "schema": crate::workspace::WORKSPACE_ASSET_COMMAND_SCHEMA,
        "command": {
            "kind": "move",
            "expected_revision_sha256": restored_asset["revision_sha256"],
            "relative_path": "custom-nodes/research/roll.json",
        },
    });
    let moved = dispatch(
        HttpRequest {
            method: "POST".into(),
            target: "/api/workspaces/ordon-route/assets/custom.roll".into(),
            body: serde_json::to_vec(&move_asset).unwrap(),
        },
        &state,
    );
    assert_eq!(moved.status, 200);

    let delete_folder = serde_json::json!({
        "schema": crate::workspace::WORKSPACE_FOLDER_COMMAND_SCHEMA,
        "command": {
            "kind": "delete_to_trash",
            "expected_revision_sha256": folder_revision,
            "allow_broken_references": false,
        },
    });
    let folder_deleted = dispatch(
        HttpRequest {
            method: "POST".into(),
            target: "/api/workspaces/ordon-route/folders/folder.custom-research".into(),
            body: serde_json::to_vec(&delete_folder).unwrap(),
        },
        &state,
    );
    assert_eq!(folder_deleted.status, 200);
    let folder_trash = dispatch(
        HttpRequest {
            method: "GET".into(),
            target: "/api/workspaces/ordon-route/folder-trash".into(),
            body: Vec::new(),
        },
        &state,
    );
    let folder_trash: serde_json::Value = serde_json::from_slice(&folder_trash.body).unwrap();
    assert_eq!(folder_trash[0]["asset_count"], 1);
    let restore_folder = serde_json::json!({
        "schema": crate::workspace::WORKSPACE_FOLDER_TRASH_COMMAND_SCHEMA,
        "expected_revision_sha256": folder_trash[0]["revision_sha256"],
        "command": "restore",
    });
    let folder_restored = dispatch(
        HttpRequest {
            method: "POST".into(),
            target: "/api/workspaces/ordon-route/folder-trash/folder.custom-research".into(),
            body: serde_json::to_vec(&restore_folder).unwrap(),
        },
        &state,
    );
    assert_eq!(folder_restored.status, 200);
    assert!(
        root.join("workspaces/ordon-route/custom-nodes/research/roll.json")
            .is_file()
    );

    let exported = dispatch(
        HttpRequest {
            method: "GET".into(),
            target: "/api/workspaces/ordon-route/export".into(),
            body: Vec::new(),
        },
        &state,
    );
    assert_eq!(exported.status, 200);
    let mut bundle =
        serde_json::from_slice::<crate::workspace::WorkspaceExport>(&exported.body).unwrap();
    assert_eq!(bundle.schema, crate::workspace::WORKSPACE_EXPORT_SCHEMA);
    assert!(bundle.folders.iter().any(|record| {
        record.folder.id == "folder.custom-research"
            && record.relative_path == std::path::Path::new("custom-nodes/research")
    }));
    assert!(bundle.assets.iter().any(|record| {
        record.asset.header.id == "custom.roll"
            && record.relative_path == std::path::Path::new("custom-nodes/research/roll.json")
    }));
    bundle.manifest.id = "ordon-route-copy".into();
    bundle.manifest.label = "Ordon route copy".into();
    let imported_workspace = dispatch(
        HttpRequest {
            method: "POST".into(),
            target: "/api/workspaces/import".into(),
            body: serde_json::to_vec(&bundle).unwrap(),
        },
        &state,
    );
    assert_eq!(imported_workspace.status, 200);
    let imported_workspace: serde_json::Value =
        serde_json::from_slice(&imported_workspace.body).unwrap();
    assert_eq!(imported_workspace["manifest"]["id"], "ordon-route-copy");
    assert_eq!(
        imported_workspace["assets"].as_array().unwrap().len(),
        bundle.assets.len()
    );
    assert!(
        root.join("workspaces/ordon-route-copy/custom-nodes/research/roll.json")
            .is_file()
    );
    std::fs::remove_dir_all(root).unwrap();
}
