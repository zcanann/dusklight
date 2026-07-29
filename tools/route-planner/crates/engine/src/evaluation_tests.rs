
use super::*;
use crate::artifact::Digest;
use crate::identity::{
    EQUIVALENCE_SET_SCHEMA, EquivalenceEvidence, EquivalenceEvidenceKind,
    RUNTIME_CONFIGURATION_SCHEMA, RuntimeConfiguration,
};
use crate::logic::{
    DerivedFact, EvidenceKind, EvidenceRecord, FACT_CATALOG_SCHEMA, FriendlyAlias, RuleEvidence,
};
use crate::snapshot::{STATE_SNAPSHOT_SCHEMA, StateSnapshot};
use crate::state::{
    ActorLifecycle, BackingAttachment, ComponentBinding, ComponentBindingProjection,
    ComponentBindingReference, ComponentKind, ComponentPayload, ComponentProvenance,
    EXECUTION_ENVIRONMENT_SCHEMA, ExecutionContext, ExecutionEnvironment, LiveWorldObject,
    PlaneRelation, PlayerForm, PlayerState, ProvenanceSourceKind, RuntimeFile,
    RuntimeFileLifecycle, RuntimeFileOrigin, SceneLocation, SemanticLifetime, SerializationOwner,
    SpatialConnection, SpatialConnectionStatus, SpatialLocalAxis, SpatialPlane, SpatialVolume,
    SpatialVolumeShape, StateComponent,
};
use crate::transition::{
    ActivationContract, InteractionBranch, InteractionPosition, InteractionVolumeTest,
    ObligationKind, StateOperation, TemporalWindow, TransitionKind, UnknownRequirement,
    VolumeReference,
};

fn evidence(truth: TruthStatus) -> RuleEvidence {
    RuleEvidence {
        truth,
        records: if matches!(truth, TruthStatus::Established | TruthStatus::Contested) {
            vec![EvidenceRecord {
                id: "source.evaluator-test".into(),
                kind: EvidenceKind::SourceAudited,
                source_sha256: Some(Digest([9; 32])),
                note: "Evaluator test evidence.".into(),
            }]
        } else {
            Vec::new()
        },
    }
}

fn component(known_mask: u8) -> StateComponent {
    StateComponent {
        id: "save-flags".into(),
        component_kind: ComponentKind::PersistentSave,
        payload: ComponentPayload::Raw {
            bytes: vec![0x20],
            known_mask: vec![known_mask],
        },
        binding: ComponentBinding::Global,
        lifetime: SemanticLifetime::RuntimeFile,
        serialization_owner: SerializationOwner::RuntimeFile {
            runtime_file_id: "file-0".into(),
        },
        provenance: vec![ComponentProvenance {
            source_kind: ProvenanceSourceKind::TraceObservation,
            source_id: "trace.test".into(),
            source_sha256: Some(Digest([8; 32])),
            transition_id: None,
        }],
    }
}

fn snapshot(known_mask: u8) -> StateSnapshot {
    StateSnapshot {
        schema: STATE_SNAPSHOT_SCHEMA.into(),
        id: "snapshot.evaluator".into(),
        sequence: 1,
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
            execution_context: crate::state::ExecutionContext::World,
            location: SceneLocation {
                stage: "F_SP103".into(),
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
            components: vec![component(known_mask)],
            static_world_objects: Vec::new(),
            spatial_volumes: Vec::new(),
            spatial_connections: Vec::new(),
            spatial_planes: Vec::new(),
            persisted_object_controls: Vec::new(),
            live_world_objects: Vec::new(),
        },
        semantic_observations: Vec::new(),
    }
}

fn scope(snapshot: &StateSnapshot) -> ContextScope {
    ContextScope {
        selectors: vec![ContextSelector::Exact {
            context: snapshot
                .environment
                .runtime_configuration
                .exact_context()
                .unwrap(),
        }],
    }
}

fn facts(snapshot: &StateSnapshot, alias_truth: TruthStatus) -> FactCatalog {
    let scope = scope(snapshot);
    FactCatalog {
        schema: FACT_CATALOG_SCHEMA.into(),
        aliases: vec![FriendlyAlias {
            id: "story.faron.twilight".into(),
            label: "Faron is in twilight".into(),
            scope: scope.clone(),
            raw: RawFactBinding {
                component_kind: ComponentKind::PersistentSave,
                binding: ComponentBindingReference::Exact {
                    binding: ComponentBinding::Global,
                },
                byte_offset: 0,
                mask: vec![0x20],
                expected: vec![0x20],
            },
            evidence: evidence(alias_truth),
        }],
        derived_facts: vec![DerivedFact {
            id: "world.faron.twilight-access".into(),
            label: "Faron twilight access state".into(),
            scope,
            rule: PredicateExpression::Fact {
                fact_id: "story.faron.twilight".into(),
            },
            evidence: evidence(TruthStatus::Established),
        }],
    }
}

fn evaluator<'a>(
    snapshot: &'a StateSnapshot,
    facts: &'a FactCatalog,
    policy: EvidencePolicy,
) -> PredicateEvaluator<'a> {
    PredicateEvaluator::new(snapshot, facts, &[], &BTreeMap::new(), policy).unwrap()
}

fn fact(id: &str) -> PredicateExpression {
    PredicateExpression::Fact { fact_id: id.into() }
}

fn transition(snapshot: &StateSnapshot, guard: PredicateExpression) -> CandidateTransition {
    CandidateTransition {
        id: "transition.test".into(),
        label: "Test transition".into(),
        scope: scope(snapshot),
        transition_kind: TransitionKind::Door,
        approach_id: "approach.front".into(),
        activation: ActivationContract {
            hard_guards: guard,
            physical_obligation_ids: vec!["obligation.wall".into()],
            effects: Vec::new(),
            unknown_requirements: Vec::new(),
        },
        evidence: evidence(TruthStatus::Established),
    }
}

#[test]
fn aliases_and_derived_facts_resolve_from_known_raw_state() {
    let snapshot = snapshot(0xff);
    let facts = facts(&snapshot, TruthStatus::Established);
    let evaluator = evaluator(&snapshot, &facts, EvidencePolicy::ESTABLISHED_ONLY);
    assert_eq!(
        evaluator.evaluate(&fact("story.faron.twilight")),
        EvaluatedTruth::True
    );
    assert_eq!(
        evaluator.evaluate(&fact("world.faron.twilight-access")),
        EvaluatedTruth::True
    );
}

