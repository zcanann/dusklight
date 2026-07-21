//! Read-only state projection for inventory, flags, bindings, and backing stores.

use crate::RuntimeEvidenceMode;
use dusklight_route_planner::PlannerContractError;
use dusklight_route_planner::artifact::Digest;
use dusklight_route_planner::evaluation::{EvaluatedTruth, EvidencePolicy, PredicateEvaluator};
use dusklight_route_planner::execution::{
    ExecutionHistoryEvent, ExecutionHistoryKind, PlannerExecutionState,
    PlannerExecutionStateDocument,
};
use dusklight_route_planner::identity::EquivalenceSet;
use dusklight_route_planner::logic::{
    FactCatalog, PredicateExpression, RawFactBinding, TruthStatus, ValueReference,
};
use dusklight_route_planner::snapshot::StateDiff;
use dusklight_route_planner::state::{BoundaryKind, ComponentPayload};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

pub const STATE_INSPECTION_SCHEMA: &str = "dusklight.route-planner.state-inspection/v7";
pub const STATE_INSPECTION_DIFF_SCHEMA: &str = "dusklight.route-planner.state-inspection-diff/v5";

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StateInspection {
    pub schema: String,
    pub execution_state_sha256: Digest,
    pub semantic_state_sha256: Digest,
    pub fact_catalog_sha256: Digest,
    pub evidence_mode: RuntimeEvidenceMode,
    pub state: PlannerExecutionStateDocument,
    pub last_field_writers: Vec<InspectedLastFieldWriter>,
    pub gate_histories: Vec<InspectedGateHistory>,
    pub facts: Vec<InspectedFact>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InspectedLastFieldWriter {
    pub component_id: String,
    pub field: String,
    pub event: Option<ExecutionHistoryEvent>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InspectedGateHistory {
    pub gate_id: String,
    pub current_state: Option<bool>,
    pub events: Vec<ExecutionHistoryEvent>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InspectedFact {
    pub id: String,
    pub label: String,
    pub source_kind: InspectedFactKind,
    pub authored_truth: TruthStatus,
    pub scope_applies: bool,
    pub evidence_permitted: bool,
    pub evaluated: InspectionTruth,
    pub raw_binding: Option<RawFactBinding>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InspectedFactKind {
    Alias,
    Derived,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InspectionTruth {
    True,
    False,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StateInspectionDiff {
    pub schema: String,
    pub before_execution_state_sha256: Digest,
    pub after_execution_state_sha256: Digest,
    pub fact_catalog_sha256: Digest,
    pub evidence_mode: RuntimeEvidenceMode,
    pub state_diff: StateDiff,
    pub gate_state_deltas: Vec<GateStateDelta>,
    pub execution_history_common_prefix_len: usize,
    pub before_execution_history_suffix: Vec<ExecutionHistoryEvent>,
    pub after_execution_history_suffix: Vec<ExecutionHistoryEvent>,
    pub fact_deltas: Vec<InspectedFactDelta>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GateStateDelta {
    pub gate_id: String,
    pub before: Option<bool>,
    pub after: Option<bool>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InspectedFactDelta {
    pub fact_id: String,
    pub before: InspectedFact,
    pub after: InspectedFact,
    pub causes: Vec<FactDeltaCause>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FactDeltaCause {
    ComponentBindingChanged { component_ids: Vec<String> },
    ComponentPayloadChanged { component_ids: Vec<String> },
    DependencyChanged { fact_ids: Vec<String> },
    RuntimeContextChanged,
    GateStateChanged,
    Unclassified,
}

pub fn inspect_state(
    state: &PlannerExecutionState,
    facts: &FactCatalog,
    equivalence_sets: &[EquivalenceSet],
    evidence_mode: RuntimeEvidenceMode,
) -> Result<StateInspection, PlannerContractError> {
    state.validate()?;
    facts.validate()?;
    let policy = match evidence_mode {
        RuntimeEvidenceMode::EstablishedOnly => EvidencePolicy::ESTABLISHED_ONLY,
        RuntimeEvidenceMode::Research => EvidencePolicy::RESEARCH,
    };
    let evaluator = PredicateEvaluator::new(
        &state.snapshot,
        facts,
        equivalence_sets,
        &state.gate_states,
        policy,
    )?;
    let mut inspected = Vec::with_capacity(facts.aliases.len() + facts.derived_facts.len());
    for alias in &facts.aliases {
        inspected.push(InspectedFact {
            id: alias.id.clone(),
            label: alias.label.clone(),
            source_kind: InspectedFactKind::Alias,
            authored_truth: alias.evidence.truth,
            scope_applies: evaluator.scope_applies(&alias.scope),
            evidence_permitted: policy.permits(alias.evidence.truth),
            evaluated: inspect_truth(evaluator.evaluate(&PredicateExpression::Fact {
                fact_id: alias.id.clone(),
            })),
            raw_binding: Some(alias.raw.clone()),
        });
    }
    for fact in &facts.derived_facts {
        inspected.push(InspectedFact {
            id: fact.id.clone(),
            label: fact.label.clone(),
            source_kind: InspectedFactKind::Derived,
            authored_truth: fact.evidence.truth,
            scope_applies: evaluator.scope_applies(&fact.scope),
            evidence_permitted: policy.permits(fact.evidence.truth),
            evaluated: inspect_truth(evaluator.evaluate(&PredicateExpression::Fact {
                fact_id: fact.id.clone(),
            })),
            raw_binding: None,
        });
    }
    inspected.sort_by(|left, right| left.id.cmp(&right.id));
    let mut last_field_writers = Vec::new();
    for component in &state.snapshot.environment.components {
        let ComponentPayload::Structured { fields } = &component.payload else {
            continue;
        };
        for field in fields.keys() {
            last_field_writers.push(InspectedLastFieldWriter {
                component_id: component.id.clone(),
                field: field.clone(),
                event: state.last_field_writer(&component.id, field).cloned(),
            });
        }
    }
    last_field_writers.sort_by(|left, right| {
        (&left.component_id, &left.field).cmp(&(&right.component_id, &right.field))
    });
    let mut gate_ids = state.gate_states.keys().cloned().collect::<BTreeSet<_>>();
    for event in &state.execution_history {
        if let ExecutionHistoryKind::Operation {
            operation:
                dusklight_route_planner::transition::StateOperation::SetGate { gate_id }
                | dusklight_route_planner::transition::StateOperation::ClearGate { gate_id },
            ..
        } = &event.event
        {
            gate_ids.insert(gate_id.clone());
        }
    }
    let gate_histories = gate_ids
        .into_iter()
        .map(|gate_id| InspectedGateHistory {
            current_state: state.gate_states.get(&gate_id).copied(),
            events: state.gate_history(&gate_id).into_iter().cloned().collect(),
            gate_id,
        })
        .collect();
    Ok(StateInspection {
        schema: STATE_INSPECTION_SCHEMA.into(),
        execution_state_sha256: state.digest()?,
        semantic_state_sha256: state.semantic_digest()?,
        fact_catalog_sha256: facts.digest()?,
        evidence_mode,
        state: state.to_document()?,
        last_field_writers,
        gate_histories,
        facts: inspected,
    })
}

pub fn inspect_state_diff(
    before: &PlannerExecutionState,
    after: &PlannerExecutionState,
    boundary: BoundaryKind,
    facts: &FactCatalog,
    equivalence_sets: &[EquivalenceSet],
    evidence_mode: RuntimeEvidenceMode,
) -> Result<StateInspectionDiff, PlannerContractError> {
    let before_inspection = inspect_state(before, facts, equivalence_sets, evidence_mode)?;
    let after_inspection = inspect_state(after, facts, equivalence_sets, evidence_mode)?;
    let state_diff = StateDiff::between(&before.snapshot, &after.snapshot, boundary)?;
    let before_facts = before_inspection
        .facts
        .iter()
        .map(|fact| (fact.id.as_str(), fact))
        .collect::<BTreeMap<_, _>>();
    let after_facts = after_inspection
        .facts
        .iter()
        .map(|fact| (fact.id.as_str(), fact))
        .collect::<BTreeMap<_, _>>();
    let changed_fact_ids = before_facts
        .keys()
        .filter(|id| {
            after_facts
                .get(**id)
                .is_some_and(|after| before_facts[*id].evaluated != after.evaluated)
        })
        .copied()
        .collect::<BTreeSet<_>>();
    let runtime_context_changed = before.snapshot.environment.runtime_configuration
        != after.snapshot.environment.runtime_configuration;
    let gates_changed = before.gate_states != after.gate_states;
    let execution_history_common_prefix_len = before
        .execution_history
        .iter()
        .zip(&after.execution_history)
        .take_while(|(left, right)| left == right)
        .count();
    let mut fact_deltas = Vec::with_capacity(changed_fact_ids.len());
    for fact_id in changed_fact_ids {
        let before_fact = before_facts[fact_id];
        let after_fact = after_facts[fact_id];
        let mut causes = match before_fact.source_kind {
            InspectedFactKind::Alias => {
                alias_delta_causes(before_fact.raw_binding.as_ref(), &state_diff)
            }
            InspectedFactKind::Derived => {
                let derived = facts.derived_facts.iter().find(|fact| fact.id == fact_id);
                let dependencies = derived
                    .map(|fact| referenced_fact_ids(&fact.rule))
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|dependency| {
                        before_facts.get(dependency.as_str()).is_some_and(|left| {
                            after_facts
                                .get(dependency.as_str())
                                .is_some_and(|right| left.evaluated != right.evaluated)
                        })
                    })
                    .collect::<Vec<_>>();
                let mut causes = Vec::new();
                if !dependencies.is_empty() {
                    causes.push(FactDeltaCause::DependencyChanged {
                        fact_ids: dependencies,
                    });
                }
                if gates_changed && derived.is_some_and(|fact| references_gate_state(&fact.rule)) {
                    causes.push(FactDeltaCause::GateStateChanged);
                }
                if runtime_context_changed
                    && derived.is_some_and(|fact| references_runtime_context(&fact.rule))
                {
                    causes.push(FactDeltaCause::RuntimeContextChanged);
                }
                causes
            }
        };
        if before_fact.scope_applies != after_fact.scope_applies
            && !causes.contains(&FactDeltaCause::RuntimeContextChanged)
        {
            causes.push(FactDeltaCause::RuntimeContextChanged);
        }
        causes.sort();
        causes.dedup();
        if causes.is_empty() {
            causes.push(FactDeltaCause::Unclassified);
        }
        fact_deltas.push(InspectedFactDelta {
            fact_id: fact_id.into(),
            before: before_fact.clone(),
            after: after_fact.clone(),
            causes,
        });
    }
    Ok(StateInspectionDiff {
        schema: STATE_INSPECTION_DIFF_SCHEMA.into(),
        before_execution_state_sha256: before.semantic_digest()?,
        after_execution_state_sha256: after.semantic_digest()?,
        fact_catalog_sha256: facts.digest()?,
        evidence_mode,
        state_diff,
        gate_state_deltas: diff_gate_states(&before.gate_states, &after.gate_states),
        execution_history_common_prefix_len,
        before_execution_history_suffix: before.execution_history
            [execution_history_common_prefix_len..]
            .to_vec(),
        after_execution_history_suffix: after.execution_history
            [execution_history_common_prefix_len..]
            .to_vec(),
        fact_deltas,
    })
}

fn diff_gate_states(
    before: &BTreeMap<String, bool>,
    after: &BTreeMap<String, bool>,
) -> Vec<GateStateDelta> {
    before
        .keys()
        .chain(after.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter_map(|gate_id| {
            let before = before.get(gate_id).copied();
            let after = after.get(gate_id).copied();
            (before != after).then(|| GateStateDelta {
                gate_id: gate_id.clone(),
                before,
                after,
            })
        })
        .collect()
}

fn alias_delta_causes(
    binding: Option<&RawFactBinding>,
    state_diff: &StateDiff,
) -> Vec<FactDeltaCause> {
    let Some(binding) = binding else {
        return Vec::new();
    };
    let mut binding_components = Vec::new();
    let mut payload_components = Vec::new();
    for delta in &state_diff.component_deltas {
        let matching_kind = delta.component_kind_before.as_ref() == Some(&binding.component_kind)
            || delta.component_kind_after.as_ref() == Some(&binding.component_kind);
        if !matching_kind {
            continue;
        }
        let matching_binding = delta.binding_before.as_ref() == Some(&binding.binding)
            || delta.binding_after.as_ref() == Some(&binding.binding);
        if delta.binding_before != delta.binding_after && matching_binding {
            binding_components.push(delta.component_id.clone());
        }
        if delta.payload_sha256_before != delta.payload_sha256_after && matching_binding {
            payload_components.push(delta.component_id.clone());
        }
    }
    let mut causes = Vec::new();
    if !binding_components.is_empty() {
        causes.push(FactDeltaCause::ComponentBindingChanged {
            component_ids: binding_components,
        });
    }
    if !payload_components.is_empty() {
        causes.push(FactDeltaCause::ComponentPayloadChanged {
            component_ids: payload_components,
        });
    }
    causes
}

fn referenced_fact_ids(expression: &PredicateExpression) -> Vec<String> {
    let mut ids = BTreeSet::new();
    collect_referenced_fact_ids(expression, &mut ids);
    ids.into_iter().collect()
}

fn collect_referenced_fact_ids(expression: &PredicateExpression, ids: &mut BTreeSet<String>) {
    match expression {
        PredicateExpression::Fact { fact_id } => {
            ids.insert(fact_id.clone());
        }
        PredicateExpression::All { terms } | PredicateExpression::Any { terms } => {
            for term in terms {
                collect_referenced_fact_ids(term, ids);
            }
        }
        PredicateExpression::Not { term } => collect_referenced_fact_ids(term, ids),
        PredicateExpression::True
        | PredicateExpression::False
        | PredicateExpression::Compare { .. } => {}
    }
}

fn references_gate_state(expression: &PredicateExpression) -> bool {
    predicate_references_value(expression, |value| {
        matches!(value, ValueReference::GateState { .. })
    })
}

fn references_runtime_context(expression: &PredicateExpression) -> bool {
    predicate_references_value(expression, |value| {
        matches!(
            value,
            ValueReference::RuntimeLanguage | ValueReference::RuntimeSetting { .. }
        )
    })
}

fn predicate_references_value(
    expression: &PredicateExpression,
    predicate: impl Copy + Fn(&ValueReference) -> bool,
) -> bool {
    match expression {
        PredicateExpression::Compare { left, right, .. } => predicate(left) || predicate(right),
        PredicateExpression::All { terms } | PredicateExpression::Any { terms } => terms
            .iter()
            .any(|term| predicate_references_value(term, predicate)),
        PredicateExpression::Not { term } => predicate_references_value(term, predicate),
        PredicateExpression::True
        | PredicateExpression::False
        | PredicateExpression::Fact { .. } => false,
    }
}

fn inspect_truth(value: EvaluatedTruth) -> InspectionTruth {
    match value {
        EvaluatedTruth::True => InspectionTruth::True,
        EvaluatedTruth::False => InspectionTruth::False,
        EvaluatedTruth::Unknown => InspectionTruth::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dusklight_route_planner::identity::{RUNTIME_CONFIGURATION_SCHEMA, RuntimeConfiguration};
    use dusklight_route_planner::logic::{
        ContextScope, EvidenceKind, EvidenceRecord, FACT_CATALOG_SCHEMA, FriendlyAlias,
        RuleEvidence,
    };
    use dusklight_route_planner::snapshot::{STATE_SNAPSHOT_SCHEMA, StateSnapshot};
    use dusklight_route_planner::state::{
        BackingAttachment, ComponentBinding, ComponentKind, ComponentPayload, ComponentProvenance,
        EXECUTION_ENVIRONMENT_SCHEMA, ExecutionEnvironment, PlayerForm, PlayerState,
        ProvenanceSourceKind, RuntimeFile, RuntimeFileLifecycle, RuntimeFileOrigin, SceneLocation,
        SemanticLifetime, SerializationOwner, StateComponent, StateValue,
    };
    use dusklight_route_planner::transition::{ComponentFieldTarget, StateOperation};
    use std::collections::BTreeMap;

    #[test]
    fn inspection_keeps_raw_inventory_and_friendly_fact_together() {
        let content = Digest([1; 32]);
        let runtime = RuntimeConfiguration {
            schema: RUNTIME_CONFIGURATION_SCHEMA.into(),
            content_sha256: content,
            language: "en".into(),
            settings: BTreeMap::new(),
        };
        let exact = runtime.exact_context().unwrap();
        let snapshot = StateSnapshot {
            schema: STATE_SNAPSHOT_SCHEMA.into(),
            id: "snapshot.inspect".into(),
            sequence: 1,
            environment: ExecutionEnvironment {
                schema: EXECUTION_ENVIRONMENT_SCHEMA.into(),
                runtime_configuration: runtime,
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
                    rotation: [0; 3],
                    has_control: Some(true),
                    action: "idle".into(),
                },
                components: vec![
                    StateComponent {
                        id: "inventory.active".into(),
                        component_kind: ComponentKind::Inventory,
                        payload: ComponentPayload::Raw {
                            bytes: vec![0b0000_0100],
                            known_mask: vec![0xff],
                        },
                        binding: ComponentBinding::RuntimeFile {
                            runtime_file_id: "file-0".into(),
                        },
                        lifetime: SemanticLifetime::RuntimeFile,
                        serialization_owner: SerializationOwner::RuntimeFile {
                            runtime_file_id: "file-0".into(),
                        },
                        provenance: vec![ComponentProvenance {
                            source_kind: ProvenanceSourceKind::TraceObservation,
                            source_id: "trace.inventory".into(),
                            source_sha256: Some(Digest([3; 32])),
                            transition_id: None,
                        }],
                    },
                    StateComponent {
                        id: "save.return".into(),
                        component_kind: ComponentKind::Restart,
                        payload: ComponentPayload::Structured {
                            fields: BTreeMap::from([(
                                "player_return_place".into(),
                                StateValue::Text("unknown".into()),
                            )]),
                        },
                        binding: ComponentBinding::RuntimeFile {
                            runtime_file_id: "file-0".into(),
                        },
                        lifetime: SemanticLifetime::RuntimeFile,
                        serialization_owner: SerializationOwner::RuntimeFile {
                            runtime_file_id: "file-0".into(),
                        },
                        provenance: vec![ComponentProvenance {
                            source_kind: ProvenanceSourceKind::TraceObservation,
                            source_id: "trace.return-place".into(),
                            source_sha256: Some(Digest([4; 32])),
                            transition_id: None,
                        }],
                    },
                ],
                static_world_objects: Vec::new(),
                spatial_volumes: Vec::new(),
                spatial_connections: Vec::new(),
                spatial_planes: Vec::new(),
                persisted_object_controls: Vec::new(),
                live_world_objects: Vec::new(),
            },
            semantic_observations: Vec::new(),
        };
        let mut state = PlannerExecutionState::new(snapshot).unwrap();
        state
            .apply_operations(
                "writer.return-place.ordon",
                "snapshot.inspect-written",
                &[
                    StateOperation::Write {
                        target: ComponentFieldTarget {
                            component_id: "save.return".into(),
                            field: "player_return_place".into(),
                        },
                        value: StateValue::Text("F_SP103:0:0:0".into()),
                    },
                    StateOperation::SetGate {
                        gate_id: "gate.no-telop".into(),
                    },
                ],
            )
            .unwrap();
        let facts = FactCatalog {
            schema: FACT_CATALOG_SCHEMA.into(),
            aliases: vec![FriendlyAlias {
                id: "inventory.fishing-rod".into(),
                label: "Fishing Rod".into(),
                scope: ContextScope {
                    selectors: vec![dusklight_route_planner::identity::ContextSelector::Exact {
                        context: exact,
                    }],
                },
                raw: RawFactBinding {
                    component_kind: ComponentKind::Inventory,
                    binding: ComponentBinding::RuntimeFile {
                        runtime_file_id: "file-0".into(),
                    },
                    byte_offset: 0,
                    mask: vec![0x04],
                    expected: vec![0x04],
                },
                evidence: RuleEvidence {
                    truth: TruthStatus::Established,
                    records: vec![EvidenceRecord {
                        id: "source.inventory".into(),
                        kind: EvidenceKind::SourceAudited,
                        source_sha256: Some(Digest([2; 32])),
                        note: "Fishing rod inventory bit.".into(),
                    }],
                },
            }],
            derived_facts: Vec::new(),
        };
        let inspection =
            inspect_state(&state, &facts, &[], RuntimeEvidenceMode::EstablishedOnly).unwrap();
        assert_eq!(inspection.facts[0].evaluated, InspectionTruth::True);
        assert_eq!(inspection.state.snapshot.environment.components.len(), 2);
        assert_eq!(inspection.state.serialized_component_stores.len(), 0);
        assert_eq!(inspection.last_field_writers.len(), 1);
        assert_eq!(
            inspection.last_field_writers[0]
                .event
                .as_ref()
                .unwrap()
                .application_id,
            "writer.return-place.ordon"
        );
        assert_eq!(inspection.gate_histories.len(), 1);
        assert_eq!(inspection.gate_histories[0].events.len(), 1);

        let before = state;
        let mut after = before.clone();
        after.snapshot.id = "snapshot.inspect-rebound".into();
        after.snapshot.sequence = before.snapshot.sequence + 1;
        after.snapshot.environment.components[0].binding = ComponentBinding::Stage {
            stage: "D_MN09".into(),
        };
        after.validate().unwrap();
        let diff = inspect_state_diff(
            &before,
            &after,
            BoundaryKind::WrongStateRespawn,
            &facts,
            &[],
            RuntimeEvidenceMode::EstablishedOnly,
        )
        .unwrap();
        assert_eq!(diff.fact_deltas.len(), 1);
        assert_ne!(
            diff.before_execution_state_sha256,
            diff.after_execution_state_sha256
        );
        assert!(diff.gate_state_deltas.is_empty());
        assert_eq!(diff.execution_history_common_prefix_len, 2);
        assert!(diff.before_execution_history_suffix.is_empty());
        assert!(diff.after_execution_history_suffix.is_empty());
        assert_eq!(diff.fact_deltas[0].before.evaluated, InspectionTruth::True);
        assert_eq!(
            diff.fact_deltas[0].after.evaluated,
            InspectionTruth::Unknown
        );
        assert_eq!(
            diff.fact_deltas[0].causes,
            vec![FactDeltaCause::ComponentBindingChanged {
                component_ids: vec!["inventory.active".into()]
            }]
        );
        assert_eq!(
            diff.state_diff.component_deltas[0].payload_sha256_before,
            diff.state_diff.component_deltas[0].payload_sha256_after
        );
        assert!(
            diff.state_diff.component_deltas[0]
                .raw_byte_deltas
                .is_empty()
        );

        let mut progressed = before.clone();
        progressed
            .apply_operations(
                "gate.no-telop.clear",
                "snapshot.inspect-gate-cleared",
                &[StateOperation::ClearGate {
                    gate_id: "gate.no-telop".into(),
                }],
            )
            .unwrap();
        let progressed_diff = inspect_state_diff(
            &before,
            &progressed,
            BoundaryKind::DialogueInterruption,
            &facts,
            &[],
            RuntimeEvidenceMode::EstablishedOnly,
        )
        .unwrap();
        assert_eq!(progressed_diff.execution_history_common_prefix_len, 2);
        assert!(progressed_diff.before_execution_history_suffix.is_empty());
        assert_eq!(progressed_diff.after_execution_history_suffix.len(), 1);
        assert_eq!(progressed_diff.gate_state_deltas.len(), 1);

        assert_eq!(
            diff_gate_states(
                &BTreeMap::from([("gate.a".into(), false)]),
                &BTreeMap::from([("gate.a".into(), true), ("gate.b".into(), true)]),
            ),
            vec![
                GateStateDelta {
                    gate_id: "gate.a".into(),
                    before: Some(false),
                    after: Some(true),
                },
                GateStateDelta {
                    gate_id: "gate.b".into(),
                    before: None,
                    after: Some(true),
                },
            ]
        );
    }
}
