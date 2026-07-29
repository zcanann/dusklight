use super::*;
use crate::artifact::Digest;
use crate::identity::{ContextSelector, ExactContext};
use crate::logic::{
    ComparisonOperator, ContextScope, EvidenceKind, EvidenceRecord, FACT_CATALOG_SCHEMA,
    RuleEvidence, TruthStatus,
};
use crate::state::{SemanticLifetime, StateValue};
use crate::transition::{
    ActivationContract, CandidateTransition, ComponentFieldTarget, FeasibilityObligation, GateRule,
    MECHANICS_CATALOG_SCHEMA, ObligationDetail, ObligationKind, Obstruction, ObstructionResolver,
    ReaderRule, ResolutionKind, RouteCost, Technique, TemporalRequirement, TemporalWindow,
    TransitionKind, WitnessedMicrotrace, WriterRule,
};

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

fn evidence() -> RuleEvidence {
    RuleEvidence {
        truth: TruthStatus::Established,
        records: vec![EvidenceRecord {
            id: "test.evidence".into(),
            kind: EvidenceKind::SourceAudited,
            source_sha256: Some(Digest([3; 32])),
            note: "Synthetic relevance acceptance evidence.".into(),
        }],
    }
}

fn field(name: &str) -> ValueReference {
    ValueReference::ComponentField {
        component_id: "state.route".into(),
        field: name.into(),
    }
}

fn equals(name: &str, value: u64) -> PredicateExpression {
    PredicateExpression::Compare {
        left: field(name),
        operator: ComparisonOperator::Equal,
        right: ValueReference::Literal {
            value: StateValue::Unsigned(value),
        },
    }
}

fn write(name: &str, value: u64) -> StateOperation {
    StateOperation::Write {
        target: ComponentFieldTarget {
            component_id: "state.route".into(),
            field: name.into(),
        },
        value: StateValue::Unsigned(value),
    }
}

fn transition(
    id: &str,
    guard: PredicateExpression,
    effects: Vec<StateOperation>,
) -> CandidateTransition {
    CandidateTransition {
        id: id.into(),
        label: id.into(),
        scope: scope(),
        transition_kind: TransitionKind::Other,
        approach_id: format!("approach.{id}"),
        activation: ActivationContract {
            hard_guards: guard,
            physical_obligation_ids: Vec::new(),
            effects,
            unknown_requirements: Vec::new(),
        },
        evidence: evidence(),
    }
}

#[test]
fn expands_all_producers_through_cycles_readers_and_writer_gates() {
    let facts = FactCatalog {
        schema: FACT_CATALOG_SCHEMA.into(),
        aliases: Vec::new(),
        derived_facts: Vec::new(),
    };
    let goal_guard = PredicateExpression::Any {
        terms: vec![equals("alternate", 1), equals("middle", 1)],
    };
    let mut mechanics = MechanicsCatalog {
        schema: MECHANICS_CATALOG_SCHEMA.into(),
        transitions: vec![
            transition(
                "transition.alternate",
                equals("root", 1),
                vec![write("alternate", 1)],
            ),
            transition(
                "transition.cycle",
                equals("middle", 1),
                vec![write("root", 1)],
            ),
            transition("transition.goal", goal_guard, vec![write("final", 1)]),
            transition(
                "transition.middle",
                equals("root", 1),
                vec![write("middle", 1)],
            ),
            transition(
                "transition.noise",
                PredicateExpression::True,
                vec![write("noise", 1)],
            ),
            transition(
                "transition.unlock-writer",
                PredicateExpression::True,
                vec![write("writer_blocked", 0)],
            ),
        ],
        obligations: Vec::new(),
        writers: vec![WriterRule {
            id: "writer.recent-item".into(),
            scope: scope(),
            activation: PredicateExpression::True,
            operation: write("recent_item", 0x4a),
            evidence: evidence(),
        }],
        gates: vec![GateRule {
            id: "gate.recent-item-writer".into(),
            scope: scope(),
            active_when: equals("writer_blocked", 1),
            blocked_writer_ids: vec!["writer.recent-item".into()],
            lifetime: SemanticLifetime::Session,
            evidence: evidence(),
        }],
        readers: vec![ReaderRule {
            id: "reader.goal-recent-item".into(),
            scope: scope(),
            source: field("recent_item"),
            consuming_transition_id: "transition.goal".into(),
            interpretation_fact_id: None,
            evidence: evidence(),
        }],
        reconstruction_rules: Vec::new(),
        obstructions: Vec::new(),
        resolvers: Vec::new(),
        techniques: Vec::new(),
        microtraces: Vec::new(),
        goals: Vec::new(),
    };
    mechanics
        .transitions
        .sort_by(|left, right| left.id.cmp(&right.id));

    let relevance = BackwardRelevance::analyze(&facts, &mechanics, &equals("final", 1))
        .expect("catalog should expand");
    assert_eq!(
        relevance.transition_ids,
        vec![
            "transition.alternate",
            "transition.cycle",
            "transition.goal",
            "transition.middle",
            "transition.unlock-writer",
        ]
    );
    assert_eq!(relevance.writer_ids, vec!["writer.recent-item"]);
    assert_eq!(relevance.gate_ids, vec!["gate.recent-item-writer"]);
    assert_eq!(relevance.reader_ids, vec!["reader.goal-recent-item"]);
    assert!(!relevance.contains_transition("transition.noise"));
    assert!(relevance.frontier_dependencies.is_empty());
}