#[test]
fn bound_component_fields_follow_the_backing_binding_and_fail_on_ambiguity() {
    let mut snapshot = snapshot(0xff);
    snapshot.environment.components.push(StateComponent {
        id: "dungeon.active".into(),
        component_kind: ComponentKind::DungeonMemory,
        payload: ComponentPayload::Structured {
            fields: BTreeMap::from([("small_keys".into(), StateValue::Unsigned(1))]),
        },
        binding: ComponentBinding::Dungeon {
            dungeon: "forest-temple".into(),
        },
        lifetime: SemanticLifetime::StageLoad,
        serialization_owner: SerializationOwner::StageBank {
            runtime_file_id: "file-0".into(),
            stage: "D_MN05".into(),
        },
        provenance: vec![ComponentProvenance {
            source_kind: ProvenanceSourceKind::TraceObservation,
            source_id: "trace.forest-keys".into(),
            source_sha256: Some(Digest([4; 32])),
            transition_id: None,
        }],
    });
    snapshot
        .environment
        .components
        .sort_by(|left, right| left.id.cmp(&right.id));
    let facts = facts(&snapshot, TruthStatus::Established);
    let reference = |dungeon: &str| ValueReference::BoundComponentField {
        component_kind: ComponentKind::DungeonMemory,
        binding: ComponentBindingReference::Exact {
            binding: ComponentBinding::Dungeon {
                dungeon: dungeon.into(),
            },
        },
        field: "small_keys".into(),
    };
    let forest_evaluator = evaluator(&snapshot, &facts, EvidencePolicy::ESTABLISHED_ONLY);
    assert_eq!(
        forest_evaluator.resolve_value(&reference("forest-temple")),
        Some(StateValue::Unsigned(1))
    );
    assert_eq!(
        forest_evaluator.resolve_value(&reference("goron-mines")),
        None
    );

    snapshot
        .environment
        .components
        .iter_mut()
        .find(|component| component.id == "dungeon.active")
        .unwrap()
        .binding = ComponentBinding::Dungeon {
        dungeon: "goron-mines".into(),
    };
    let goron_evaluator = evaluator(&snapshot, &facts, EvidencePolicy::ESTABLISHED_ONLY);
    assert_eq!(
        goron_evaluator.resolve_value(&reference("goron-mines")),
        Some(StateValue::Unsigned(1))
    );
    assert_eq!(
        goron_evaluator.resolve_value(&reference("forest-temple")),
        None
    );

    let mut duplicate = snapshot
        .environment
        .components
        .iter()
        .find(|component| component.id == "dungeon.active")
        .unwrap()
        .clone();
    duplicate.id = "dungeon.ambiguous".into();
    snapshot.environment.components.push(duplicate);
    snapshot
        .environment
        .components
        .sort_by(|left, right| left.id.cmp(&right.id));
    assert_eq!(
        evaluator(&snapshot, &facts, EvidencePolicy::ESTABLISHED_ONLY)
            .resolve_value(&reference("goron-mines")),
        None
    );
}

#[test]
fn bound_raw_values_follow_stage_memory_and_fail_on_unknown_or_ambiguous_bytes() {
    let mut snapshot = snapshot(0xff);
    let mut bytes = vec![0_u8; 0x20];
    bytes[0x1c] = 2;
    bytes[0x1d] = 0b0100_0111;
    snapshot.environment.components.push(StateComponent {
        id: "stage-memory.active".into(),
        component_kind: ComponentKind::DungeonMemory,
        payload: ComponentPayload::Raw {
            bytes,
            known_mask: vec![0xff; 0x20],
        },
        binding: ComponentBinding::Stage {
            stage: "D_MN05".into(),
        },
        lifetime: SemanticLifetime::StageLoad,
        serialization_owner: SerializationOwner::StageBank {
            runtime_file_id: "file-0".into(),
            stage: "D_MN05".into(),
        },
        provenance: vec![ComponentProvenance {
            source_kind: ProvenanceSourceKind::TraceObservation,
            source_id: "trace.forest-stage-memory".into(),
            source_sha256: Some(Digest([5; 32])),
            transition_id: None,
        }],
    });
    snapshot
        .environment
        .components
        .sort_by(|left, right| left.id.cmp(&right.id));
    let facts = facts(&snapshot, TruthStatus::Established);
    let stage_value = |stage: &str, byte_offset: u32, mask: u64| ValueReference::BoundRawBits {
        component_kind: ComponentKind::DungeonMemory,
        binding: ComponentBindingReference::Exact {
            binding: ComponentBinding::Stage {
                stage: stage.into(),
            },
        },
        byte_offset,
        byte_width: 1,
        mask,
    };
    let initial_evaluator = evaluator(&snapshot, &facts, EvidencePolicy::ESTABLISHED_ONLY);
    assert_eq!(
        initial_evaluator.resolve_value(&stage_value("D_MN05", 0x1c, 0xff)),
        Some(StateValue::Unsigned(2))
    );
    assert_eq!(
        initial_evaluator.resolve_value(&stage_value("D_MN05", 0x1d, 1 << 2)),
        Some(StateValue::Unsigned(1 << 2))
    );
    assert_eq!(
        initial_evaluator.resolve_value(&stage_value("D_MN04", 0x1c, 0xff)),
        None
    );

    let stage_memory = snapshot
        .environment
        .components
        .iter_mut()
        .find(|component| component.id == "stage-memory.active")
        .unwrap();
    let ComponentPayload::Raw { known_mask, .. } = &mut stage_memory.payload else {
        unreachable!()
    };
    known_mask[0x1d] = 0xfb;
    assert_eq!(
        evaluator(&snapshot, &facts, EvidencePolicy::ESTABLISHED_ONLY).resolve_value(&stage_value(
            "D_MN05",
            0x1d,
            1 << 2
        )),
        None
    );
    assert_eq!(
        evaluator(&snapshot, &facts, EvidencePolicy::ESTABLISHED_ONLY)
            .resolve_value(&stage_value("D_MN05", 0x1c, 0xff)),
        Some(StateValue::Unsigned(2))
    );

    let mut duplicate = snapshot
        .environment
        .components
        .iter()
        .find(|component| component.id == "stage-memory.active")
        .unwrap()
        .clone();
    duplicate.id = "stage-memory.ambiguous".into();
    snapshot.environment.components.push(duplicate);
    snapshot
        .environment
        .components
        .sort_by(|left, right| left.id.cmp(&right.id));
    assert_eq!(
        evaluator(&snapshot, &facts, EvidencePolicy::ESTABLISHED_ONLY)
            .resolve_value(&stage_value("D_MN05", 0x1c, 0xff)),
        None
    );
}

#[test]
fn bound_raw_bits_follow_rebinding_and_fail_on_ambiguity() {
    let mut snapshot = snapshot(0xff);
    let raw_bank = |id: &str, dungeon: &str| StateComponent {
        id: id.into(),
        component_kind: ComponentKind::DungeonMemory,
        payload: ComponentPayload::Raw {
            bytes: vec![0b0000_0011],
            known_mask: vec![0xff],
        },
        binding: ComponentBinding::Dungeon {
            dungeon: dungeon.into(),
        },
        lifetime: SemanticLifetime::StageLoad,
        serialization_owner: SerializationOwner::StageBank {
            runtime_file_id: "file-0".into(),
            stage: "D_MN05".into(),
        },
        provenance: vec![ComponentProvenance {
            source_kind: ProvenanceSourceKind::TraceObservation,
            source_id: "trace.raw-dungeon-bank".into(),
            source_sha256: Some(Digest([5; 32])),
            transition_id: None,
        }],
    };
    snapshot
        .environment
        .components
        .push(raw_bank("dungeon.raw", "forest-temple"));
    snapshot
        .environment
        .components
        .sort_by(|left, right| left.id.cmp(&right.id));
    let facts = facts(&snapshot, TruthStatus::Established);
    let reference = |dungeon: &str| ValueReference::BoundRawBits {
        component_kind: ComponentKind::DungeonMemory,
        binding: ComponentBindingReference::Exact {
            binding: ComponentBinding::Dungeon {
                dungeon: dungeon.into(),
            },
        },
        byte_offset: 0,
        byte_width: 1,
        mask: 0x0f,
    };
    assert_eq!(
        evaluator(&snapshot, &facts, EvidencePolicy::ESTABLISHED_ONLY)
            .resolve_value(&reference("forest-temple")),
        Some(StateValue::Unsigned(3))
    );
    assert_eq!(
        evaluator(&snapshot, &facts, EvidencePolicy::ESTABLISHED_ONLY)
            .resolve_value(&reference("goron-mines")),
        None
    );

    snapshot
        .environment
        .components
        .iter_mut()
        .find(|component| component.id == "dungeon.raw")
        .unwrap()
        .binding = ComponentBinding::Dungeon {
        dungeon: "goron-mines".into(),
    };
    assert_eq!(
        evaluator(&snapshot, &facts, EvidencePolicy::ESTABLISHED_ONLY)
            .resolve_value(&reference("goron-mines")),
        Some(StateValue::Unsigned(3))
    );

    snapshot
        .environment
        .components
        .push(raw_bank("dungeon.raw.duplicate", "goron-mines"));
    snapshot
        .environment
        .components
        .sort_by(|left, right| left.id.cmp(&right.id));
    assert_eq!(
        evaluator(&snapshot, &facts, EvidencePolicy::ESTABLISHED_ONLY)
            .resolve_value(&reference("goron-mines")),
        None
    );
}

