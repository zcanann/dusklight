//! Composable ordinary Fishing Rod quest producers.

use crate::PlannerContractError;
use crate::logic::{
    ComparisonOperator, ContextScope, PredicateExpression, RuleEvidence, TruthStatus,
    ValueReference,
};
use crate::state::StateValue;
use crate::transition::{
    ActivationContract, CandidateTransition, ComponentFieldTarget, FeasibilityObligation, Goal,
    MECHANICS_CATALOG_SCHEMA, MechanicsCatalog, ObligationDetail, ObligationKind, ObligationStage,
    StateOperation, TransitionKind,
};

pub const ORDINARY_ROD_QUEST_COMPONENT_ID: &str = "quest.ordon-fishing-rod";
pub const ORDINARY_ROD_INVENTORY_COMPONENT_ID: &str = "inventory.active";
pub const FISHING_ROD_ITEM_ID: u8 = 0x4a;

#[derive(Clone, Debug)]
pub struct OrdinaryFishingRodProfile {
    pub scope: ContextScope,
    pub evidence: RuleEvidence,
}

impl OrdinaryFishingRodProfile {
    pub fn compile(&self) -> Result<MechanicsCatalog, PlannerContractError> {
        self.scope.validate("ordinary_fishing_rod.scope")?;
        self.evidence.validate("ordinary_fishing_rod.evidence")?;
        if self.evidence.truth == TruthStatus::Unknown {
            return Err(PlannerContractError::new(
                "ordinary_fishing_rod.evidence",
                "producer effects require at least hypothetical evidence",
            ));
        }

        let unknown = RuleEvidence {
            truth: TruthStatus::Unknown,
            records: Vec::new(),
        };
        let obligation = |id: &str,
                          label: &str,
                          stage: ObligationStage,
                          kind: ObligationKind,
                          question: &str| FeasibilityObligation {
            id: id.into(),
            label: label.into(),
            scope: self.scope.clone(),
            obligation_kind: kind,
            stage,
            detail: ObligationDetail::Unresolved {
                research_question: question.into(),
            },
            evidence: unknown.clone(),
        };
        let transition =
            |id: &str,
             label: &str,
             kind: TransitionKind,
             guards: Vec<PredicateExpression>,
             obligations: Vec<&str>,
             effects: Vec<StateOperation>| CandidateTransition {
                id: id.into(),
                label: label.into(),
                scope: self.scope.clone(),
                transition_kind: kind,
                approach_id: format!("approach.{id}"),
                activation: ActivationContract {
                    hard_guards: all(guards),
                    physical_obligation_ids: obligations.into_iter().map(str::to_owned).collect(),
                    effects,
                    unknown_requirements: Vec::new(),
                },
                evidence: self.evidence.clone(),
            };

        let transitions = vec![
            transition(
                "transition.fishing-rod.ordinary.01-vine-guidance",
                "Receive the ordinary vine-route guidance",
                TransitionKind::MessageAction,
                vec![field_is("vine_guidance", StateValue::Boolean(false))],
                vec!["obligation.fishing-rod.ordinary.vine-talk"],
                vec![write("vine_guidance", StateValue::Boolean(true))],
            ),
            transition(
                "transition.fishing-rod.ordinary.02-reach-hawk-grass",
                "Climb the vine route and reach the hawk grass",
                TransitionKind::ActorDriven,
                vec![field_is("vine_guidance", StateValue::Boolean(true))],
                vec!["obligation.fishing-rod.ordinary.reach-hawk-grass"],
                vec![write("hawk_perch_reached", StateValue::Boolean(true))],
            ),
            transition(
                "transition.fishing-rod.ordinary.03-hawk-displaces-cradle",
                "Use the hawk to displace the cradle",
                TransitionKind::ActorDriven,
                vec![
                    field_is("hawk_perch_reached", StateValue::Boolean(true)),
                    field_is("cradle_state", StateValue::Text("held-by-monkey".into())),
                ],
                vec!["obligation.fishing-rod.ordinary.hawk-command"],
                vec![write("cradle_state", StateValue::Text("grounded".into()))],
            ),
            transition(
                "transition.fishing-rod.ordinary.04-pick-up-cradle",
                "Pick up the displaced cradle",
                TransitionKind::ActorDriven,
                vec![field_is(
                    "cradle_state",
                    StateValue::Text("grounded".into()),
                )],
                vec!["obligation.fishing-rod.ordinary.cradle-pickup"],
                vec![
                    write("carrying_cradle", StateValue::Boolean(true)),
                    write("cradle_state", StateValue::Text("carried".into())),
                ],
            ),
            transition(
                "transition.fishing-rod.ordinary.05-return-cradle-to-uli",
                "Return the cradle to Uli",
                TransitionKind::NpcReward,
                vec![
                    field_is("carrying_cradle", StateValue::Boolean(true)),
                    field_is("cradle_returned", StateValue::Boolean(false)),
                ],
                vec!["obligation.fishing-rod.ordinary.uli-cradle-return"],
                vec![
                    write("carrying_cradle", StateValue::Boolean(false)),
                    write("cradle_returned", StateValue::Boolean(true)),
                    write("cradle_state", StateValue::Text("returned".into())),
                ],
            ),
            transition(
                "transition.fishing-rod.ordinary.06-uli-reward",
                "Receive the Fishing Rod from Uli",
                TransitionKind::NpcReward,
                vec![
                    field_is("cradle_returned", StateValue::Boolean(true)),
                    field_is("reward_claimed", StateValue::Boolean(false)),
                ],
                vec![
                    "obligation.fishing-rod.ordinary.uli-reward-effect",
                    "obligation.fishing-rod.ordinary.uli-reward-talk",
                ],
                vec![
                    write(
                        "reward_item_id",
                        StateValue::Unsigned(u64::from(FISHING_ROD_ITEM_ID)),
                    ),
                    StateOperation::SetBitFromValue {
                        source: target("reward_item_id"),
                        target: ComponentFieldTarget {
                            component_id: ORDINARY_ROD_INVENTORY_COMPONENT_ID.into(),
                            field: "owned_item_ids".into(),
                        },
                    },
                    write("reward_claimed", StateValue::Boolean(true)),
                ],
            ),
        ];
        let obligations = vec![
            obligation(
                "obligation.fishing-rod.ordinary.cradle-pickup",
                "Reach and pick up the displaced cradle",
                ObligationStage::Activate,
                ObligationKind::Interaction,
                "Import the cradle pickup volume, control state, and carry acceptance rules.",
            ),
            obligation(
                "obligation.fishing-rod.ordinary.hawk-command",
                "Activate the hawk command toward the cradle",
                ObligationStage::Activate,
                ObligationKind::Interaction,
                "Import the grass interaction, hawk command, target selection, and cradle displacement behavior.",
            ),
            obligation(
                "obligation.fishing-rod.ordinary.reach-hawk-grass",
                "Reach the ordinary hawk-grass approach",
                ObligationStage::Reach,
                ObligationKind::Geometry,
                "Prove ordinary collision connectivity from the vine route to the hawk-grass approach.",
            ),
            obligation(
                "obligation.fishing-rod.ordinary.uli-cradle-return",
                "Activate Uli's cradle-return interaction",
                ObligationStage::Activate,
                ObligationKind::Interaction,
                "Import Uli's talk/attention geometry and acceptance of the carried cradle state.",
            ),
            obligation(
                "obligation.fishing-rod.ordinary.uli-reward-effect",
                "Commit Uli's Fishing Rod reward",
                ObligationStage::Effect,
                ObligationKind::ActorState,
                "Import the exact reward phase that grants item 0x4a and commits its quest state.",
            ),
            obligation(
                "obligation.fishing-rod.ordinary.uli-reward-talk",
                "Activate Uli's Fishing Rod reward dialogue",
                ObligationStage::Activate,
                ObligationKind::Interaction,
                "Import Uli's post-cradle reward dialogue activation and control requirements.",
            ),
            obligation(
                "obligation.fishing-rod.ordinary.vine-talk",
                "Activate the ordinary vine-route dialogue",
                ObligationStage::Activate,
                ObligationKind::Interaction,
                "Import the vine NPC's talk/attention geometry and ordinary quest-state branch.",
            ),
        ];
        let catalog = MechanicsCatalog {
            schema: MECHANICS_CATALOG_SCHEMA.into(),
            transitions,
            obligations,
            writers: Vec::new(),
            gates: Vec::new(),
            readers: Vec::new(),
            reconstruction_rules: Vec::new(),
            obstructions: Vec::new(),
            resolvers: Vec::new(),
            techniques: Vec::new(),
            microtraces: Vec::new(),
            goals: vec![Goal {
                id: "goal.obtain-fishing-rod".into(),
                label: "Obtain Fishing Rod".into(),
                predicate: fishing_rod_owned(),
            }],
        };
        catalog.validate()?;
        Ok(catalog)
    }
}

