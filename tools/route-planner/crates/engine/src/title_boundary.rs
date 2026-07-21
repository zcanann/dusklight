//! Exact GZ2E01 reset-to-opening prefix mechanics.
//!
//! This module deliberately stops at the process handoff. The opening-scene
//! initializer and later title/file-select transitions have broader backing
//! effects and remain separate audit targets.

use crate::PlannerContractError;
use crate::artifact::Digest;
use crate::identity::{ContentIdentity, ContextSelector, ExactContext, RuntimeConfiguration};
use crate::logic::{
    ComparisonOperator, ContextScope, EvidenceKind, EvidenceRecord, PredicateExpression,
    RuleEvidence, TruthStatus, ValueReference,
};
use crate::return_place::{GZ2E01_CONTENT_SHA256, GZ2E01_EN_RUNTIME_SHA256};
use crate::state::{ExecutionContext, SceneLocation, StateValue};
use crate::transition::{
    ActivationContract, CandidateTransition, ComponentFieldTarget, MECHANICS_CATALOG_SCHEMA,
    MechanicsCatalog, StateOperation, TransitionKind,
};

const RESET_CONTROL_COMPONENT: &str = "reset-control";
const RESTART_COMPONENT: &str = "restart";

/// Compiles the exact successful prefix of `dComIfG_resetToOpening` for
/// GZ2E01. It records the scheduled opening process/load and the restart-room
/// parameter write without pretending that the pending F_SP102 request is an
/// already loaded, traversable world location.
pub fn gz2e01_reset_to_opening_mechanics(
    content: &ContentIdentity,
    runtime: &RuntimeConfiguration,
) -> Result<MechanicsCatalog, PlannerContractError> {
    content.validate()?;
    runtime.validate()?;
    let content_sha256 = content.digest()?;
    let runtime_sha256 = runtime.digest()?;
    if content_sha256 != GZ2E01_CONTENT_SHA256
        || runtime_sha256 != GZ2E01_EN_RUNTIME_SHA256
        || runtime.content_sha256 != content_sha256
    {
        return Err(PlannerContractError::new(
            "title_boundary.identity",
            "requires the exact GZ2E01/English context",
        ));
    }

    let scope = ContextScope {
        selectors: vec![ContextSelector::Exact {
            context: ExactContext {
                content_sha256,
                runtime_configuration_sha256: runtime_sha256,
            },
        }],
    };
    let evidence = RuleEvidence {
        truth: TruthStatus::Established,
        records: vec![
            EvidenceRecord {
                id: "binary.gz2e01.dcomifg-reset-to-opening".into(),
                kind: EvidenceKind::Extracted,
                source_sha256: Some(parse_digest(
                    "bde63a102b6502e418e5a8c53cff364f66f6510420a7316a492664ab7530e28d",
                )),
                note: "Canonical exact-DOL function artifact: VA 0x8002cd44, size 0x74, code SHA-256 3cc637771d531950401a332a83b90296df2b5aa9bec6cc292ad5546fec23df30.".into(),
            },
            EvidenceRecord {
                id: "binary.gz2e01.dcomifg-change-opening-scene".into(),
                kind: EvidenceKind::Extracted,
                source_sha256: Some(parse_digest(
                    "658f63b09b0f43dcb5b2662dbbf140de889fe19374dac8ccee32d9545ac2d781",
                )),
                note: "Canonical exact-DOL function artifact: VA 0x8002cc54, size 0xf0, code SHA-256 0b5c465a32ffb343d9863e04970f5c2621a5bb0b854efc974708fb0229828a41.".into(),
            },
            EvidenceRecord {
                id: "source.gcn-reset-to-opening-prefix".into(),
                kind: EvidenceKind::SourceAudited,
                source_sha256: Some(parse_digest(
                    "b9b37aed0b76eef2d27b35a2ece6ee077086a970f98d18936a83649303f15761",
                )),
                note: "Source-family audit establishes the GCN guards, F_SP102/start 100/room 0/layer 10 pending load, PROC_OPENING_SCENE request, and restart-room parameter zero write.".into(),
            },
        ],
    };
    let compare = |left: ValueReference, operator, value| PredicateExpression::Compare {
        left,
        operator,
        right: ValueReference::Literal { value },
    };
    let control_field = |field: &str| ValueReference::ComponentField {
        component_id: RESET_CONTROL_COMPONENT.into(),
        field: field.into(),
    };
    let transition = CandidateTransition {
        id: "transition.gz2e01.reset-to-opening".into(),
        label: "Reset the active play scene to the opening/title process".into(),
        scope,
        transition_kind: TransitionKind::TitleReturn,
        approach_id: "system-reset.gcn".into(),
        activation: ActivationContract {
            hard_guards: PredicateExpression::All {
                terms: vec![
                    compare(
                        control_field("reset_requested"),
                        ComparisonOperator::Equal,
                        StateValue::Boolean(true),
                    ),
                    compare(
                        control_field("return_to_menu"),
                        ComparisonOperator::Equal,
                        StateValue::Boolean(false),
                    ),
                    compare(
                        control_field("fader_status"),
                        ComparisonOperator::NotEqual,
                        StateValue::Unsigned(2),
                    ),
                ],
            },
            physical_obligation_ids: Vec::new(),
            effects: vec![
                StateOperation::Write {
                    target: ComponentFieldTarget {
                        component_id: RESTART_COMPONENT.into(),
                        field: "room_param".into(),
                    },
                    value: StateValue::Unsigned(0),
                },
                StateOperation::SetExecutionContext {
                    context: ExecutionContext::Process {
                        process_name: "PROC_OPENING_SCENE".into(),
                        pending_world_load: Some(SceneLocation {
                            stage: "F_SP102".into(),
                            room: 0,
                            layer: 10,
                            spawn: 100,
                        }),
                    },
                },
            ],
            unknown_requirements: Vec::new(),
        },
        evidence,
    };
    let catalog = MechanicsCatalog {
        schema: MECHANICS_CATALOG_SCHEMA.into(),
        transitions: vec![transition],
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
    catalog.validate()?;
    Ok(catalog)
}

fn parse_digest(value: &str) -> Digest {
    value.parse().expect("compile-time SHA-256 literal")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evaluation::{
        EvaluatedTruth, EvidencePolicy, FeasibilityMode, PredicateEvaluator,
        TransitionClassification,
    };
    use crate::execution::PlannerExecutionState;
    use crate::identity::{
        CONTENT_IDENTITY_SCHEMA, ContentFingerprint, GamePlatform, GameRegion,
        RUNTIME_CONFIGURATION_SCHEMA,
    };
    use crate::logic::{FACT_CATALOG_SCHEMA, FactCatalog};
    use crate::snapshot::{STATE_SNAPSHOT_SCHEMA, StateDiff, StateSnapshot};
    use crate::state::{
        ActorLifecycle, BackingAttachment, ComponentBinding, ComponentBindingReference,
        ComponentKind, ComponentPayload, ComponentProvenance, EXECUTION_ENVIRONMENT_SCHEMA,
        ExecutionEnvironment, LiveWorldObject, PlayerForm, PlayerState, ProvenanceSourceKind,
        RuntimeFile, RuntimeFileLifecycle, RuntimeFileOrigin, SemanticLifetime, SerializationOwner,
        StateComponent,
    };
    use std::collections::{BTreeMap, BTreeSet};

    fn context() -> (ContentIdentity, RuntimeConfiguration) {
        let content = ContentIdentity {
            schema: CONTENT_IDENTITY_SCHEMA.into(),
            id: "gcn-us-1.0-gz2e01".into(),
            fingerprint: ContentFingerprint {
                platform: GamePlatform::GameCube,
                region: GameRegion::Usa,
                revision: "1.0".into(),
                product_id: "GZ2E01".into(),
                executable_sha256: parse_digest(
                    "e7f197436815e66c4a11df3d7bd557d66083b641ff8a8e76439f3caba7ae60e8",
                ),
                game_data_sha256: parse_digest(
                    "0bc3bb229279d4b8a8c7cbe962b0bffdfecd35ff21e2d6761ad42e90a070f772",
                ),
                resource_manifest_sha256: parse_digest(
                    "2ab36f6c1d9d551c1397e1cf59e13288d2684c973cb7bd0ad6878f5a3b3a2ab1",
                ),
            },
        };
        let runtime = RuntimeConfiguration {
            schema: RUNTIME_CONFIGURATION_SCHEMA.into(),
            content_sha256: GZ2E01_CONTENT_SHA256,
            language: "en".into(),
            settings: BTreeMap::new(),
        };
        (content, runtime)
    }

    fn component(
        id: &str,
        kind: ComponentKind,
        fields: impl IntoIterator<Item = (&'static str, StateValue)>,
    ) -> StateComponent {
        StateComponent {
            id: id.into(),
            component_kind: kind,
            payload: ComponentPayload::Structured {
                fields: fields
                    .into_iter()
                    .map(|(field, value)| (field.into(), value))
                    .collect(),
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
                source_id: "trace.reset-control".into(),
                source_sha256: Some(Digest([9; 32])),
                transition_id: None,
            }],
        }
    }

    fn snapshot(runtime: RuntimeConfiguration) -> StateSnapshot {
        StateSnapshot {
            schema: STATE_SNAPSHOT_SCHEMA.into(),
            id: "snapshot.before-reset".into(),
            sequence: 0,
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
                execution_context: ExecutionContext::World,
                location: SceneLocation {
                    stage: "R_SP107".into(),
                    room: 3,
                    layer: 0,
                    spawn: 0,
                },
                player: PlayerState {
                    form: PlayerForm::Wolf,
                    mount: None,
                    position: [0.0; 3],
                    rotation: [0; 3],
                    has_control: Some(true),
                    action: "idle".into(),
                },
                components: vec![
                    component(
                        RESET_CONTROL_COMPONENT,
                        ComponentKind::Session,
                        [
                            ("reset_requested", StateValue::Boolean(true)),
                            ("return_to_menu", StateValue::Boolean(false)),
                            ("fader_status", StateValue::Unsigned(1)),
                        ],
                    ),
                    component(
                        RESTART_COMPONENT,
                        ComponentKind::Restart,
                        [("room_param", StateValue::Unsigned(0xc9))],
                    ),
                ],
                static_world_objects: Vec::new(),
                spatial_volumes: Vec::new(),
                spatial_connections: Vec::new(),
                spatial_planes: Vec::new(),
                persisted_object_controls: Vec::new(),
                live_world_objects: vec![LiveWorldObject {
                    instance_id: "actor.retained-world-probe".into(),
                    static_object_id: None,
                    actor_type: "probe".into(),
                    lifecycle: ActorLifecycle::Loaded,
                    fields: BTreeMap::from([("active".into(), StateValue::Boolean(true))]),
                }],
            },
            semantic_observations: Vec::new(),
        }
    }

    #[test]
    fn reset_prefix_enters_process_without_claiming_pending_map_is_loaded() {
        let (content, runtime) = context();
        let catalog = gz2e01_reset_to_opening_mechanics(&content, &runtime).unwrap();
        let transition = &catalog.transitions[0];
        let facts = FactCatalog {
            schema: FACT_CATALOG_SCHEMA.into(),
            aliases: Vec::new(),
            derived_facts: Vec::new(),
        };
        let before = snapshot(runtime);
        let evaluator = PredicateEvaluator::new(
            &before,
            &facts,
            &[],
            &BTreeMap::new(),
            EvidencePolicy::RESEARCH,
        )
        .unwrap();
        let assessment = evaluator.assess_transition(
            transition,
            &BTreeSet::new(),
            &BTreeSet::new(),
            FeasibilityMode::Modeled,
        );
        assert_eq!(
            assessment.classification,
            TransitionClassification::Executable
        );

        let mut state = PlannerExecutionState::new(before.clone()).unwrap();
        state
            .apply_operations(
                &transition.id,
                "snapshot.after-reset-prefix",
                &transition.activation.effects,
            )
            .unwrap();
        assert_eq!(
            state.snapshot.environment.execution_context,
            ExecutionContext::Process {
                process_name: "PROC_OPENING_SCENE".into(),
                pending_world_load: Some(SceneLocation {
                    stage: "F_SP102".into(),
                    room: 0,
                    layer: 10,
                    spawn: 100,
                }),
            }
        );
        assert_eq!(state.snapshot.environment.location.stage, "R_SP107");
        assert_eq!(
            ComponentBindingReference::CurrentStage.resolve(&state.snapshot.environment),
            None
        );
        let diff = StateDiff::between(
            &before,
            &state.snapshot,
            crate::state::BoundaryKind::TitleReturn,
        )
        .unwrap();
        assert!(diff.execution_context_changed);
        assert_eq!(diff.execution_context_before, ExecutionContext::World);
        assert_eq!(
            diff.execution_context_after,
            state.snapshot.environment.execution_context
        );
        assert!(!diff.location_changed);

        let evaluator = PredicateEvaluator::new(
            &state.snapshot,
            &facts,
            &[],
            &BTreeMap::new(),
            EvidencePolicy::RESEARCH,
        )
        .unwrap();
        assert_eq!(
            evaluator.evaluate(&PredicateExpression::Compare {
                left: ValueReference::LocationStage,
                operator: ComparisonOperator::Equal,
                right: ValueReference::Literal {
                    value: StateValue::Text("R_SP107".into()),
                },
            }),
            EvaluatedTruth::Unknown
        );
        assert_eq!(
            evaluator.resolve_value(&ValueReference::ExecutionProcess),
            Some(StateValue::Text("PROC_OPENING_SCENE".into()))
        );
        assert_eq!(
            evaluator.resolve_value(&ValueReference::ActorField {
                instance_id: "actor.retained-world-probe".into(),
                field: "active".into(),
            }),
            None
        );
        state
            .apply_operations(
                "transition.complete-world-load",
                "snapshot.after-world-load",
                &[StateOperation::SetLocation {
                    location: SceneLocation {
                        stage: "F_SP102".into(),
                        room: 0,
                        layer: 10,
                        spawn: 100,
                    },
                }],
            )
            .unwrap();
        assert_eq!(
            state.snapshot.environment.execution_context,
            ExecutionContext::World
        );
        assert_eq!(state.snapshot.environment.location.stage, "F_SP102");
    }

    #[test]
    fn reset_prefix_is_guard_blocked_when_fader_is_busy() {
        let (content, runtime) = context();
        let catalog = gz2e01_reset_to_opening_mechanics(&content, &runtime).unwrap();
        let mut before = snapshot(runtime);
        let ComponentPayload::Structured { fields } = &mut before.environment.components[0].payload
        else {
            unreachable!()
        };
        fields.insert("fader_status".into(), StateValue::Unsigned(2));
        let facts = FactCatalog {
            schema: FACT_CATALOG_SCHEMA.into(),
            aliases: Vec::new(),
            derived_facts: Vec::new(),
        };
        let evaluator = PredicateEvaluator::new(
            &before,
            &facts,
            &[],
            &BTreeMap::new(),
            EvidencePolicy::RESEARCH,
        )
        .unwrap();
        assert_eq!(
            evaluator
                .assess_transition(
                    &catalog.transitions[0],
                    &BTreeSet::new(),
                    &BTreeSet::new(),
                    FeasibilityMode::Modeled,
                )
                .classification,
            TransitionClassification::GuardBlocked
        );
    }
}