#[test]
fn bound_raw_bits_can_follow_the_current_stage() {
    let mut snapshot = snapshot(0xff);
    snapshot.environment.location.stage = "D_MN05".into();
    snapshot.environment.components.push(StateComponent {
        id: "stage.raw".into(),
        component_kind: ComponentKind::DungeonMemory,
        payload: ComponentPayload::Raw {
            bytes: vec![0b0000_0110],
            known_mask: vec![0xff],
        },
        binding: ComponentBinding::Stage {
            stage: "D_MN05".into(),
        },
        lifetime: SemanticLifetime::StageLoad,
        serialization_owner: SerializationOwner::StageBank {
            runtime_file_id: "file-0".into(),
            stage: "D_MN05".into(),
        },
        provenance: vec![ComponentProvenance {
            source_kind: ProvenanceSourceKind::TraceObservation,
            source_id: "trace.current-stage-bank".into(),
            source_sha256: Some(Digest([5; 32])),
            transition_id: None,
        }],
    });
    snapshot
        .environment
        .components
        .sort_by(|left, right| left.id.cmp(&right.id));
    let facts = facts(&snapshot, TruthStatus::Established);
    let reference = ValueReference::BoundRawBits {
        component_kind: ComponentKind::DungeonMemory,
        binding: ComponentBindingReference::CurrentStage,
        byte_offset: 0,
        byte_width: 1,
        mask: 0x0f,
    };

    assert_eq!(
        evaluator(&snapshot, &facts, EvidencePolicy::ESTABLISHED_ONLY).resolve_value(&reference),
        Some(StateValue::Unsigned(6))
    );

    snapshot.environment.location.stage = "D_MN04".into();
    assert_eq!(
        evaluator(&snapshot, &facts, EvidencePolicy::ESTABLISHED_ONLY).resolve_value(&reference),
        None
    );
}

#[test]
fn bound_raw_bits_can_follow_a_binding_projected_from_live_flow_state() {
    let mut snapshot = snapshot(0xff);
    snapshot.environment.components.extend([
        StateComponent {
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
            provenance: vec![ComponentProvenance {
                source_kind: ProvenanceSourceKind::Initialized,
                source_id: "fixture.message-session".into(),
                source_sha256: None,
                transition_id: None,
            }],
        },
        StateComponent {
            id: "zone.raw".into(),
            component_kind: ComponentKind::ZoneMemory,
            payload: ComponentPayload::Raw {
                bytes: vec![0b0000_0110],
                known_mask: vec![0xff],
            },
            binding: ComponentBinding::Zone {
                stage: "D_MN01".into(),
                zone: 7,
            },
            lifetime: SemanticLifetime::RoomLoad,
            serialization_owner: SerializationOwner::None,
            provenance: vec![ComponentProvenance {
                source_kind: ProvenanceSourceKind::Initialized,
                source_id: "fixture.zone-memory".into(),
                source_sha256: None,
                transition_id: None,
            }],
        },
    ]);
    snapshot
        .environment
        .components
        .sort_by(|left, right| left.id.cmp(&right.id));
    let facts = facts(&snapshot, TruthStatus::Established);
    let reference = ValueReference::BoundRawBits {
        component_kind: ComponentKind::ZoneMemory,
        binding: ComponentBindingReference::Projected {
            component_id: "message-session".into(),
            projection: Box::new(ComponentBindingProjection::Zone {
                stage_field: "speaker_stage".into(),
                zone_field: "speaker_zone".into(),
            }),
        },
        byte_offset: 0,
        byte_width: 1,
        mask: 0x0f,
    };

    assert_eq!(
        evaluator(&snapshot, &facts, EvidencePolicy::ESTABLISHED_ONLY).resolve_value(&reference),
        Some(StateValue::Unsigned(6))
    );
    let flow = snapshot
        .environment
        .components
        .iter_mut()
        .find(|component| component.id == "message-session")
        .unwrap();
    let ComponentPayload::Structured { fields } = &mut flow.payload else {
        unreachable!()
    };
    fields.insert("speaker_zone".into(), StateValue::Text("unknown".into()));
    assert_eq!(
        evaluator(&snapshot, &facts, EvidencePolicy::ESTABLISHED_ONLY).resolve_value(&reference),
        None
    );
}

#[test]
fn raw_alias_can_follow_the_active_runtime_file() {
    let mut snapshot = snapshot(0xff);
    let component = &mut snapshot.environment.components[0];
    component.binding = ComponentBinding::RuntimeFile {
        runtime_file_id: "file-0".into(),
    };
    let mut facts = facts(&snapshot, TruthStatus::Established);
    facts.aliases[0].raw.binding = ComponentBindingReference::ActiveRuntimeFile;

    assert_eq!(
        evaluator(&snapshot, &facts, EvidencePolicy::ESTABLISHED_ONLY)
            .evaluate(&fact("story.faron.twilight")),
        EvaluatedTruth::True
    );

    snapshot.environment.active_runtime_file.id = "loaded-runtime".into();
    let component = &mut snapshot.environment.components[0];
    component.binding = ComponentBinding::RuntimeFile {
        runtime_file_id: "loaded-runtime".into(),
    };
    component.serialization_owner = SerializationOwner::RuntimeFile {
        runtime_file_id: "loaded-runtime".into(),
    };
    assert_eq!(
        evaluator(&snapshot, &facts, EvidencePolicy::ESTABLISHED_ONLY)
            .evaluate(&fact("story.faron.twilight")),
        EvaluatedTruth::True
    );
}

#[test]
fn missing_known_bits_and_disallowed_evidence_remain_unknown() {
    let unknown_snapshot = snapshot(0xdf);
    let established = facts(&unknown_snapshot, TruthStatus::Established);
    assert_eq!(
        evaluator(
            &unknown_snapshot,
            &established,
            EvidencePolicy::ESTABLISHED_ONLY,
        )
        .evaluate(&fact("story.faron.twilight")),
        EvaluatedTruth::Unknown
    );

    let observed_snapshot = snapshot(0xff);
    let hypothetical = facts(&observed_snapshot, TruthStatus::Hypothetical);
    assert_eq!(
        evaluator(
            &observed_snapshot,
            &hypothetical,
            EvidencePolicy::ESTABLISHED_ONLY,
        )
        .evaluate(&fact("story.faron.twilight")),
        EvaluatedTruth::Unknown
    );
    assert_eq!(
        evaluator(&observed_snapshot, &hypothetical, EvidencePolicy::RESEARCH,)
            .evaluate(&fact("story.faron.twilight")),
        EvaluatedTruth::True
    );
}

