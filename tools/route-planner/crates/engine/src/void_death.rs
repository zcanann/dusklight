//! Exact-context GZ2E01 void-selection and death-diversion mechanics.

use crate::PlannerContractError;
use crate::artifact::Digest;
use crate::identity::{ContentIdentity, ContextSelector, ExactContext, RuntimeConfiguration};
use crate::logic::{
    ComparisonOperator, ContextScope, EvidenceKind, EvidenceRecord, PredicateExpression,
    RuleEvidence, TruthStatus, ValueReference,
};
use crate::return_place::{GZ2E01_CONTENT_SHA256, GZ2E01_EN_RUNTIME_SHA256};
use crate::state::{ExecutionContext, StateValue};
use crate::transition::{
    ActivationContract, CandidateTransition, ComponentFieldTarget, FeasibilityObligation,
    MECHANICS_CATALOG_SCHEMA, MechanicsCatalog, ObligationDetail, ObligationKind, ObligationStage,
    StateOperation, TransitionKind, UnknownRequirement,
};

pub const VOID_CONTROL_COMPONENT_ID: &str = "control.void-selection";
pub const DEATH_FLOW_COMPONENT_ID: &str = "control.death-flow";
pub const RESET_CONTROL_COMPONENT_ID: &str = "reset-control";

