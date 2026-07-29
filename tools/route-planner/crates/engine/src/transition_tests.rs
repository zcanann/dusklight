use super::*;
use crate::identity::{ContextSelector, ExactContext};
use crate::logic::{EvidenceRecord, TruthStatus};

fn scope() -> ContextScope {
    ContextScope {
        selectors: vec![ContextSelector::Exact {
            context: ExactContext {
                content_sha256: Digest([1; 32]),
                runtime_configuration_sha256: Digest([2; 32]),
            },
        }],
    }
}

fn evidence(truth: TruthStatus) -> RuleEvidence {
    RuleEvidence {
        truth,
        records: if truth == TruthStatus::Unknown {
            Vec::new()
        } else {
            vec![EvidenceRecord {
                id: "source.test".into(),
                kind: EvidenceKind::SourceAudited,
                source_sha256: Some(Digest([3; 32])),
                note: "Test source audit.".into(),
            }]
        },
    }
}

fn locked_door_catalog() -> MechanicsCatalog {
    MechanicsCatalog {
        schema: MECHANICS_CATALOG_SCHEMA.into(),
        transitions: vec![CandidateTransition {
            id: "transition.forest.door-1".into(),
            label: "Enter the next Forest Temple room".into(),
            scope: scope(),
            transition_kind: TransitionKind::Door,
            approach_id: "approach.front".into(),
            activation: ActivationContract {
                hard_guards: PredicateExpression::Fact {
                    fact_id: "dungeon.forest.small-key-positive".into(),
                },
                physical_obligation_ids: vec!["obligation.reach-door".into()],
                effects: vec![StateOperation::Write {
                    target: ComponentFieldTarget {
                        component_id: "forest-memory".into(),
                        field: "small-keys".into(),
                    },
                    value: StateValue::Unsigned(0),
                }],
                unknown_requirements: Vec::new(),
            },
            evidence: evidence(TruthStatus::Established),
        }],
        obligations: vec![FeasibilityObligation {
            id: "obligation.reach-door".into(),
            label: "Reach the front of the locked door".into(),
            scope: scope(),
            obligation_kind: ObligationKind::Geometry,
            stage: crate::transition::ObligationStage::Reach,
            detail: ObligationDetail::Geometry {
                approach_id: "approach.front".into(),
                source_region_id: "forest.room-0".into(),
                destination_region_id: "forest.door-1.front".into(),
            },
            evidence: evidence(TruthStatus::Unknown),
        }],
        writers: Vec::new(),
        gates: Vec::new(),
        readers: Vec::new(),
        reconstruction_rules: Vec::new(),
        obstructions: Vec::new(),
        resolvers: Vec::new(),
        techniques: Vec::new(),
        microtraces: Vec::new(),
        goals: Vec::new(),
    }
}

#[test]
fn encoded_destination_requires_both_hard_guard_and_physical_obligation() {
    let catalog = locked_door_catalog();
    catalog.validate().unwrap();
    let bytes = catalog.canonical_bytes().unwrap();
    assert_eq!(MechanicsCatalog::decode_canonical(&bytes).unwrap(), catalog);
    assert_ne!(catalog.digest().unwrap(), Digest::ZERO);
    let transition = &catalog.transitions[0];
    assert!(matches!(
        transition.activation.hard_guards,
        PredicateExpression::Fact { .. }
    ));
    assert_eq!(
        transition.activation.physical_obligation_ids,
        vec!["obligation.reach-door"]
    );
}

#[test]
fn missing_obligation_reference_fails_closed() {
    let mut catalog = locked_door_catalog();
    catalog.transitions[0].activation.physical_obligation_ids = vec!["obligation.missing".into()];
    assert_eq!(
        catalog.validate().unwrap_err().field(),
        "transitions.activation.physical_obligation_ids"
    );
}

#[test]
fn extracted_destination_without_an_activation_contract_fails_closed() {
    let mut catalog = locked_door_catalog();
    let transition = &mut catalog.transitions[0];
    transition.transition_kind = TransitionKind::EncodedMapExit;
    transition.activation.effects = vec![StateOperation::SetLocation {
        location: crate::state::SceneLocation {
            stage: "D_MN05B".into(),
            room: 1,
            layer: 0,
            spawn: 2,
        },
    }];
    transition.evidence.records[0].kind = EvidenceKind::Extracted;
    transition.activation.physical_obligation_ids.clear();
    assert_eq!(
        catalog.validate().unwrap_err().field(),
        "transitions.activation.extracted_destination"
    );

    catalog.transitions[0].activation.unknown_requirements = vec![UnknownRequirement {
        id: "unknown.destination-activation".into(),
        description: "The destination is decoded but its activation physics are unknown.".into(),
        evidence: evidence(TruthStatus::Established),
    }];
    catalog.validate().unwrap();
}