#[test]
fn equivalence_scope_requires_an_explicit_evidenced_set() {
    let snapshot = snapshot(0xff);
    let context = snapshot
        .environment
        .runtime_configuration
        .exact_context()
        .unwrap();
    let scope = ContextScope {
        selectors: vec![ContextSelector::Equivalent {
            equivalence_set_id: "equivalence.sd".into(),
        }],
    };
    let facts = FactCatalog {
        schema: FACT_CATALOG_SCHEMA.into(),
        aliases: vec![FriendlyAlias {
            id: "story.faron.twilight".into(),
            label: "Faron is in twilight".into(),
            scope,
            raw: RawFactBinding {
                component_kind: ComponentKind::PersistentSave,
                binding: ComponentBindingReference::Exact {
                    binding: ComponentBinding::Global,
                },
                byte_offset: 0,
                mask: vec![0x20],
                expected: vec![0x20],
            },
            evidence: evidence(TruthStatus::Established),
        }],
        derived_facts: Vec::new(),
    };
    assert_eq!(
        PredicateEvaluator::new(
            &snapshot,
            &facts,
            &[],
            &BTreeMap::new(),
            EvidencePolicy::ESTABLISHED_ONLY,
        )
        .unwrap()
        .evaluate(&fact("story.faron.twilight")),
        EvaluatedTruth::Unknown
    );

    let mut contexts = vec![
        context,
        ExactContext {
            content_sha256: Digest([2; 32]),
            runtime_configuration_sha256: Digest([3; 32]),
        },
    ];
    contexts.sort();
    let equivalence = EquivalenceSet {
        schema: EQUIVALENCE_SET_SCHEMA.into(),
        id: "equivalence.sd".into(),
        semantic_scope: "story-flags".into(),
        contexts,
        evidence: vec![EquivalenceEvidence {
            kind: EquivalenceEvidenceKind::StaticDiff,
            source_id: "diff.sd".into(),
            source_sha256: Digest([4; 32]),
        }],
    };
    assert_eq!(
        PredicateEvaluator::new(
            &snapshot,
            &facts,
            &[equivalence],
            &BTreeMap::new(),
            EvidencePolicy::ESTABLISHED_ONLY,
        )
        .unwrap()
        .evaluate(&fact("story.faron.twilight")),
        EvaluatedTruth::True
    );
}

#[test]
fn transition_assessment_separates_guards_obligations_and_unknowns() {
    let snapshot = snapshot(0xff);
    let facts = facts(&snapshot, TruthStatus::Established);
    let evaluator = evaluator(&snapshot, &facts, EvidencePolicy::ESTABLISHED_ONLY);
    let mut candidate = transition(&snapshot, fact("story.faron.twilight"));

    let upper = evaluator.assess_transition(
        &candidate,
        &BTreeSet::new(),
        &BTreeSet::new(),
        FeasibilityMode::UpperBound,
    );
    assert_eq!(upper.classification, TransitionClassification::Executable);
    assert_eq!(upper.outstanding_obligation_ids, vec!["obligation.wall"]);

    let modeled = evaluator.assess_transition(
        &candidate,
        &BTreeSet::new(),
        &BTreeSet::new(),
        FeasibilityMode::Modeled,
    );
    assert_eq!(modeled.classification, TransitionClassification::Obstructed);

    candidate.activation.hard_guards = PredicateExpression::False;
    assert_eq!(
        evaluator
            .assess_transition(
                &candidate,
                &BTreeSet::new(),
                &BTreeSet::new(),
                FeasibilityMode::Modeled,
            )
            .classification,
        TransitionClassification::GuardBlocked
    );

    candidate.activation.hard_guards = PredicateExpression::True;
    candidate.activation.unknown_requirements = vec![UnknownRequirement {
        id: "unknown.trigger-semantics".into(),
        description: "The encoded exit does not establish activation physics.".into(),
        evidence: evidence(TruthStatus::Established),
    }];
    let assessment = evaluator.assess_transition(
        &candidate,
        &BTreeSet::from(["obligation.wall".into()]),
        &BTreeSet::new(),
        FeasibilityMode::UpperBound,
    );
    assert_eq!(
        assessment.classification,
        TransitionClassification::FeasibilityUnknown
    );
    assert_eq!(
        assessment.unknown_requirement_ids,
        vec!["unknown.trigger-semantics"]
    );
}

#[test]
fn writer_activation_and_gate_suppression_are_distinct_states() {
    let snapshot = snapshot(0xff);
    let facts = facts(&snapshot, TruthStatus::Established);
    let evaluator = evaluator(&snapshot, &facts, EvidencePolicy::ESTABLISHED_ONLY);
    let writer = WriterRule {
        id: "writer.return-place".into(),
        scope: scope(&snapshot),
        activation: PredicateExpression::True,
        operation: crate::transition::StateOperation::SetLocation {
            location: snapshot.environment.location.clone(),
        },
        evidence: evidence(TruthStatus::Established),
    };
    let mut gate = GateRule {
        id: "gate.no-teleport".into(),
        scope: scope(&snapshot),
        active_when: PredicateExpression::True,
        blocked_writer_ids: vec![writer.id.clone()],
        lifetime: SemanticLifetime::RuntimeFile,
        evidence: evidence(TruthStatus::Established),
    };

    let blocked = evaluator.assess_writer(&writer, &[gate.clone()]);
    assert_eq!(blocked.classification, WriterClassification::GateBlocked);
    assert_eq!(blocked.active_gate_ids, vec!["gate.no-teleport"]);

    gate.active_when = PredicateExpression::Fact {
        fact_id: "missing.gate-source".into(),
    };
    let uncertain = evaluator.assess_writer(&writer, &[gate.clone()]);
    assert_eq!(uncertain.classification, WriterClassification::GateUnknown);
    assert_eq!(uncertain.unknown_gate_ids, vec!["gate.no-teleport"]);

    gate.active_when = PredicateExpression::False;
    assert_eq!(
        evaluator.assess_writer(&writer, &[gate]).classification,
        WriterClassification::Executable
    );

    let mut inactive = writer;
    inactive.activation = PredicateExpression::False;
    assert_eq!(
        evaluator.assess_writer(&inactive, &[]).classification,
        WriterClassification::Inactive
    );
}

#[test]
fn readers_keep_raw_source_and_friendly_interpretation_separate() {
    let snapshot = snapshot(0xff);
    let facts = facts(&snapshot, TruthStatus::Established);
    let evaluator = evaluator(&snapshot, &facts, EvidencePolicy::ESTABLISHED_ONLY);
    let reader = ReaderRule {
        id: "reader.savewarp".into(),
        scope: scope(&snapshot),
        source: ValueReference::LocationStage,
        consuming_transition_id: "transition.savewarp".into(),
        interpretation_fact_id: Some("story.faron.twilight".into()),
        evidence: evidence(TruthStatus::Established),
    };
    let assessment = evaluator.assess_reader(&reader);
    assert_eq!(
        assessment.source_value,
        Some(StateValue::Text("F_SP103".into()))
    );
    assert_eq!(assessment.interpretation, Some(EvaluatedTruth::True));
}