#[test]
fn item_slot_normalization_declares_both_inputs_and_outputs() {
    let operation = StateOperation::NormalizeItemSlotsAndLineup {
        component_id: "save.items".into(),
        inventory_field: "inventory".into(),
        lineup_field: "item_lineup".into(),
        primary_slot: 9,
        secondary_slot: 10,
        single_item: 0x44,
        combined_item: 0x47,
        empty_item: 0xff,
        lineup_order: vec![10, 9],
    };
    let inventory = StateDependency::ComponentField {
        component_id: "save.items".into(),
        field: "inventory".into(),
    };
    let lineup = StateDependency::ComponentField {
        component_id: "save.items".into(),
        field: "item_lineup".into(),
    };

    let mut builder = RelevanceBuilder::default();
    builder.add_operation_inputs(&operation);
    assert_eq!(
        builder.dependencies,
        BTreeSet::from([inventory.clone(), lineup.clone()])
    );
    assert_eq!(operation_outputs(&operation), vec![inventory, lineup]);
}

#[test]
fn pulls_feasibility_alternatives_and_exact_temporal_witnesses_inward() {
    let facts = FactCatalog {
        schema: FACT_CATALOG_SCHEMA.into(),
        aliases: Vec::new(),
        derived_facts: Vec::new(),
    };
    let timing = TemporalWindow {
        earliest_frame: 12,
        latest_frame: 12,
        required_input: Some("sidehop".into()),
    };
    let mut target = transition(
        "transition.target",
        PredicateExpression::True,
        vec![write("final", 1)],
    );
    target.approach_id = "approach.target".into();
    target.activation.physical_obligation_ids = vec!["obligation.timing".into()];
    let mechanics = MechanicsCatalog {
        schema: MECHANICS_CATALOG_SCHEMA.into(),
        transitions: vec![target],
        obligations: vec![FeasibilityObligation {
            id: "obligation.timing".into(),
            label: "Witness the exact interruption".into(),
            scope: scope(),
            obligation_kind: ObligationKind::Timing,
            stage: crate::transition::ObligationStage::Interrupt,
            detail: ObligationDetail::Temporal {
                requirement: TemporalRequirement {
                    action_id: "dialogue.overwrite".into(),
                    window: timing.clone(),
                },
                precondition: equals("armed", 1),
            },
            evidence: evidence(),
        }],
        writers: Vec::new(),
        gates: Vec::new(),
        readers: Vec::new(),
        reconstruction_rules: Vec::new(),
        obstructions: vec![Obstruction {
            id: "obstruction.target".into(),
            label: "Target is physically obstructed".into(),
            scope: scope(),
            blocked_action_id: "transition.target".into(),
            approach_id: "approach.target".into(),
            active_when: equals("blocked", 1),
            obligation_ids: vec!["obligation.timing".into()],
            evidence: evidence(),
        }],
        resolvers: vec![ObstructionResolver {
            id: "resolver.target".into(),
            label: "Bypass the target obstruction".into(),
            scope: scope(),
            obstruction_id: "obstruction.target".into(),
            resolution_kind: ResolutionKind::Bypass,
            applicable_when: equals("resolver_ready", 1),
            operations: Vec::new(),
            evidence: evidence(),
        }],
        techniques: vec![Technique {
            id: "technique.timing".into(),
            label: "Perform the timing setup".into(),
            scope: scope(),
            prerequisites: equals("technique_ready", 1),
            operations: Vec::new(),
            discharged_obligation_ids: vec!["obligation.timing".into()],
            introduced_obligation_ids: Vec::new(),
            cost: RouteCost {
                axes: Default::default(),
            },
            evidence: evidence(),
        }],
        microtraces: vec![WitnessedMicrotrace {
            id: "microtrace.timing".into(),
            scope: scope(),
            precondition: equals("trace_ready", 1),
            operations: vec![StateOperation::Interrupt {
                action_id: "dialogue.overwrite".into(),
                window: timing.clone(),
            }],
            postcondition: PredicateExpression::True,
            timing,
            evidence: evidence(),
        }],
        goals: Vec::new(),
    };

    let relevance = BackwardRelevance::analyze(&facts, &mechanics, &equals("final", 1))
        .expect("catalog should expand");
    assert_eq!(relevance.obligation_ids, vec!["obligation.timing"]);
    assert_eq!(relevance.obstruction_ids, vec!["obstruction.target"]);
    assert_eq!(relevance.resolver_ids, vec!["resolver.target"]);
    assert_eq!(relevance.technique_ids, vec!["technique.timing"]);
    assert_eq!(relevance.microtrace_ids, vec!["microtrace.timing"]);
    for field in [
        "armed",
        "blocked",
        "resolver_ready",
        "technique_ready",
        "trace_ready",
    ] {
        assert!(
            relevance
                .dependencies
                .contains(&StateDependency::ComponentField {
                    component_id: "state.route".into(),
                    field: field.into(),
                })
        );
    }
}