#[test]
fn loaded_image_and_runtime_carry_manifests_must_be_disjoint() {
    let operation = StateOperation::LoadRuntimeFromSlot {
        source_runtime_file_id: "file-0".into(),
        source_slot: PhysicalSlotId(1),
        source_persistent_file_id: "persistent-slot-1".into(),
        destination_runtime_file_id: "loaded-slot-1".into(),
        destination_allowed_serialization_targets: vec![PhysicalSlotId(1)],
        runtime_component_ids: vec!["save.main".into()],
        stage_bank_stages: Vec::new(),
        carried_runtime_component_ids: vec!["save.main".into()],
    };
    assert_eq!(
        operation.validate().unwrap_err().field(),
        "operation.carried_runtime_component_ids"
    );
}

#[test]
fn process_buffer_operations_cannot_replace_runtime_or_physical_stores() {
    let replace = StateOperation::ReplaceCustomStore {
        owner: SerializationOwner::PhysicalSlot {
            slot: PhysicalSlotId(1),
        },
        components: Vec::new(),
    };
    assert_eq!(
        replace.validate().unwrap_err().field(),
        "operation.replace_custom_store.owner"
    );

    let restore = StateOperation::RestorePayloadsFromCustomStore {
        owner: SerializationOwner::RuntimeFile {
            runtime_file_id: "file-0".into(),
        },
        component_ids: vec!["save.main".into()],
    };
    assert_eq!(
        restore.validate().unwrap_err().field(),
        "operation.restore_payloads_from_custom_store.owner"
    );
}

#[test]
fn obstruction_resolution_is_directional_and_does_not_delete_world_fact() {
    let mut catalog = locked_door_catalog();
    catalog.obstructions.push(Obstruction {
        id: "obstruction.wall".into(),
        label: "Wall blocks the front approach".into(),
        scope: scope(),
        blocked_action_id: "transition.forest.door-1".into(),
        approach_id: "approach.front".into(),
        active_when: PredicateExpression::True,
        obligation_ids: vec!["obligation.reach-door".into()],
        evidence: evidence(TruthStatus::Established),
    });
    catalog.resolvers.push(ObstructionResolver {
        id: "resolver.wall-clip".into(),
        label: "Clip around this wall".into(),
        scope: scope(),
        obstruction_id: "obstruction.wall".into(),
        resolution_kind: ResolutionKind::Bypass,
        applicable_when: PredicateExpression::True,
        operations: Vec::new(),
        evidence: evidence(TruthStatus::Hypothetical),
    });
    catalog.validate().unwrap();
    assert_eq!(catalog.obstructions.len(), 1);
    assert_eq!(catalog.resolvers[0].obstruction_id, "obstruction.wall");
}

#[test]
fn dialogue_interruption_names_window_flow_and_cleanup_operations() {
    let trace = WitnessedMicrotrace {
        id: "microtrace.auru-sidehop".into(),
        scope: scope(),
        precondition: PredicateExpression::True,
        operations: vec![
            StateOperation::AdvanceFlow {
                flow_component_id: "flow.auru".into(),
                node_id: "node.item".into(),
            },
            StateOperation::CancelCleanup {
                cleanup_id: "cleanup.message-progress".into(),
            },
            StateOperation::Interrupt {
                action_id: "dialogue.auru".into(),
                window: TemporalWindow {
                    earliest_frame: 0,
                    latest_frame: 0,
                    required_input: Some("sidehop".into()),
                },
            },
        ],
        postcondition: PredicateExpression::Fact {
            fact_id: "message.temporary-item-state-held".into(),
        },
        timing: TemporalWindow {
            earliest_frame: 0,
            latest_frame: 0,
            required_input: Some("sidehop".into()),
        },
        evidence: evidence(TruthStatus::Established),
    };
    validate_microtrace(&trace).unwrap();
}