#[test]
fn resolvers_discharge_obligations_without_deleting_active_obstructions() {
    let snapshot = snapshot(0xff);
    let facts = facts(&snapshot, TruthStatus::Established);
    let evaluator = evaluator(&snapshot, &facts, EvidencePolicy::ESTABLISHED_ONLY);
    let candidate = transition(&snapshot, PredicateExpression::True);
    let mut obstruction = Obstruction {
        id: "obstruction.npc-blocker".into(),
        label: "NPCs block the entrance".into(),
        scope: scope(&snapshot),
        blocked_action_id: candidate.id.clone(),
        approach_id: candidate.approach_id.clone(),
        active_when: PredicateExpression::True,
        obligation_ids: vec!["obligation.wall".into()],
        evidence: evidence(TruthStatus::Established),
    };
    let resolver = ObstructionResolver {
        id: "resolver.text-displacement".into(),
        label: "Displace the blocking text state".into(),
        scope: scope(&snapshot),
        obstruction_id: obstruction.id.clone(),
        resolution_kind: crate::transition::ResolutionKind::Bypass,
        applicable_when: fact("story.faron.twilight"),
        operations: Vec::new(),
        evidence: evidence(TruthStatus::Established),
    };

    let unresolved = evaluator.resolve_feasibility(
        &candidate,
        &[],
        &[obstruction.clone()],
        &[],
        &[],
        FeasibilitySelection {
            resolver_ids: &BTreeSet::new(),
            technique_ids: &BTreeSet::new(),
            already_discharged: &BTreeSet::new(),
            microtraces: &[],
        },
    );
    assert_eq!(
        unresolved.active_obstruction_ids,
        vec!["obstruction.npc-blocker"]
    );
    assert!(
        !unresolved
            .discharged_obligation_ids
            .contains("obligation.wall")
    );

    let resolved = evaluator.resolve_feasibility(
        &candidate,
        &[],
        &[obstruction.clone()],
        &[resolver],
        &[],
        FeasibilitySelection {
            resolver_ids: &BTreeSet::from(["resolver.text-displacement".into()]),
            technique_ids: &BTreeSet::new(),
            already_discharged: &BTreeSet::new(),
            microtraces: &[],
        },
    );
    assert_eq!(
        resolved.active_obstruction_ids,
        vec!["obstruction.npc-blocker"]
    );
    assert_eq!(
        resolved.applied_resolver_ids,
        vec!["resolver.text-displacement"]
    );
    assert!(
        resolved
            .discharged_obligation_ids
            .contains("obligation.wall")
    );

    obstruction.active_when = fact("missing.obstruction-state");
    let uncertain = evaluator.resolve_feasibility(
        &candidate,
        &[],
        &[obstruction],
        &[],
        &[],
        FeasibilitySelection {
            resolver_ids: &BTreeSet::new(),
            technique_ids: &BTreeSet::new(),
            already_discharged: &BTreeSet::new(),
            microtraces: &[],
        },
    );
    assert_eq!(
        uncertain.unknown_obstruction_ids,
        vec!["obstruction.npc-blocker"]
    );
}

#[test]
fn techniques_discharge_and_introduce_only_named_obligations() {
    let snapshot = snapshot(0xff);
    let facts = facts(&snapshot, TruthStatus::Established);
    let evaluator = evaluator(&snapshot, &facts, EvidencePolicy::ESTABLISHED_ONLY);
    let candidate = transition(&snapshot, PredicateExpression::True);
    let technique = Technique {
        id: "technique.epona-oob".into(),
        label: "Epona out of bounds".into(),
        scope: scope(&snapshot),
        prerequisites: fact("story.faron.twilight"),
        operations: Vec::new(),
        discharged_obligation_ids: vec!["obligation.wall".into()],
        introduced_obligation_ids: vec!["obligation.precise-movement".into()],
        cost: crate::transition::RouteCost {
            axes: BTreeMap::from([("difficulty".into(), 5)]),
        },
        evidence: evidence(TruthStatus::Established),
    };
    let resolution = evaluator.resolve_feasibility(
        &candidate,
        &[],
        &[],
        &[],
        &[technique],
        FeasibilitySelection {
            resolver_ids: &BTreeSet::new(),
            technique_ids: &BTreeSet::from(["technique.epona-oob".into()]),
            already_discharged: &BTreeSet::from(["obligation.precise-movement".into()]),
            microtraces: &[],
        },
    );
    assert_eq!(
        resolution.applicable_technique_ids,
        vec!["technique.epona-oob"]
    );
    assert!(
        resolution
            .discharged_obligation_ids
            .contains("obligation.wall")
    );
    assert!(
        !resolution
            .discharged_obligation_ids
            .contains("obligation.precise-movement")
    );
}

#[test]
fn predicate_obligations_derive_discharge_and_unknownness_from_state() {
    let snapshot = snapshot(0xff);
    let facts = facts(&snapshot, TruthStatus::Established);
    let evaluator = evaluator(&snapshot, &facts, EvidencePolicy::ESTABLISHED_ONLY);
    let candidate = transition(&snapshot, PredicateExpression::True);
    let mut obligation = FeasibilityObligation {
        id: "obligation.wall".into(),
        label: "State-derived wall condition".into(),
        scope: scope(&snapshot),
        obligation_kind: ObligationKind::Twilight,
        stage: crate::transition::ObligationStage::Activate,
        detail: ObligationDetail::Predicate {
            predicate: fact("story.faron.twilight"),
        },
        evidence: evidence(TruthStatus::Established),
    };

    let satisfied = evaluator.resolve_feasibility(
        &candidate,
        std::slice::from_ref(&obligation),
        &[],
        &[],
        &[],
        FeasibilitySelection {
            resolver_ids: &BTreeSet::new(),
            technique_ids: &BTreeSet::new(),
            already_discharged: &BTreeSet::new(),
            microtraces: &[],
        },
    );
    assert!(
        satisfied
            .discharged_obligation_ids
            .contains("obligation.wall")
    );
    assert!(satisfied.unknown_obligation_ids.is_empty());

    obligation.detail = ObligationDetail::Predicate {
        predicate: PredicateExpression::False,
    };
    let unsatisfied = evaluator.resolve_feasibility(
        &candidate,
        std::slice::from_ref(&obligation),
        &[],
        &[],
        &[],
        FeasibilitySelection {
            resolver_ids: &BTreeSet::new(),
            technique_ids: &BTreeSet::new(),
            already_discharged: &BTreeSet::new(),
            microtraces: &[],
        },
    );
    assert!(unsatisfied.discharged_obligation_ids.is_empty());
    assert!(unsatisfied.unknown_obligation_ids.is_empty());

    obligation.detail = ObligationDetail::Predicate {
        predicate: PredicateExpression::Fact {
            fact_id: "missing.twilight-state".into(),
        },
    };
    let unknown = evaluator.resolve_feasibility(
        &candidate,
        &[obligation],
        &[],
        &[],
        &[],
        FeasibilitySelection {
            resolver_ids: &BTreeSet::new(),
            technique_ids: &BTreeSet::new(),
            already_discharged: &BTreeSet::new(),
            microtraces: &[],
        },
    );
    assert!(unknown.discharged_obligation_ids.is_empty());
    assert_eq!(
        unknown.unknown_obligation_ids,
        BTreeSet::from(["obligation.wall".into()])
    );
}

