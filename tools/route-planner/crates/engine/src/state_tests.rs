use super::*;
use crate::identity::RUNTIME_CONFIGURATION_SCHEMA;

fn provenance(source_id: &str) -> ComponentProvenance {
    ComponentProvenance {
        source_kind: ProvenanceSourceKind::Initialized,
        source_id: source_id.into(),
        source_sha256: Some(Digest([9; 32])),
        transition_id: None,
    }
}

fn component(id: &str, kind: ComponentKind) -> StateComponent {
    StateComponent {
        id: id.into(),
        component_kind: kind,
        payload: ComponentPayload::Structured {
            fields: BTreeMap::new(),
        },
        binding: ComponentBinding::RuntimeFile {
            runtime_file_id: "file-0".into(),
        },
        lifetime: SemanticLifetime::RuntimeFile,
        serialization_owner: SerializationOwner::RuntimeFile {
            runtime_file_id: "file-0".into(),
        },
        provenance: vec![provenance("title-init")],
    }
}

fn file_zero_environment() -> ExecutionEnvironment {
    ExecutionEnvironment {
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
            allowed_serialization_targets: vec![
                PhysicalSlotId(1),
                PhysicalSlotId(2),
                PhysicalSlotId(3),
            ],
            lifecycle: RuntimeFileLifecycle::Active,
        },
        inactive_runtime_files: Vec::new(),
        physical_slots: Vec::new(),
        physical_slot_observations: Vec::new(),
        execution_context: ExecutionContext::World,
        location: SceneLocation {
            stage: "F_SP103".into(),
            room: 0,
            layer: 0,
            spawn: 0,
        },
        player: PlayerState {
            form: PlayerForm::Human,
            mount: None,
            position: [0.0, 1.0, 2.0],
            attention_position: None,
            rotation: [0, 0, 0],
            has_control: Some(true),
            action: "idle".into(),
        },
        components: vec![
            component("inventory", ComponentKind::Inventory),
            component("progress", ComponentKind::PersistentSave),
        ],
        static_world_objects: Vec::new(),
        spatial_volumes: Vec::new(),
        spatial_connections: Vec::new(),
        spatial_planes: Vec::new(),
        persisted_object_controls: Vec::new(),
        live_world_objects: Vec::new(),
    }
}

#[test]
fn file_zero_is_memory_backed_but_can_hold_persistent_domain_components() {
    let environment = file_zero_environment();
    environment.validate().unwrap();
    assert_eq!(
        environment.active_runtime_file.backing,
        BackingAttachment::MemoryOnly
    );
    assert!(
        environment
            .components
            .iter()
            .any(|component| component.component_kind == ComponentKind::PersistentSave)
    );
    assert_ne!(environment.digest().unwrap(), Digest::ZERO);
}

#[test]
fn slot_zero_and_duplicate_component_ids_fail_closed() {
    let mut environment = file_zero_environment();
    environment
        .active_runtime_file
        .allowed_serialization_targets = vec![PhysicalSlotId(0)];
    assert_eq!(
        environment.validate().unwrap_err().field(),
        "allowed_serialization_targets"
    );

    let mut environment = file_zero_environment();
    environment
        .components
        .push(component("progress", ComponentKind::Inventory));
    assert_eq!(environment.validate().unwrap_err().field(), "components");
}

#[test]
fn raw_unknown_mask_and_component_provenance_are_explicit() {
    let mut component = component("stage-memory", ComponentKind::StageMemory);
    component.payload = ComponentPayload::Raw {
        bytes: vec![0xaa, 0x55],
        known_mask: vec![0xff, 0x00],
    };
    component.provenance.push(ComponentProvenance {
        source_kind: ProvenanceSourceKind::Transition,
        source_id: "bite-splice".into(),
        source_sha256: None,
        transition_id: Some("technique.bite".into()),
    });
    component.validate().unwrap();

    if let ComponentPayload::Raw { known_mask, .. } = component.payload {
        assert_eq!(known_mask, vec![0xff, 0x00]);
    } else {
        panic!("expected raw component payload");
    }
}

#[test]
fn actor_placement_persistence_and_live_instance_are_independent() {
    let mut environment = file_zero_environment();
    environment.static_world_objects.push(StaticWorldObject {
        id: "gate.ordon".into(),
        actor_type: "obj_gate".into(),
        placement_sha256: Digest([3; 32]),
        binding: ComponentBinding::Room {
            stage: "F_SP103".into(),
            room: 0,
        },
        parameters: BTreeMap::new(),
    });
    environment
        .persisted_object_controls
        .push(PersistedObjectControl {
            object_id: "gate.ordon".into(),
            fields: BTreeMap::from([("open".into(), StateValue::Boolean(true))]),
        });
    environment.live_world_objects.push(LiveWorldObject {
        instance_id: "gate.ordon/live/1".into(),
        static_object_id: Some("gate.ordon".into()),
        actor_type: "obj_gate".into(),
        lifecycle: ActorLifecycle::Unloaded,
        fields: BTreeMap::new(),
    });
    environment.validate().unwrap();
    assert_eq!(environment.static_world_objects.len(), 1);
    assert_eq!(environment.persisted_object_controls.len(), 1);
    assert_eq!(environment.live_world_objects.len(), 1);
}