#[test]
fn actor_reconstruction_consumes_persisted_state_explicitly() {
    let mut catalog = locked_door_catalog();
    catalog.reconstruction_rules.push(ActorReconstructionRule {
        id: "reconstruct.forest-door".into(),
        label: "Reconstruct the Forest Temple door actor".into(),
        scope: scope(),
        actor_type: "obj_door".into(),
        instantiate_when: PredicateExpression::Fact {
            fact_id: "world.forest-door.placed-on-layer".into(),
        },
        initialization_operations: vec![StateOperation::Write {
            target: ComponentFieldTarget {
                component_id: "actor.forest-door/live".into(),
                field: "opened".into(),
            },
            value: StateValue::Boolean(false),
        }],
        evidence: evidence(TruthStatus::Established),
    });
    catalog.validate().unwrap();
    assert_eq!(catalog.reconstruction_rules.len(), 1);
}

#[test]
fn projection_requires_an_explicit_component_set() {
    let operation = StateOperation::Project {
        source_runtime_file_id: "file-0".into(),
        destination_runtime_file_id: "slot-1-runtime".into(),
        component_ids: Vec::new(),
    };
    assert_eq!(
        operation.validate().unwrap_err().field(),
        "operation.component_ids"
    );
}

#[test]
fn save_projection_cannot_write_outside_its_persistent_manifest() {
    let operation = StateOperation::SaveActiveRuntimeToSlot {
        destination_slot: PhysicalSlotId(1),
        destination_id_suffix: "save-slot-1".into(),
        runtime_component_ids: vec!["save.main".into()],
        projection_operations: vec![SaveProjectionOperation::Write {
            target: ComponentFieldTarget {
                component_id: "save-menu-control".into(),
                field: "phase".into(),
            },
            value: StateValue::Text("done".into()),
        }],
    };
    assert_eq!(
        operation.validate().unwrap_err().field(),
        "operation.save_active_runtime_to_slot.projection_operations"
    );
}

#[test]
fn bound_raw_adjustment_requires_an_explicit_binding() {
    let operation = StateOperation::AdjustBoundRawUnsigned {
        component_kind: ComponentKind::DungeonMemory,
        binding: ComponentBindingReference::Exact {
            binding: ComponentBinding::Unbound,
        },
        byte_offset: 0,
        byte_width: 1,
        delta: 1,
    };
    assert_eq!(
        operation.validate().unwrap_err().field(),
        "operation.adjust_bound_raw_unsigned.binding"
    );

    let write = StateOperation::WriteBoundRaw {
        component_kind: ComponentKind::StageMemory,
        binding: ComponentBindingReference::Exact {
            binding: ComponentBinding::Unbound,
        },
        byte_offset: 0,
        mask: vec![1],
        value: vec![1],
    };
    assert_eq!(
        write.validate().unwrap_err().field(),
        "operation.bound_raw.binding"
    );
}

#[test]
fn unsigned_minimum_clamp_requires_a_nonzero_floor() {
    let operation = StateOperation::ClampUnsignedMinimum {
        target: ComponentFieldTarget {
            component_id: "save.main".into(),
            field: "life".into(),
        },
        minimum: 0,
    };
    assert_eq!(
        operation.validate().unwrap_err().field(),
        "operation.clamp_unsigned_minimum.minimum"
    );
}

#[test]
fn unsigned_debit_requires_a_nonzero_amount() {
    let operation = StateOperation::DebitUnsigned {
        target: ComponentFieldTarget {
            component_id: "save.main".into(),
            field: "rupees".into(),
        },
        amount: 0,
    };
    assert_eq!(
        operation.validate().unwrap_err().field(),
        "operation.debit_unsigned.amount"
    );
}

#[test]
fn item_slot_normalization_rejects_ambiguous_layout_contracts() {
    let operation = StateOperation::NormalizeItemSlotsAndLineup {
        component_id: "save.main".into(),
        inventory_field: "inventory".into(),
        lineup_field: "item_lineup".into(),
        primary_slot: 9,
        secondary_slot: 10,
        single_item: 0x44,
        combined_item: 0x47,
        empty_item: 0xff,
        lineup_order: vec![10, 9, 10],
    };
    assert_eq!(
        operation.validate().unwrap_err().field(),
        "operation.normalize_item_slots_and_lineup.lineup_order"
    );

    let same_field = StateOperation::NormalizeItemSlotsAndLineup {
        component_id: "save.main".into(),
        inventory_field: "inventory".into(),
        lineup_field: "inventory".into(),
        primary_slot: 9,
        secondary_slot: 10,
        single_item: 0x44,
        combined_item: 0x47,
        empty_item: 0xff,
        lineup_order: vec![10, 9],
    };
    assert_eq!(
        same_field.validate().unwrap_err().field(),
        "operation.normalize_item_slots_and_lineup.lineup_field"
    );
}