#[test]
fn interaction_obligations_derive_volume_pose_direction_and_action_from_state() {
    let mut snapshot = snapshot(0xff);
    snapshot.environment.spatial_volumes = vec![
        SpatialVolume {
            object_id: "actor.auru".into(),
            volume_id: "cutscene-trigger".into(),
            shape: SpatialVolumeShape::AxisAlignedBox {
                minimum: [0.5, 0.5, 0.5],
                maximum: [1.5, 1.5, 1.5],
            },
            source_sha256: Digest([5; 32]),
        },
        SpatialVolume {
            object_id: "actor.auru".into(),
            volume_id: "talk".into(),
            shape: SpatialVolumeShape::AxisAlignedBox {
                minimum: [-2.0, -2.0, -2.0],
                maximum: [2.0, 2.0, 2.0],
            },
            source_sha256: Digest([6; 32]),
        },
    ];
    snapshot.environment.live_world_objects = vec![LiveWorldObject {
        instance_id: "actor.auru".into(),
        static_object_id: Some("actor.auru".into()),
        actor_type: "npc.auru".into(),
        lifecycle: ActorLifecycle::Loaded,
        fields: BTreeMap::new(),
    }];
    snapshot.environment.player.rotation[1] = 0x1000;
    snapshot.environment.validate().unwrap();
    let facts = facts(&snapshot, TruthStatus::Established);
    let pose = PredicateExpression::All {
        terms: vec![
            PredicateExpression::Compare {
                left: ValueReference::PlayerRotationY,
                operator: ComparisonOperator::Equal,
                right: ValueReference::Literal {
                    value: StateValue::Signed(0x1000),
                },
            },
            PredicateExpression::Compare {
                left: ValueReference::PlayerAction,
                operator: ComparisonOperator::Equal,
                right: ValueReference::Literal {
                    value: StateValue::Text("idle".into()),
                },
            },
            PredicateExpression::Compare {
                left: ValueReference::PlayerControl,
                operator: ComparisonOperator::Equal,
                right: ValueReference::Literal {
                    value: StateValue::Boolean(true),
                },
            },
        ],
    };
    let mut obligation = FeasibilityObligation {
        id: "obligation.auru-talk".into(),
        label: "Talk to Auru without entering his cutscene trigger".into(),
        scope: scope(&snapshot),
        obligation_kind: ObligationKind::Interaction,
        stage: crate::transition::ObligationStage::Activate,
        detail: ObligationDetail::Interaction {
            actor_instance_id: "actor.auru".into(),
            interaction_mode: "talk".into(),
            required_volumes: vec![VolumeReference {
                object_id: "actor.auru".into(),
                volume_id: "talk".into(),
            }],
            excluded_volumes: vec![VolumeReference {
                object_id: "actor.auru".into(),
                volume_id: "cutscene-trigger".into(),
            }],
            pose_predicate: pose,
            temporal_requirement: None,
        },
        evidence: evidence(TruthStatus::Established),
    };

    assert_eq!(
        evaluator(&snapshot, &facts, EvidencePolicy::ESTABLISHED_ONLY)
            .assess_obligation(&obligation, &[])
            .classification,
        ObligationClassification::Satisfied
    );

    snapshot.environment.player.position = [1.0, 1.0, 1.0];
    assert_eq!(
        evaluator(&snapshot, &facts, EvidencePolicy::ESTABLISHED_ONLY)
            .assess_obligation(&obligation, &[])
            .classification,
        ObligationClassification::Unsatisfied
    );

    let ObligationDetail::Interaction {
        required_volumes, ..
    } = &mut obligation.detail
    else {
        unreachable!();
    };
    required_volumes[0].volume_id = "missing-talk-volume".into();
    snapshot.environment.player.position = [0.0, 0.0, 0.0];
    assert_eq!(
        evaluator(&snapshot, &facts, EvidencePolicy::ESTABLISHED_ONLY)
            .assess_obligation(&obligation, &[])
            .classification,
        ObligationClassification::EvaluationUnknown
    );

    let ObligationDetail::Interaction {
        required_volumes,
        temporal_requirement,
        ..
    } = &mut obligation.detail
    else {
        unreachable!();
    };
    required_volumes[0].volume_id = "talk".into();
    *temporal_requirement = Some(TemporalRequirement {
        action_id: "dialogue.auru".into(),
        window: TemporalWindow {
            earliest_frame: 0,
            latest_frame: 1,
            required_input: Some("sidehop".into()),
        },
    });
    assert_eq!(
        evaluator(&snapshot, &facts, EvidencePolicy::ESTABLISHED_ONLY)
            .assess_obligation(&obligation, &[])
            .classification,
        ObligationClassification::EvaluationUnknown
    );

    let mut microtrace = WitnessedMicrotrace {
        id: "microtrace.auru-sidehop".into(),
        scope: scope(&snapshot),
        precondition: PredicateExpression::True,
        operations: vec![StateOperation::Interrupt {
            action_id: "dialogue.auru".into(),
            window: TemporalWindow {
                earliest_frame: 1,
                latest_frame: 1,
                required_input: Some("sidehop".into()),
            },
        }],
        postcondition: PredicateExpression::True,
        timing: TemporalWindow {
            earliest_frame: 1,
            latest_frame: 1,
            required_input: Some("sidehop".into()),
        },
        evidence: evidence(TruthStatus::Established),
    };
    let timed = evaluator(&snapshot, &facts, EvidencePolicy::ESTABLISHED_ONLY)
        .assess_obligation(&obligation, std::slice::from_ref(&microtrace));
    assert_eq!(timed.classification, ObligationClassification::Satisfied);
    assert_eq!(
        timed.supporting_microtrace_ids,
        vec!["microtrace.auru-sidehop"]
    );

    snapshot.environment.player.position = [1.0, 1.0, 1.0];
    let spatially_blocked = evaluator(&snapshot, &facts, EvidencePolicy::ESTABLISHED_ONLY)
        .assess_obligation(&obligation, std::slice::from_ref(&microtrace));
    assert_eq!(
        spatially_blocked.classification,
        ObligationClassification::Unsatisfied
    );
    assert!(spatially_blocked.supporting_microtrace_ids.is_empty());
    snapshot.environment.player.position = [0.0, 0.0, 0.0];

    let StateOperation::Interrupt { action_id, .. } = &mut microtrace.operations[0] else {
        unreachable!();
    };
    *action_id = "dialogue.unrelated".into();
    assert_eq!(
        evaluator(&snapshot, &facts, EvidencePolicy::ESTABLISHED_ONLY)
            .assess_obligation(&obligation, std::slice::from_ref(&microtrace))
            .classification,
        ObligationClassification::EvaluationUnknown
    );
    let StateOperation::Interrupt { action_id, .. } = &mut microtrace.operations[0] else {
        unreachable!();
    };
    *action_id = "dialogue.auru".into();

    microtrace.evidence = evidence(TruthStatus::Hypothetical);
    assert_eq!(
        evaluator(&snapshot, &facts, EvidencePolicy::ESTABLISHED_ONLY)
            .assess_obligation(&obligation, std::slice::from_ref(&microtrace))
            .classification,
        ObligationClassification::EvaluationUnknown
    );
    assert_eq!(
        evaluator(&snapshot, &facts, EvidencePolicy::RESEARCH)
            .assess_obligation(&obligation, std::slice::from_ref(&microtrace))
            .classification,
        ObligationClassification::Satisfied
    );

    let ObligationDetail::Interaction {
        temporal_requirement,
        ..
    } = &mut obligation.detail
    else {
        unreachable!();
    };
    *temporal_requirement = None;
    snapshot.environment.live_world_objects[0].lifecycle = ActorLifecycle::Destroyed;
    assert_eq!(
        evaluator(&snapshot, &facts, EvidencePolicy::ESTABLISHED_ONLY)
            .assess_obligation(&obligation, &[])
            .classification,
        ObligationClassification::Unsatisfied
    );
}