pub fn gz2e01_void_death_mechanics(
    content: &ContentIdentity,
    runtime: &RuntimeConfiguration,
) -> Result<MechanicsCatalog, PlannerContractError> {
    content.validate()?;
    runtime.validate()?;
    let content_sha256 = content.digest()?;
    let runtime_configuration_sha256 = runtime.digest()?;
    if content_sha256 != GZ2E01_CONTENT_SHA256
        || runtime_configuration_sha256 != GZ2E01_EN_RUNTIME_SHA256
        || runtime.content_sha256 != content_sha256
    {
        return Err(PlannerContractError::new(
            "void_death.identity",
            "requires the exact GZ2E01/English context",
        ));
    }

    let scope = ContextScope {
        selectors: vec![ContextSelector::Exact {
            context: ExactContext {
                content_sha256,
                runtime_configuration_sha256,
            },
        }],
    };
    let evidence = established_evidence();
    let trigger = "obligation.gz2e01.void-selection-trigger";
    let transition =
        |id: &str,
         label: &str,
         kind: TransitionKind,
         guards: Vec<PredicateExpression>,
         effects: Vec<StateOperation>,
         unknown_requirements: Vec<UnknownRequirement>| CandidateTransition {
            id: id.into(),
            label: label.into(),
            scope: scope.clone(),
            transition_kind: kind,
            approach_id: "approach.gz2e01.void-selection".into(),
            activation: ActivationContract {
                hard_guards: PredicateExpression::All { terms: guards },
                physical_obligation_ids: vec![trigger.into()],
                effects,
                unknown_requirements,
            },
            evidence: evidence.clone(),
        };
    let ordinary_guards = || {
        vec![
            world_active(),
            control_is("selection_requested", StateValue::Boolean(true)),
            control_is("event_acquired", StateValue::Boolean(true)),
            control_is("restart_one_shot", StateValue::Boolean(false)),
            control_is("hazard_variant", StateValue::Text("ordinary".into())),
        ]
    };
    let request_effects = |stage: &str, room: &str, spawn: &str, phase: &str| {
        vec![
            StateOperation::SetExecutionContext {
                context: ExecutionContext::Process {
                    process_name: "PROC_PLAY_SCENE".into(),
                    pending_world_load: None,
                },
            },
            StateOperation::SetPendingWorldLoadFromFields {
                component_id: VOID_CONTROL_COMPONENT_ID.into(),
                stage_field: stage.into(),
                room_field: room.into(),
                spawn_field: spawn.into(),
                layer: -1,
            },
            write_control("restart_one_shot", StateValue::Boolean(true)),
            write_death("phase", StateValue::Text(phase.into())),
            StateOperation::SetPlayerControl { has_control: None },
        ]
    };
    let mut collision_guards = ordinary_guards();
    collision_guards.extend([
        control_is("lethal", StateValue::Boolean(false)),
        control_is("collision_exit_usable", StateValue::Boolean(true)),
    ]);
    let mut restart_guards = ordinary_guards();
    restart_guards.extend([
        control_is("lethal", StateValue::Boolean(false)),
        control_is("collision_exit_usable", StateValue::Boolean(false)),
    ]);
    let mut lethal_guards = ordinary_guards();
    lethal_guards.push(control_is("lethal", StateValue::Boolean(true)));
    let transitions = vec![
        transition(
            "transition.gz2e01.void.01-collision-exit-request",
            "Request the decoded collision-exit hazard destination",
            TransitionKind::VoidReload,
            collision_guards,
            request_effects(
                "collision_destination_stage",
                "collision_destination_room",
                "collision_destination_spawn",
                "collision_exit_requested",
            ),
            Vec::new(),
        ),
        transition(
            "transition.gz2e01.void.02-held-restart-request",
            "Request the decoded held restart-room destination",
            TransitionKind::VoidReload,
            restart_guards,
            request_effects(
                "restart_destination_stage",
                "restart_destination_room",
                "restart_destination_spawn",
                "held_restart_requested",
            ),
            Vec::new(),
        ),
        transition(
            "transition.gz2e01.void.03-lethal-diversion",
            "Divert lethal restart damage into game over",
            TransitionKind::DeathReload,
            lethal_guards,
            vec![
                write_death("phase", StateValue::Text("game_over_wait".into())),
                StateOperation::SetPlayerControl { has_control: None },
                StateOperation::SetPlayerAction {
                    action: "dead".into(),
                },
            ],
            Vec::new(),
        ),
        transition(
            "transition.gz2e01.void.04-return-to-title-request",
            "Convert a declined game-over choice into a guarded reset request",
            TransitionKind::TitleReturn,
            vec![
                death_is("phase", StateValue::Text("game_over_wait".into())),
                death_is("choice", StateValue::Text("decline_continue".into())),
                reset_is("reset_requested", StateValue::Boolean(false)),
            ],
            vec![
                StateOperation::Write {
                    target: ComponentFieldTarget {
                        component_id: RESET_CONTROL_COMPONENT_ID.into(),
                        field: "reset_requested".into(),
                    },
                    value: StateValue::Boolean(true),
                },
                write_death("phase", StateValue::Text("title_reset_requested".into())),
            ],
            Vec::new(),
        ),
        transition(
            "transition.gz2e01.void.05-special-hazard-unresolved",
            "Retain an unresolved special-hazard restart branch",
            TransitionKind::VoidReload,
            vec![
                world_active(),
                control_is("selection_requested", StateValue::Boolean(true)),
                PredicateExpression::Not {
                    term: Box::new(control_is(
                        "hazard_variant",
                        StateValue::Text("ordinary".into()),
                    )),
                },
            ],
            Vec::new(),
            vec![UnknownRequirement {
                id: "unknown.gz2e01.special-hazard-restart-semantics".into(),
                description: "Decode and witness the selected special hazard's damage, mode, room, and packed restart parameter before authorizing a request.".into(),
                evidence: RuleEvidence {
                    truth: TruthStatus::Unknown,
                    records: Vec::new(),
                },
            }],
        ),
    ];
    let catalog = MechanicsCatalog {
        schema: MECHANICS_CATALOG_SCHEMA.into(),
        transitions,
        obligations: vec![FeasibilityObligation {
            id: trigger.into(),
            label: "Enter the void or hazard restart selection logic".into(),
            scope,
            obligation_kind: ObligationKind::VoidPlane,
            stage: ObligationStage::Activate,
            detail: ObligationDetail::Predicate {
                predicate: control_is("trigger_entered", StateValue::Boolean(true)),
            },
            evidence: evidence.clone(),
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
    };
    catalog.validate()?;
    Ok(catalog)
}

fn established_evidence() -> RuleEvidence {
    RuleEvidence {
        truth: TruthStatus::Established,
        records: vec![
            EvidenceRecord {
                id: "binary.gz2e01.check-restart-room".into(),
                kind: EvidenceKind::Extracted,
                source_sha256: Some(parse_digest(
                    "0a557c9bed46530e109182f8d1e648182a6b10c1ded0eec56649b67451d0e92f",
                )),
                note: "Exact function artifact for checkRestartRoom at VA 0x800be3e4; collision-exit, held restart, and lethal branches remain distinct.".into(),
            },
            EvidenceRecord {
                id: "binary.gz2e01.proc-co-dead".into(),
                kind: EvidenceKind::Extracted,
                source_sha256: Some(parse_digest(
                    "84641294b6728c4acad403f2dc37b897df75b8c1e8976808a91f551e2bcd7a4e",
                )),
                note: "Exact function artifact for procCoDead at VA 0x8011c1b4; a declined continue choice requests reset rather than directly loading title state.".into(),
            },
            EvidenceRecord {
                id: "source.gz2e01.void-death-selection".into(),
                kind: EvidenceKind::SourceAudited,
                source_sha256: Some(parse_digest(
                    "e03a99558b9badea3f3976cc7d8c7a11b716a7402de6ad8b8b7832750ae8525c",
                )),
                note: "Source audit establishes mutually exclusive collision-exit, held-restart, lethal, and special-hazard selection branches.".into(),
            },
        ],
    }
}

fn control_is(field: &str, value: StateValue) -> PredicateExpression {
    equals(component_field(VOID_CONTROL_COMPONENT_ID, field), value)
}

fn death_is(field: &str, value: StateValue) -> PredicateExpression {
    equals(component_field(DEATH_FLOW_COMPONENT_ID, field), value)
}

fn reset_is(field: &str, value: StateValue) -> PredicateExpression {
    equals(component_field(RESET_CONTROL_COMPONENT_ID, field), value)
}

fn world_active() -> PredicateExpression {
    equals(
        ValueReference::WorldExecutionActive,
        StateValue::Boolean(true),
    )
}

fn component_field(component_id: &str, field: &str) -> ValueReference {
    ValueReference::ComponentField {
        component_id: component_id.into(),
        field: field.into(),
    }
}

fn equals(left: ValueReference, value: StateValue) -> PredicateExpression {
    PredicateExpression::Compare {
        left,
        operator: ComparisonOperator::Equal,
        right: ValueReference::Literal { value },
    }
}

fn write_control(field: &str, value: StateValue) -> StateOperation {
    StateOperation::Write {
        target: ComponentFieldTarget {
            component_id: VOID_CONTROL_COMPONENT_ID.into(),
            field: field.into(),
        },
        value,
    }
}

fn write_death(field: &str, value: StateValue) -> StateOperation {
    StateOperation::Write {
        target: ComponentFieldTarget {
            component_id: DEATH_FLOW_COMPONENT_ID.into(),
            field: field.into(),
        },
        value,
    }
}

fn parse_digest(value: &str) -> Digest {
    value.parse().expect("compile-time SHA-256 literal")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evaluation::{EvaluatedTruth, EvidencePolicy, PredicateEvaluator};
    use crate::execution::PlannerExecutionState;
    use crate::identity::{
        CONTENT_IDENTITY_SCHEMA, ContentFingerprint, GamePlatform, GameRegion,
        RUNTIME_CONFIGURATION_SCHEMA,
    };
    use crate::logic::{FACT_CATALOG_SCHEMA, FactCatalog};
    use crate::snapshot::{STATE_SNAPSHOT_SCHEMA, StateSnapshot};
    use crate::state::{
        BackingAttachment, ComponentBinding, ComponentKind, ComponentPayload, ComponentProvenance,
        EXECUTION_ENVIRONMENT_SCHEMA, ExecutionEnvironment, PlayerForm, PlayerState,
        ProvenanceSourceKind, RuntimeFile, RuntimeFileLifecycle, RuntimeFileOrigin, SceneLocation,
        SemanticLifetime, SerializationOwner, StateComponent,
    };
    use std::collections::BTreeMap;

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
        fields: BTreeMap<String, StateValue>,
    ) -> StateComponent {
        StateComponent {
            id: id.into(),
            component_kind: kind,
            payload: ComponentPayload::Structured { fields },
            binding: ComponentBinding::RuntimeFile {
                runtime_file_id: "file-1".into(),
            },
            lifetime: SemanticLifetime::RuntimeFile,
            serialization_owner: SerializationOwner::RuntimeFile {
                runtime_file_id: "file-1".into(),
            },
            provenance: vec![ComponentProvenance {
                source_kind: ProvenanceSourceKind::TraceObservation,
                source_id: "fixture.gz2e01-void-selection".into(),
                source_sha256: Some(Digest([7; 32])),
                transition_id: None,
            }],
        }
    }

    fn state(collision_exit_usable: bool, lethal: bool) -> PlannerExecutionState {
        let (_, runtime_configuration) = context();
        let snapshot = StateSnapshot {
            schema: STATE_SNAPSHOT_SCHEMA.into(),
            id: "snapshot.gz2e01-void-selection".into(),
            sequence: 1,
            environment: ExecutionEnvironment {
                schema: EXECUTION_ENVIRONMENT_SCHEMA.into(),
                runtime_configuration,
                active_runtime_file: RuntimeFile {
                    id: "file-1".into(),
                    origin: RuntimeFileOrigin::NewFile,
                    backing: BackingAttachment::MemoryOnly,
                    allowed_serialization_targets: Vec::new(),
                    lifecycle: RuntimeFileLifecycle::Active,
                },
                inactive_runtime_files: Vec::new(),
                physical_slots: Vec::new(),
                physical_slot_observations: Vec::new(),
                execution_context: ExecutionContext::World,
                location: SceneLocation {
                    stage: "F_SP108".into(),
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
                    action: "fall".into(),
                },
                components: vec![
                    component(
                        DEATH_FLOW_COMPONENT_ID,
                        ComponentKind::PendingOperation,
                        BTreeMap::from([
                            ("choice".into(), StateValue::Text("decline_continue".into())),
                            ("phase".into(), StateValue::Text("idle".into())),
                        ]),
                    ),
                    component(
                        VOID_CONTROL_COMPONENT_ID,
                        ComponentKind::PendingOperation,
                        BTreeMap::from([
                            ("collision_destination_room".into(), StateValue::Signed(2)),
                            ("collision_destination_spawn".into(), StateValue::Signed(5)),
                            (
                                "collision_destination_stage".into(),
                                StateValue::Text("F_SP109".into()),
                            ),
                            (
                                "collision_exit_usable".into(),
                                StateValue::Boolean(collision_exit_usable),
                            ),
                            ("event_acquired".into(), StateValue::Boolean(true)),
                            ("hazard_variant".into(), StateValue::Text("ordinary".into())),
                            ("lethal".into(), StateValue::Boolean(lethal)),
                            ("restart_destination_room".into(), StateValue::Signed(0)),
                            ("restart_destination_spawn".into(), StateValue::Signed(3)),
                            (
                                "restart_destination_stage".into(),
                                StateValue::Text("F_SP108".into()),
                            ),
                            ("restart_one_shot".into(), StateValue::Boolean(false)),
                            ("selection_requested".into(), StateValue::Boolean(true)),
                            ("trigger_entered".into(), StateValue::Boolean(true)),
                        ]),
                    ),
                    component(
                        RESET_CONTROL_COMPONENT_ID,
                        ComponentKind::Session,
                        BTreeMap::from([("reset_requested".into(), StateValue::Boolean(false))]),
                    ),
                    component(
                        "restart",
                        ComponentKind::Restart,
                        BTreeMap::from([
                            ("room".into(), StateValue::Signed(0)),
                            ("start_point".into(), StateValue::Signed(3)),
                        ]),
                    ),
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
        PlannerExecutionState::new(snapshot).unwrap()
    }

    fn apply(state: &mut PlannerExecutionState, transition: &CandidateTransition) {
        let facts = FactCatalog {
            schema: FACT_CATALOG_SCHEMA.into(),
            aliases: Vec::new(),
            derived_facts: Vec::new(),
        };
        let evaluator = PredicateEvaluator::new(
            &state.snapshot,
            &facts,
            &[],
            &state.gate_states,
            EvidencePolicy::ESTABLISHED_ONLY,
        )
        .unwrap();
        assert_eq!(
            evaluator.evaluate(&transition.activation.hard_guards),
            EvaluatedTruth::True
        );
        state
            .apply_operations(
                &transition.id,
                "snapshot.after-void-selection",
                &transition.activation.effects,
            )
            .unwrap();
    }

    #[test]
    fn collision_exit_and_held_restart_requests_remain_distinct() {
        let (content, runtime) = context();
        let mechanics = gz2e01_void_death_mechanics(&content, &runtime).unwrap();
        let mut collision = state(true, false);
        let restart_before = collision.snapshot.environment.components[3].payload.clone();
        apply(&mut collision, &mechanics.transitions[0]);
        assert!(matches!(
            collision.snapshot.environment.execution_context,
            ExecutionContext::Process {
                ref process_name,
                pending_world_load: Some(SceneLocation { ref stage, room: 2, spawn: 5, .. })
            } if process_name == "PROC_PLAY_SCENE" && stage == "F_SP109"
        ));
        assert_eq!(
            collision.snapshot.environment.components[3].payload,
            restart_before
        );

        let mut fallback = state(false, false);
        apply(&mut fallback, &mechanics.transitions[1]);
        assert!(matches!(
            fallback.snapshot.environment.execution_context,
            ExecutionContext::Process {
                pending_world_load: Some(SceneLocation { ref stage, room: 0, spawn: 3, .. }),
                ..
            } if stage == "F_SP108"
        ));
    }

    #[test]
    fn lethal_diversion_precedes_the_title_reset_request() {
        let (content, runtime) = context();
        let mechanics = gz2e01_void_death_mechanics(&content, &runtime).unwrap();
        let mut state = state(false, true);
        apply(&mut state, &mechanics.transitions[2]);
        assert_eq!(state.snapshot.environment.player.action, "dead");
        assert!(matches!(
            state.snapshot.environment.execution_context,
            ExecutionContext::World
        ));
        apply(&mut state, &mechanics.transitions[3]);
        let reset = &state.snapshot.environment.components[2];
        let ComponentPayload::Structured { fields } = &reset.payload else {
            unreachable!()
        };
        assert_eq!(fields["reset_requested"], StateValue::Boolean(true));
        assert!(matches!(
            state.snapshot.environment.execution_context,
            ExecutionContext::World
        ));
    }

    #[test]
    fn profile_rejects_neighboring_runtime_configuration() {
        let (content, mut runtime) = context();
        runtime.language = "fr".into();
        assert!(gz2e01_void_death_mechanics(&content, &runtime).is_err());
    }
}