#[test]
fn dynamic_binding_dependencies_overlap_only_compatible_exact_backings() {
    let current_stage = StateDependency::BoundRawBits {
        component_kind: ComponentKind::StageMemory,
        binding: ComponentBindingReference::CurrentStage,
        byte_offset: 8,
        byte_width: 2,
    };
    let exact_stage = StateDependency::BoundRawBits {
        component_kind: ComponentKind::StageMemory,
        binding: ComponentBindingReference::Exact {
            binding: ComponentBinding::Stage {
                stage: "F_SP115".into(),
            },
        },
        byte_offset: 9,
        byte_width: 1,
    };
    let exact_room = StateDependency::BoundRawBits {
        component_kind: ComponentKind::StageMemory,
        binding: ComponentBindingReference::Exact {
            binding: ComponentBinding::Room {
                stage: "F_SP115".into(),
                room: 1,
            },
        },
        byte_offset: 9,
        byte_width: 1,
    };
    assert!(dependencies_overlap(&current_stage, &exact_stage));
    assert!(!dependencies_overlap(&current_stage, &exact_room));

    let active_runtime = StateDependency::BoundComponentField {
        component_kind: ComponentKind::PersistentSave,
        binding: ComponentBindingReference::ActiveRuntimeFile,
        field: "return_place".into(),
    };
    let exact_runtime = StateDependency::BoundComponentField {
        component_kind: ComponentKind::PersistentSave,
        binding: ComponentBindingReference::Exact {
            binding: ComponentBinding::RuntimeFile {
                runtime_file_id: "file-0".into(),
            },
        },
        field: "return_place".into(),
    };
    assert!(dependencies_overlap(&active_runtime, &exact_runtime));

    let projected_zone = StateDependency::BoundRawBits {
        component_kind: ComponentKind::ZoneMemory,
        binding: ComponentBindingReference::Projected {
            component_id: "message-session".into(),
            projection: Box::new(ComponentBindingProjection::Zone {
                stage_field: "speaker_stage".into(),
                zone_field: "speaker_zone".into(),
            }),
        },
        byte_offset: 4,
        byte_width: 1,
    };
    let exact_zone = StateDependency::BoundRawBits {
        component_kind: ComponentKind::ZoneMemory,
        binding: ComponentBindingReference::Exact {
            binding: ComponentBinding::Zone {
                stage: "D_MN01".into(),
                zone: 7,
            },
        },
        byte_offset: 4,
        byte_width: 1,
    };
    assert!(dependencies_overlap(&projected_zone, &exact_zone));
    assert!(!dependencies_overlap(&projected_zone, &exact_room));
}