#[test]
fn geometry_and_plane_obligations_derive_from_exact_spatial_observations() {
    let mut snapshot = snapshot(0xff);
    snapshot.environment.player.position = [0.0, 5.0, 0.0];
    snapshot.environment.spatial_connections = vec![SpatialConnection {
        approach_id: "approach.front".into(),
        source_region_id: "region.before-wall".into(),
        destination_region_id: "region.exit".into(),
        status: SpatialConnectionStatus::Blocked,
        source_sha256: Digest([7; 32]),
    }];
    snapshot.environment.spatial_planes = vec![SpatialPlane {
        plane_id: "void.room-0".into(),
        normal: [0.0, 1.0, 0.0],
        offset: -2.0,
        source_sha256: Digest([8; 32]),
    }];
    snapshot.environment.validate().unwrap();
    let facts = facts(&snapshot, TruthStatus::Established);
    let mut geometry = FeasibilityObligation {
        id: "obligation.wall".into(),
        label: "Reach the exit region".into(),
        scope: scope(&snapshot),
        obligation_kind: ObligationKind::Geometry,
        stage: crate::transition::ObligationStage::Reach,
        detail: ObligationDetail::Geometry {
            approach_id: "approach.front".into(),
            source_region_id: "region.before-wall".into(),
            destination_region_id: "region.exit".into(),
        },
        evidence: evidence(TruthStatus::Established),
    };
    let void_side = FeasibilityObligation {
        id: "obligation.above-void".into(),
        label: "Remain on the non-void side".into(),
        scope: scope(&snapshot),
        obligation_kind: ObligationKind::VoidPlane,
        stage: crate::transition::ObligationStage::Activate,
        detail: ObligationDetail::PlaneSide {
            plane_id: "void.room-0".into(),
            relation: PlaneRelation::NonNegative,
        },
        evidence: evidence(TruthStatus::Established),
    };
    snapshot.environment.player.rotation[1] = i16::MIN;
    let facing = FeasibilityObligation {
        id: "obligation.face-door".into(),
        label: "Face the door across the signed binary-angle wrap".into(),
        scope: scope(&snapshot),
        obligation_kind: ObligationKind::Interaction,
        stage: crate::transition::ObligationStage::Activate,
        detail: ObligationDetail::Facing {
            yaw: ValueReference::PlayerRotationY,
            target_yaw: i16::MAX,
            maximum_delta: 1,
        },
        evidence: evidence(TruthStatus::Established),
    };

    let initial_evaluator = evaluator(&snapshot, &facts, EvidencePolicy::ESTABLISHED_ONLY);
    assert_eq!(
        initial_evaluator
            .assess_obligation(&facing, &[])
            .classification,
        ObligationClassification::Satisfied
    );
    assert_eq!(
        initial_evaluator
            .assess_obligation(&geometry, &[])
            .classification,
        ObligationClassification::Unsatisfied
    );
    assert_eq!(
        initial_evaluator
            .assess_obligation(&void_side, &[])
            .classification,
        ObligationClassification::Satisfied
    );

    snapshot.environment.spatial_connections[0].status = SpatialConnectionStatus::Traversable;
    snapshot.environment.player.position[1] = 1.0;
    let moved_evaluator = evaluator(&snapshot, &facts, EvidencePolicy::ESTABLISHED_ONLY);
    assert_eq!(
        moved_evaluator
            .assess_obligation(&geometry, &[])
            .classification,
        ObligationClassification::Satisfied
    );
    assert_eq!(
        moved_evaluator
            .assess_obligation(&void_side, &[])
            .classification,
        ObligationClassification::Unsatisfied
    );

    let ObligationDetail::Geometry {
        destination_region_id,
        ..
    } = &mut geometry.detail
    else {
        unreachable!();
    };
    *destination_region_id = "region.unknown".into();
    assert_eq!(
        evaluator(&snapshot, &facts, EvidencePolicy::ESTABLISHED_ONLY)
            .assess_obligation(&geometry, &[])
            .classification,
        ObligationClassification::EvaluationUnknown
    );

    snapshot.environment.execution_context = ExecutionContext::Process {
        process_name: "PROC_OPENING_SCENE".into(),
        pending_world_load: None,
    };
    let process_evaluator = evaluator(&snapshot, &facts, EvidencePolicy::ESTABLISHED_ONLY);
    assert_eq!(
        process_evaluator
            .assess_obligation(&void_side, &[])
            .classification,
        ObligationClassification::EvaluationUnknown
    );
}

