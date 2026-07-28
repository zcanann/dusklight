//! Build scheduler-observed title and name-scene activation transitions.

use super::*;

fn compare(
    left: ValueReference,
    operator: ComparisonOperator,
    value: StateValue,
) -> PredicateExpression {
    PredicateExpression::Compare {
        left,
        operator,
        right: ValueReference::Literal { value },
    }
}

fn pending_compare(left: ValueReference, value: StateValue) -> PredicateExpression {
    compare(left, ComparisonOperator::Equal, value)
}

fn title_field(field: &str) -> ValueReference {
    ValueReference::ComponentField {
        component_id: TITLE_CONTROL_COMPONENT.into(),
        field: field.into(),
    }
}

fn process_component(
    id: &str,
    component_kind: ComponentKind,
    fields: BTreeMap<String, StateValue>,
) -> StateComponent {
    StateComponent {
        id: id.into(),
        component_kind,
        payload: ComponentPayload::Structured { fields },
        binding: ComponentBinding::Session {
            session_id: "process".into(),
        },
        lifetime: SemanticLifetime::Session,
        serialization_owner: SerializationOwner::None,
        provenance: vec![ComponentProvenance {
            source_kind: ProvenanceSourceKind::TraceObservation,
            source_id: "observation.gz2e01-process-activation".into(),
            source_sha256: Some(parse_digest(
                "f095894aabc198c068ee0ac9872f6c277c0e035b36c4d29d1f896e7c2eb0fe4b",
            )),
            transition_id: None,
        }],
    }
}

pub(super) fn opening_process_activation_transition(
    scope: ContextScope,
    scheduler_evidence: RuleEvidence,
) -> CandidateTransition {
    CandidateTransition {
        id: "transition.gz2e01.observe-opening-phase-4".into(),
        label: "Observe opening process activation at phase 4".into(),
        scope,
        transition_kind: TransitionKind::Other,
        approach_id: "scheduler.observe-opening-phase-4".into(),
        activation: ActivationContract {
            hard_guards: PredicateExpression::All {
                terms: vec![
                    compare(
                        ValueReference::ExecutionProcess,
                        ComparisonOperator::Equal,
                        StateValue::Text("PROC_OPENING_SCENE".into()),
                    ),
                    compare(
                        ValueReference::PendingWorldLoadStage,
                        ComparisonOperator::Equal,
                        StateValue::Text("F_SP102".into()),
                    ),
                    compare(
                        ValueReference::ComponentField {
                            component_id: RESET_CONTROL_COMPONENT.into(),
                            field: "opening_process_observed".into(),
                        },
                        ComparisonOperator::Equal,
                        StateValue::Boolean(false),
                    ),
                ],
            },
            physical_obligation_ids: Vec::new(),
            effects: vec![
                StateOperation::Initialize {
                    component: process_component(
                        OPENING_PROCESS_CONTROL_COMPONENT,
                        ComponentKind::Session,
                        BTreeMap::from([("phase".into(), StateValue::Text("phase_4".into()))]),
                    ),
                },
                StateOperation::Initialize {
                    component: process_component(
                        TITLE_CONTROL_COMPONENT,
                        ComponentKind::Title,
                        BTreeMap::from([
                            ("phase".into(), StateValue::Text("key_wait".into())),
                            ("reset_requested".into(), StateValue::Boolean(false)),
                            ("overlap_peek".into(), StateValue::Boolean(false)),
                            ("a_triggered".into(), StateValue::Boolean(true)),
                            ("start_triggered".into(), StateValue::Boolean(false)),
                        ]),
                    ),
                },
                StateOperation::Write {
                    target: ComponentFieldTarget {
                        component_id: RESET_CONTROL_COMPONENT.into(),
                        field: "opening_process_observed".into(),
                    },
                    value: StateValue::Boolean(true),
                },
            ],
            unknown_requirements: Vec::new(),
        },
        evidence: scheduler_evidence.clone(),
    }
}

pub(super) fn name_scene_activation_transition(
    scope: ContextScope,
    scheduler_evidence: RuleEvidence,
) -> CandidateTransition {
    CandidateTransition {
        id: "transition.gz2e01.observe-name-scene-create".into(),
        label: "Observe name scene activation at file-select creation".into(),
        scope,
        transition_kind: TransitionKind::Other,
        approach_id: "scheduler.observe-name-scene-create".into(),
        activation: ActivationContract {
            hard_guards: PredicateExpression::All {
                terms: vec![
                    pending_compare(
                        ValueReference::ExecutionProcess,
                        StateValue::Text("PROC_OPENING_SCENE".into()),
                    ),
                    pending_compare(
                        title_field("phase"),
                        StateValue::Text("scene_requested".into()),
                    ),
                    pending_compare(
                        ValueReference::ComponentField {
                            component_id: RESET_CONTROL_COMPONENT.into(),
                            field: "name_scene_observed".into(),
                        },
                        StateValue::Boolean(false),
                    ),
                ],
            },
            physical_obligation_ids: Vec::new(),
            effects: vec![
                StateOperation::Initialize {
                    component: process_component(
                        NAME_SCENE_CONTROL_COMPONENT,
                        ComponentKind::Title,
                        BTreeMap::from([(
                            "phase".into(),
                            StateValue::Text("create_file_select".into()),
                        )]),
                    ),
                },
                StateOperation::SetExecutionContext {
                    context: ExecutionContext::Process {
                        process_name: "PROC_NAME_SCENE".into(),
                        pending_world_load: None,
                    },
                },
                StateOperation::Write {
                    target: ComponentFieldTarget {
                        component_id: RESET_CONTROL_COMPONENT.into(),
                        field: "name_scene_observed".into(),
                    },
                    value: StateValue::Boolean(true),
                },
            ],
            unknown_requirements: Vec::new(),
        },
        evidence: scheduler_evidence,
    }
}