#[test]
fn spatial_volumes_require_canonical_ordered_evidenced_bounds() {
    let mut environment = file_zero_environment();
    environment.spatial_volumes.push(SpatialVolume {
        object_id: "actor.auru".into(),
        volume_id: "talk".into(),
        shape: SpatialVolumeShape::AxisAlignedBox {
            minimum: [-1.0, 0.0, -2.0],
            maximum: [1.0, 2.0, 2.0],
        },
        source_sha256: Digest([4; 32]),
    });
    environment.validate().unwrap();

    let mut invalid_bounds = environment.clone();
    invalid_bounds.spatial_volumes[0].shape = SpatialVolumeShape::AxisAlignedBox {
        minimum: [2.0, 0.0, 0.0],
        maximum: [1.0, 1.0, 1.0],
    };
    assert_eq!(
        invalid_bounds.validate().unwrap_err().field(),
        "spatial_volumes.shape"
    );

    let mut invalid_digest = environment;
    invalid_digest.spatial_volumes[0].source_sha256 = Digest::ZERO;
    assert_eq!(
        invalid_digest.validate().unwrap_err().field(),
        "spatial_volumes.source_sha256"
    );

    let mut invalid_sphere = file_zero_environment();
    invalid_sphere.spatial_volumes.push(SpatialVolume {
        object_id: "actor.auru".into(),
        volume_id: "talk".into(),
        shape: SpatialVolumeShape::Sphere {
            center: [0.0; 3],
            radius: 0.0,
        },
        source_sha256: Digest([4; 32]),
    });
    assert_eq!(
        invalid_sphere.validate().unwrap_err().field(),
        "spatial_volumes.shape"
    );
}

#[test]
fn spatial_connections_and_planes_are_directional_and_evidenced() {
    let mut environment = file_zero_environment();
    environment.spatial_connections.push(SpatialConnection {
        approach_id: "approach.front".into(),
        source_region_id: "region.a".into(),
        destination_region_id: "region.b".into(),
        status: SpatialConnectionStatus::Blocked,
        source_sha256: Digest([5; 32]),
    });
    environment.spatial_planes.push(SpatialPlane {
        plane_id: "void.room-0".into(),
        normal: [0.0, 1.0, 0.0],
        offset: 0.0,
        source_sha256: Digest([6; 32]),
    });
    environment.validate().unwrap();

    let mut reverse = environment.clone();
    reverse.spatial_connections.push(SpatialConnection {
        approach_id: "approach.front".into(),
        source_region_id: "region.b".into(),
        destination_region_id: "region.a".into(),
        status: SpatialConnectionStatus::Traversable,
        source_sha256: Digest([7; 32]),
    });
    reverse.validate().unwrap();

    let mut invalid_plane = environment;
    invalid_plane.spatial_planes[0].normal = [0.0; 3];
    assert_eq!(
        invalid_plane.validate().unwrap_err().field(),
        "spatial_planes"
    );
}

#[test]
fn boundary_policy_never_implicitly_preserves_unmentioned_components() {
    let policy = BoundaryPolicy {
        schema: BOUNDARY_POLICY_SCHEMA.into(),
        id: "boundary.stage-transition".into(),
        boundary: BoundaryKind::StageTransition,
        default_disposition: BoundaryDisposition::Unknown,
        component_rules: vec![ComponentBoundaryRule {
            selector: ComponentSelector::Kind {
                component_kind: ComponentKind::PersistentSave,
            },
            disposition: BoundaryDisposition::Preserve,
        }],
    };
    policy.validate().unwrap();
    assert_eq!(policy.default_disposition, BoundaryDisposition::Unknown);

    let mut duplicate = policy;
    duplicate
        .component_rules
        .push(duplicate.component_rules[0].clone());
    assert_eq!(duplicate.validate().unwrap_err().field(), "component_rules");
}

#[test]
fn binding_references_resolve_from_the_live_environment() {
    let environment = file_zero_environment();
    assert_eq!(
        ComponentBindingReference::ActiveRuntimeFile.resolve(&environment),
        Some(ComponentBinding::RuntimeFile {
            runtime_file_id: "file-0".into(),
        })
    );
    assert_eq!(
        ComponentBindingReference::CurrentStage.resolve(&environment),
        Some(ComponentBinding::Stage {
            stage: "F_SP103".into(),
        })
    );
    assert_eq!(
        ComponentBindingReference::CurrentRoom.resolve(&environment),
        Some(ComponentBinding::Room {
            stage: "F_SP103".into(),
            room: 0,
        })
    );
    let exact = ComponentBindingReference::Exact {
        binding: ComponentBinding::Dungeon {
            dungeon: "forest-temple".into(),
        },
    };
    assert_eq!(
        exact.resolve(&environment),
        Some(ComponentBinding::Dungeon {
            dungeon: "forest-temple".into(),
        })
    );

    let mut projected_environment = environment;
    projected_environment.components.push(StateComponent {
        id: "message-session".into(),
        component_kind: ComponentKind::MessageFlow,
        payload: ComponentPayload::Structured {
            fields: BTreeMap::from([
                ("speaker_stage".into(), StateValue::Text("D_MN01".into())),
                ("speaker_zone".into(), StateValue::Signed(7)),
            ]),
        },
        binding: ComponentBinding::Global,
        lifetime: SemanticLifetime::Action,
        serialization_owner: SerializationOwner::None,
        provenance: vec![provenance("fixture")],
    });
    let projected = ComponentBindingReference::Projected {
        component_id: "message-session".into(),
        projection: Box::new(ComponentBindingProjection::Zone {
            stage_field: "speaker_stage".into(),
            zone_field: "speaker_zone".into(),
        }),
    };
    assert_eq!(
        projected.resolve(&projected_environment),
        Some(ComponentBinding::Zone {
            stage: "D_MN01".into(),
            zone: 7,
        })
    );
    projected_environment.components.last_mut().unwrap().payload = ComponentPayload::Unknown {
        expected_bytes: None,
    };
    assert_eq!(projected.resolve(&projected_environment), None);
}