#[test]
fn sphere_and_cylinder_volume_boundaries_are_inclusive() {
    let mut snapshot = snapshot(0xff);
    snapshot.environment.live_world_objects = vec![LiveWorldObject {
        instance_id: "actor.test".into(),
        static_object_id: None,
        actor_type: "npc.test".into(),
        lifecycle: ActorLifecycle::Loaded,
        fields: BTreeMap::from([("ready".into(), StateValue::Boolean(true))]),
    }];
    snapshot.environment.components.push(StateComponent {
        id: "actor-instance.test".into(),
        component_kind: ComponentKind::ActorInstance,
        payload: ComponentPayload::Structured {
            fields: BTreeMap::from([("executing".into(), StateValue::Boolean(true))]),
        },
        binding: ComponentBinding::Actor {
            instance_id: "actor.test".into(),
        },
        lifetime: SemanticLifetime::RoomLoad,
        serialization_owner: SerializationOwner::None,
        provenance: vec![ComponentProvenance {
            source_kind: ProvenanceSourceKind::TraceObservation,
            source_id: "trace.actor-test".into(),
            source_sha256: Some(Digest([12; 32])),
            transition_id: None,
        }],
    });
    snapshot
        .environment
        .components
        .sort_by(|left, right| left.id.cmp(&right.id));
    snapshot.environment.spatial_volumes = vec![
        SpatialVolume {
            object_id: "actor.test".into(),
            volume_id: "cylinder".into(),
            shape: SpatialVolumeShape::VerticalCylinder {
                center_xz: [0.0, 0.0],
                minimum_y: -1.0,
                maximum_y: 1.0,
                radius: 2.0,
            },
            source_sha256: Digest([10; 32]),
        },
        SpatialVolume {
            object_id: "actor.test".into(),
            volume_id: "oriented".into(),
            shape: SpatialVolumeShape::YawOrientedRectangle {
                origin_xz: [0.0, 0.0],
                yaw: 0x4000,
                minimum_local_xz: [-1.0, 0.0],
                maximum_local_xz: [1.0, 3.0],
            },
            source_sha256: Digest([13; 32]),
        },
        SpatialVolume {
            object_id: "actor.test".into(),
            volume_id: "sphere".into(),
            shape: SpatialVolumeShape::Sphere {
                center: [0.0, 0.0, 0.0],
                radius: 2.0,
            },
            source_sha256: Digest([11; 32]),
        },
    ];
    snapshot.environment.player.position = [2.0, 0.0, 0.0];
    snapshot.environment.validate().unwrap();
    let facts = facts(&snapshot, TruthStatus::Established);
    let obligation_scope = scope(&snapshot);
    let obligation = |volume_id: &str| FeasibilityObligation {
        id: format!("obligation.{volume_id}"),
        label: format!("Inside {volume_id}"),
        scope: obligation_scope.clone(),
        obligation_kind: ObligationKind::Interaction,
        stage: crate::transition::ObligationStage::Activate,
        detail: ObligationDetail::Interaction {
            actor_instance_id: "actor.test".into(),
            interaction_mode: "talk".into(),
            required_volumes: vec![VolumeReference {
                object_id: "actor.test".into(),
                volume_id: volume_id.into(),
            }],
            excluded_volumes: Vec::new(),
            pose_predicate: PredicateExpression::True,
            temporal_requirement: None,
        },
        evidence: evidence(TruthStatus::Established),
    };
    let world_evaluator = evaluator(&snapshot, &facts, EvidencePolicy::ESTABLISHED_ONLY);
    assert_eq!(
        world_evaluator
            .assess_obligation(&obligation("sphere"), &[])
            .classification,
        ObligationClassification::Satisfied
    );
    assert_eq!(
        world_evaluator
            .assess_obligation(&obligation("cylinder"), &[])
            .classification,
        ObligationClassification::Satisfied
    );
    assert_eq!(
        world_evaluator
            .assess_obligation(&obligation("oriented"), &[])
            .classification,
        ObligationClassification::Satisfied
    );
    assert_eq!(
        world_evaluator.resolve_value(&ValueReference::ActorField {
            instance_id: "actor.test".into(),
            field: "ready".into(),
        }),
        Some(StateValue::Boolean(true))
    );

    snapshot.environment.player.position = [0.0, 99.0, 2.0];
    assert_eq!(
        evaluator(&snapshot, &facts, EvidencePolicy::ESTABLISHED_ONLY)
            .assess_obligation(&obligation("oriented"), &[])
            .classification,
        ObligationClassification::Unsatisfied,
        "oriented rectangles rotate with actor yaw and intentionally ignore height"
    );

    snapshot.environment.execution_context = ExecutionContext::Process {
        process_name: "PROC_OPENING_SCENE".into(),
        pending_world_load: None,
    };
    let process_evaluator = evaluator(&snapshot, &facts, EvidencePolicy::ESTABLISHED_ONLY);
    assert_eq!(
        process_evaluator
            .assess_obligation(&obligation("sphere"), &[])
            .classification,
        ObligationClassification::EvaluationUnknown
    );
    assert_eq!(
        process_evaluator.resolve_value(&ValueReference::ActorField {
            instance_id: "actor.test".into(),
            field: "ready".into(),
        }),
        None
    );
    assert_eq!(
        process_evaluator.resolve_value(&ValueReference::ComponentField {
            component_id: "actor-instance.test".into(),
            field: "executing".into(),
        }),
        None
    );
}

#[test]
fn compound_interaction_selects_form_branch_and_keeps_attention_distinct() {
    let mut snapshot = snapshot(0xff);
    snapshot.environment.player.form = PlayerForm::Wolf;
    snapshot.environment.player.position = [100.0, 0.0, 1000.0];
    snapshot.environment.player.attention_position = Some([150.0, 0.0, 50.0]);
    snapshot.environment.live_world_objects = vec![LiveWorldObject {
        instance_id: "actor.boss-door".into(),
        static_object_id: None,
        actor_type: "door.boss-l1".into(),
        lifecycle: ActorLifecycle::Loaded,
        fields: BTreeMap::new(),
    }];
    snapshot.environment.spatial_volumes = vec![
        SpatialVolume {
            object_id: "actor.boss-door".into(),
            volume_id: "check-area".into(),
            shape: SpatialVolumeShape::YawOrientedRectangle {
                origin_xz: [0.0, 0.0],
                yaw: 0,
                minimum_local_xz: [-200.0, -100.0],
                maximum_local_xz: [200.0, 100.0],
            },
            source_sha256: Digest([20; 32]),
        },
        SpatialVolume {
            object_id: "actor.boss-door".into(),
            volume_id: "wolf-current-x".into(),
            shape: SpatialVolumeShape::YawOrientedStrip {
                origin_xz: [0.0, 0.0],
                yaw: 0,
                axis: SpatialLocalAxis::X,
                minimum: -130.0,
                maximum: 130.0,
            },
            source_sha256: Digest([20; 32]),
        },
    ];
    snapshot.environment.validate().unwrap();
    let reference = |volume_id: &str, position| InteractionVolumeTest {
        position,
        volume: VolumeReference {
            object_id: "actor.boss-door".into(),
            volume_id: volume_id.into(),
        },
        must_be_inside: true,
    };
    let form_is = |form: &str| PredicateExpression::Compare {
        left: ValueReference::PlayerForm,
        operator: ComparisonOperator::Equal,
        right: ValueReference::Literal {
            value: StateValue::Text(form.into()),
        },
    };
    let obligation = FeasibilityObligation {
        id: "obligation.boss-door-area".into(),
        label: "Satisfy form-specific boss-door area checks".into(),
        scope: scope(&snapshot),
        obligation_kind: ObligationKind::Interaction,
        stage: crate::transition::ObligationStage::Activate,
        detail: ObligationDetail::CompoundInteraction {
            actor_instance_id: "actor.boss-door".into(),
            interaction_mode: "door".into(),
            branches: vec![
                InteractionBranch {
                    when: form_is("human"),
                    volume_tests: vec![reference("check-area", InteractionPosition::Player)],
                    pose_predicate: PredicateExpression::True,
                },
                InteractionBranch {
                    when: form_is("wolf"),
                    volume_tests: vec![
                        reference("check-area", InteractionPosition::PlayerAttention),
                        reference("wolf-current-x", InteractionPosition::Player),
                    ],
                    pose_predicate: PredicateExpression::True,
                },
            ],
            temporal_requirement: None,
        },
        evidence: evidence(TruthStatus::Established),
    };
    let facts = facts(&snapshot, TruthStatus::Established);
    assert_eq!(
        evaluator(&snapshot, &facts, EvidencePolicy::ESTABLISHED_ONLY)
            .assess_obligation(&obligation, &[])
            .classification,
        ObligationClassification::Satisfied
    );

    snapshot.environment.player.attention_position = None;
    assert_eq!(
        evaluator(&snapshot, &facts, EvidencePolicy::ESTABLISHED_ONLY)
            .assess_obligation(&obligation, &[])
            .classification,
        ObligationClassification::EvaluationUnknown
    );

    snapshot.environment.player.form = PlayerForm::Human;
    snapshot.environment.player.position = [150.0, 999.0, 50.0];
    assert_eq!(
        evaluator(&snapshot, &facts, EvidencePolicy::ESTABLISHED_ONLY)
            .assess_obligation(&obligation, &[])
            .classification,
        ObligationClassification::Satisfied,
        "the inactive wolf branch must not demand an attention observation"
    );

    snapshot.environment.player.form = PlayerForm::Wolf;
    snapshot.environment.player.attention_position = Some([150.0, 0.0, 50.0]);
    snapshot.environment.player.position = [131.0, 0.0, 0.0];
    assert_eq!(
        evaluator(&snapshot, &facts, EvidencePolicy::ESTABLISHED_ONLY)
            .assess_obligation(&obligation, &[])
            .classification,
        ObligationClassification::Unsatisfied
    );
}
