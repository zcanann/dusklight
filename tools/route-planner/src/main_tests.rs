use super::*;
use dusklight_route_planner::authorization::AuthorizationGraph;
use dusklight_route_planner::identity::RUNTIME_CONFIGURATION_SCHEMA;
use dusklight_route_planner::logic::FACT_CATALOG_SCHEMA;
use dusklight_route_planner::snapshot::STATE_SNAPSHOT_SCHEMA;
use dusklight_route_planner::state::{
    BackingAttachment, EXECUTION_ENVIRONMENT_SCHEMA, ExecutionContext, ExecutionEnvironment,
    PlayerForm, PlayerState, RuntimeFile, RuntimeFileLifecycle, RuntimeFileOrigin, SceneLocation,
};
use dusklight_route_planner::transition::MECHANICS_CATALOG_SCHEMA;
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn project_authorization_graph_command_writes_a_canonical_base_graph() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = env::temp_dir().join(format!(
        "dusklight-authorization-cli-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir(&root).unwrap();
    let state_path = root.join("state.json");
    let facts_path = root.join("facts.json");
    let mechanics_path = root.join("mechanics.json");
    let output_path = root.join("authorization.json");
    let snapshot = StateSnapshot {
        schema: STATE_SNAPSHOT_SCHEMA.into(),
        id: "snapshot.cli-start".into(),
        sequence: 0,
        environment: ExecutionEnvironment {
            schema: EXECUTION_ENVIRONMENT_SCHEMA.into(),
            runtime_configuration: RuntimeConfiguration {
                schema: RUNTIME_CONFIGURATION_SCHEMA.into(),
                content_sha256: Digest([1; 32]),
                language: "en".into(),
                settings: BTreeMap::new(),
            },
            active_runtime_file: RuntimeFile {
                id: "file-0".into(),
                origin: RuntimeFileOrigin::TitleFile0,
                backing: BackingAttachment::MemoryOnly,
                allowed_serialization_targets: Vec::new(),
                lifecycle: RuntimeFileLifecycle::Active,
            },
            inactive_runtime_files: Vec::new(),
            physical_slots: Vec::new(),
            physical_slot_observations: Vec::new(),
            execution_context: ExecutionContext::World,
            location: SceneLocation {
                stage: "STAGE_A".into(),
                room: 0,
                layer: 0,
                spawn: 0,
            },
            player: PlayerState {
                form: PlayerForm::Human,
                mount: None,
                position: [0.0; 3],
                attention_position: None,
                rotation: [0; 3],
                has_control: Some(true),
                action: "idle".into(),
            },
            components: Vec::new(),
            static_world_objects: Vec::new(),
            spatial_volumes: Vec::new(),
            spatial_connections: Vec::new(),
            spatial_planes: Vec::new(),
            persisted_object_controls: Vec::new(),
            live_world_objects: Vec::new(),
        },
        semantic_observations: Vec::new(),
    };
    let state = PlannerExecutionState::new(snapshot)
        .unwrap()
        .to_document()
        .unwrap();
    let facts = FactCatalog {
        schema: FACT_CATALOG_SCHEMA.into(),
        aliases: Vec::new(),
        derived_facts: Vec::new(),
    };
    let mechanics = MechanicsCatalog {
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
    };
    fs::write(&state_path, state.canonical_bytes().unwrap()).unwrap();
    fs::write(&facts_path, facts.canonical_bytes().unwrap()).unwrap();
    fs::write(&mechanics_path, mechanics.canonical_bytes().unwrap()).unwrap();

    project_authorization_graph(&[
        "--state".into(),
        state_path.to_string_lossy().into_owned(),
        "--facts".into(),
        facts_path.to_string_lossy().into_owned(),
        "--mechanics".into(),
        mechanics_path.to_string_lossy().into_owned(),
        "--output".into(),
        output_path.to_string_lossy().into_owned(),
        "--max-depth".into(),
        "4".into(),
        "--max-states".into(),
        "8".into(),
    ])
    .unwrap();
    let graph = AuthorizationGraph::decode_canonical(&fs::read(&output_path).unwrap()).unwrap();
    assert!(graph.traversal_complete);
    assert_eq!(graph.nodes.len(), 1);
    assert_eq!(graph.evaluated_states, 1);
    assert!(graph.edges.is_empty());
    assert!(graph.unknown_activation_candidates.is_empty());
    assert_eq!(graph.refinement_stack_sha256, None);
    fs::remove_dir_all(root).unwrap();
}
