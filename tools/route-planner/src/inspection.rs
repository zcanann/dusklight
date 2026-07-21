//! Read-only state projection for inventory, flags, bindings, and backing stores.

use crate::RuntimeEvidenceMode;
use dusklight_route_planner::PlannerContractError;
use dusklight_route_planner::artifact::Digest;
use dusklight_route_planner::evaluation::{EvaluatedTruth, EvidencePolicy, PredicateEvaluator};
use dusklight_route_planner::execution::{PlannerExecutionState, PlannerExecutionStateDocument};
use dusklight_route_planner::identity::EquivalenceSet;
use dusklight_route_planner::logic::{
    FactCatalog, PredicateExpression, RawFactBinding, TruthStatus,
};
use serde::Serialize;

pub const STATE_INSPECTION_SCHEMA: &str = "dusklight.route-planner.state-inspection/v2";

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StateInspection {
    pub schema: String,
    pub execution_state_sha256: Digest,
    pub semantic_state_sha256: Digest,
    pub fact_catalog_sha256: Digest,
    pub evidence_mode: RuntimeEvidenceMode,
    pub state: PlannerExecutionStateDocument,
    pub facts: Vec<InspectedFact>,
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
    Ok(StateInspection {
        schema: STATE_INSPECTION_SCHEMA.into(),
        execution_state_sha256: state.digest()?,
        semantic_state_sha256: state.semantic_digest()?,
        fact_catalog_sha256: facts.digest()?,
        evidence_mode,
        state: state.to_document()?,
        facts: inspected,
    })
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
        SemanticLifetime, SerializationOwner, StateComponent,
    };
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
                components: vec![StateComponent {
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
                }],
                static_world_objects: Vec::new(),
                spatial_volumes: Vec::new(),
                persisted_object_controls: Vec::new(),
                live_world_objects: Vec::new(),
            },
            semantic_observations: Vec::new(),
        };
        let state = PlannerExecutionState::new(snapshot).unwrap();
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
        assert_eq!(inspection.state.snapshot.environment.components.len(), 1);
        assert_eq!(inspection.state.serialized_component_stores.len(), 0);
    }
}