pub fn fishing_rod_owned() -> PredicateExpression {
    let mut mask = vec![0; 32];
    mask[usize::from(FISHING_ROD_ITEM_ID / 8)] = 1 << (FISHING_ROD_ITEM_ID % 8);
    PredicateExpression::Compare {
        left: ValueReference::ComponentField {
            component_id: ORDINARY_ROD_INVENTORY_COMPONENT_ID.into(),
            field: "owned_item_ids".into(),
        },
        operator: ComparisonOperator::ContainsBits,
        right: ValueReference::Literal {
            value: StateValue::Bytes(mask),
        },
    }
}

fn field_is(field: &str, value: StateValue) -> PredicateExpression {
    PredicateExpression::Compare {
        left: ValueReference::ComponentField {
            component_id: ORDINARY_ROD_QUEST_COMPONENT_ID.into(),
            field: field.into(),
        },
        operator: ComparisonOperator::Equal,
        right: ValueReference::Literal { value },
    }
}

fn all(terms: Vec<PredicateExpression>) -> PredicateExpression {
    match terms.as_slice() {
        [term] => term.clone(),
        _ => PredicateExpression::All { terms },
    }
}

fn target(field: &str) -> ComponentFieldTarget {
    ComponentFieldTarget {
        component_id: ORDINARY_ROD_QUEST_COMPONENT_ID.into(),
        field: field.into(),
    }
}

fn write(field: &str, value: StateValue) -> StateOperation {
    StateOperation::Write {
        target: target(field),
        value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact::Digest;
    use crate::evaluation::{EvidencePolicy, FeasibilityMode};
    use crate::execution::PlannerExecutionState;
    use crate::identity::{ContextSelector, RUNTIME_CONFIGURATION_SCHEMA, RuntimeConfiguration};
    use crate::logic::{EvidenceKind, EvidenceRecord, FACT_CATALOG_SCHEMA, FactCatalog};
    use crate::obligation_coverage::ObligationCoverageReport;
    use crate::snapshot::{STATE_SNAPSHOT_SCHEMA, StateSnapshot};
    use crate::solver::{ForwardSolver, SearchStatus, SolverOptions};
    use crate::state::{
        BackingAttachment, ComponentBinding, ComponentKind, ComponentPayload, ComponentProvenance,
        EXECUTION_ENVIRONMENT_SCHEMA, ExecutionContext, ExecutionEnvironment, PlayerForm,
        PlayerState, ProvenanceSourceKind, RuntimeFile, RuntimeFileLifecycle, RuntimeFileOrigin,
        SceneLocation, SemanticLifetime, SerializationOwner, StateComponent,
    };
    use std::collections::BTreeMap;

    fn snapshot() -> StateSnapshot {
        let runtime_configuration = RuntimeConfiguration {
            schema: RUNTIME_CONFIGURATION_SCHEMA.into(),
            content_sha256: Digest([1; 32]),
            language: "en".into(),
            settings: BTreeMap::new(),
        };
        let component = |id: &str, kind: ComponentKind, fields| StateComponent {
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
                source_kind: ProvenanceSourceKind::Initialized,
                source_id: "fixture.ordinary-rod-start".into(),
                source_sha256: Some(Digest([3; 32])),
                transition_id: None,
            }],
        };
        StateSnapshot {
            schema: STATE_SNAPSHOT_SCHEMA.into(),
            id: "snapshot.ordinary-rod-start".into(),
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
                components: vec![
                    component(
                        ORDINARY_ROD_INVENTORY_COMPONENT_ID,
                        ComponentKind::Inventory,
                        BTreeMap::from([("owned_item_ids".into(), StateValue::Bytes(vec![0; 32]))]),
                    ),
                    component(
                        ORDINARY_ROD_QUEST_COMPONENT_ID,
                        ComponentKind::Custom {
                            id: "ordon-fishing-rod-quest".into(),
                        },
                        BTreeMap::from([
                            ("carrying_cradle".into(), StateValue::Boolean(false)),
                            (
                                "cradle_state".into(),
                                StateValue::Text("held-by-monkey".into()),
                            ),
                            ("cradle_returned".into(), StateValue::Boolean(false)),
                            ("hawk_perch_reached".into(), StateValue::Boolean(false)),
                            ("reward_claimed".into(), StateValue::Boolean(false)),
                            ("reward_item_id".into(), StateValue::Unsigned(0xff)),
                            ("vine_guidance".into(), StateValue::Boolean(false)),
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
        }
    }

    fn profile(snapshot: &StateSnapshot) -> OrdinaryFishingRodProfile {
        OrdinaryFishingRodProfile {
            scope: ContextScope {
                selectors: vec![ContextSelector::Exact {
                    context: snapshot
                        .environment
                        .runtime_configuration
                        .exact_context()
                        .unwrap(),
                }],
            },
            evidence: RuleEvidence {
                truth: TruthStatus::Contested,
                records: vec![EvidenceRecord {
                    id: "evidence.community.ordinary-fishing-rod-route".into(),
                    kind: EvidenceKind::CommunityReported,
                    source_sha256: Some(Digest([4; 32])),
                    note: "Ordinary quest ordering fixture pending exact actor/event import."
                        .into(),
                }],
            },
        }
    }

    #[test]
    fn ordinary_chain_is_state_causal_and_keeps_physics_explicit() {
        let snapshot = snapshot();
        let mechanics = profile(&snapshot).compile().unwrap();
        let coverage = ObligationCoverageReport::build(&mechanics).unwrap();
        assert_eq!(coverage.transitions.len(), 6);
        assert_eq!(
            coverage.transitions[5].effect_obligation_ids,
            ["obligation.fishing-rod.ordinary.uli-reward-effect"]
        );

        let facts = FactCatalog {
            schema: FACT_CATALOG_SCHEMA.into(),
            aliases: Vec::new(),
            derived_facts: Vec::new(),
        };
        let options = SolverOptions {
            max_depth: 8,
            max_states: 256,
            max_resolution_combinations: 16,
            feasibility_mode: FeasibilityMode::UpperBound,
            evidence_policy: EvidencePolicy::RESEARCH,
        };
        let result = ForwardSolver::new(&facts, &mechanics, &[], options)
            .unwrap()
            .solve(
                PlannerExecutionState::new(snapshot.clone()).unwrap(),
                &fishing_rod_owned(),
            )
            .unwrap();
        assert_eq!(result.status, SearchStatus::Reached);
        assert_eq!(result.steps.len(), 6);
        assert_eq!(
            result.steps.last().unwrap().action_id,
            "transition.fishing-rod.ordinary.06-uli-reward"
        );

        let modeled = ForwardSolver::new(
            &facts,
            &mechanics,
            &[],
            SolverOptions {
                feasibility_mode: FeasibilityMode::Modeled,
                ..options
            },
        )
        .unwrap()
        .solve(
            PlannerExecutionState::new(snapshot).unwrap(),
            &fishing_rod_owned(),
        )
        .unwrap();
        assert_eq!(modeled.status, SearchStatus::Unknown);
        assert!(
            modeled
                .unknown_transition_ids
                .contains(&"transition.fishing-rod.ordinary.01-vine-guidance".into())
        );
    }
}
