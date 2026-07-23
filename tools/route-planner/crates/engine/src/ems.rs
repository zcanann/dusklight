//! Layered Early Master Sword validation mechanics.

use crate::PlannerContractError;
use crate::logic::{
    ComparisonOperator, ContextScope, PredicateExpression, RuleEvidence, TruthStatus,
    ValueReference,
};
use crate::state::{PlayerForm, SceneLocation, StateValue};
use crate::transition::{
    ActivationContract, CandidateTransition, ComponentFieldTarget, FeasibilityObligation, Goal,
    MECHANICS_CATALOG_SCHEMA, MechanicsCatalog, ObligationDetail, ObligationKind, ObligationStage,
    Obstruction, ObstructionResolver, ResolutionKind, RouteCost, StateOperation, Technique,
    TransitionKind,
};
use std::collections::BTreeMap;

pub const EMS_PROGRESS_COMPONENT_ID: &str = "progress.ems-validation";

#[derive(Clone, Debug)]
pub struct EmsValidationProfile {
    pub scope: ContextScope,
    pub evidence: RuleEvidence,
}

impl EmsValidationProfile {
    pub fn compile(&self) -> Result<MechanicsCatalog, PlannerContractError> {
        self.scope.validate("ems.scope")?;
        self.evidence.validate("ems.evidence")?;
        if self.evidence.truth == TruthStatus::Unknown {
            return Err(PlannerContractError::new(
                "ems.evidence",
                "candidate effects require at least hypothetical evidence",
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
                          detail: ObligationDetail,
                          evidence: RuleEvidence| FeasibilityObligation {
            id: id.into(),
            label: label.into(),
            scope: self.scope.clone(),
            obligation_kind: kind,
            stage,
            detail,
            evidence,
        };
        let candidate =
            |id: &str,
             label: &str,
             approach: &str,
             guards: Vec<PredicateExpression>,
             obligations: Vec<&str>,
             effects: Vec<StateOperation>| CandidateTransition {
                id: id.into(),
                label: label.into(),
                scope: self.scope.clone(),
                transition_kind: TransitionKind::ActorDriven,
                approach_id: approach.into(),
                activation: ActivationContract {
                    hard_guards: all(guards),
                    physical_obligation_ids: obligations.into_iter().map(str::to_owned).collect(),
                    effects,
                    unknown_requirements: Vec::new(),
                },
                evidence: self.evidence.clone(),
            };
        let charge = "obligation.ems.charge-attack-approach";
        let standard_boundary = "obligation.ems.standard-castle-boundary";
        let epona_boundary = "obligation.ems.epona-castle-boundary";
        let human = "obligation.ems.human-form";
        let mount = "obligation.ems.epona-mount";
        let epona_execution = "obligation.ems.epona-oob-execution";
        let non_twilight = "obligation.ems.non-twilight";

        let transitions = vec![
            candidate(
                "transition.ems.01-acquire-master-sword",
                "Acquire the Master Sword early",
                "approach.ems.sacred-grove",
                vec![
                    field_is("master_sword_owned", StateValue::Boolean(false)),
                    field_is("hero_clothes_owned", StateValue::Boolean(true)),
                ],
                vec![charge, human],
                vec![write("master_sword_owned", StateValue::Boolean(true))],
            ),
            candidate(
                "transition.ems.02-enter-hyrule-castle-epona",
                "Enter Hyrule Castle through the Epona OOB approach",
                "approach.ems.hyrule-castle.epona-oob",
                vec![field_is("master_sword_owned", StateValue::Boolean(true))],
                vec![epona_boundary, mount, epona_execution, non_twilight],
                vec![StateOperation::SetLocation {
                    location: castle_location(),
                }],
            ),
            candidate(
                "transition.ems.02-enter-hyrule-castle-standard",
                "Enter Hyrule Castle through the standard EMS approach",
                "approach.ems.hyrule-castle.standard",
                vec![field_is("master_sword_owned", StateValue::Boolean(true))],
                vec![standard_boundary],
                vec![StateOperation::SetLocation {
                    location: castle_location(),
                }],
            ),
            candidate(
                "transition.ems.03-defeat-ganon",
                "Defeat Ganon with the Master Sword",
                "approach.ems.ganon",
                vec![
                    location_is("D_MN09"),
                    field_is("master_sword_owned", StateValue::Boolean(true)),
                    field_is("ganon_defeated", StateValue::Boolean(false)),
                ],
                Vec::new(),
                vec![write("ganon_defeated", StateValue::Boolean(true))],
            ),
        ];
        let obligations = vec![
            obligation(
                charge,
                "Cross the charge-attack EMS approach",
                ObligationStage::Reach,
                ObligationKind::Geometry,
                unresolved(
                    "Witness the selected charge-attack approach into the early sword route.",
                ),
                unknown.clone(),
            ),
            obligation(
                epona_boundary,
                "Cross the Epona OOB castle boundary",
                ObligationStage::Reach,
                ObligationKind::Geometry,
                unresolved("Witness the selected Epona OOB collision boundary."),
                unknown.clone(),
            ),
            obligation(
                mount,
                "Remain mounted on Epona for the OOB approach",
                ObligationStage::Activate,
                ObligationKind::Mount,
                predicate(player_mount_is_epona()),
                self.evidence.clone(),
            ),
            obligation(
                epona_execution,
                "Execute the Epona OOB technique",
                ObligationStage::Activate,
                ObligationKind::Geometry,
                unresolved("Witness the Epona OOB setup and boundary-crossing execution."),
                unknown.clone(),
            ),
            obligation(
                human,
                "Remain human during the EMS setup",
                ObligationStage::Activate,
                ObligationKind::Form,
                predicate(player_form_is_human()),
                self.evidence.clone(),
            ),
            obligation(
                non_twilight,
                "Use Epona only outside Faron twilight",
                ObligationStage::Activate,
                ObligationKind::Twilight,
                predicate(not_twilight()),
                self.evidence.clone(),
            ),
            obligation(
                standard_boundary,
                "Cross the standard EMS castle boundary",
                ObligationStage::Reach,
                ObligationKind::Geometry,
                unresolved(
                    "Witness the standard EMS geometry through the selected castle approach.",
                ),
                unknown,
            ),
        ];
        let obstructions = vec![
            Obstruction {
                id: "obstruction.ems.epona-boundary".into(),
                label: "Epona OOB collision boundary".into(),
                scope: self.scope.clone(),
                blocked_action_id: "transition.ems.02-enter-hyrule-castle-epona".into(),
                approach_id: "approach.ems.hyrule-castle.epona-oob".into(),
                active_when: PredicateExpression::True,
                obligation_ids: vec![epona_boundary.into()],
                evidence: self.evidence.clone(),
            },
            Obstruction {
                id: "obstruction.ems.epona-mount".into(),
                label: "Epona is not mounted".into(),
                scope: self.scope.clone(),
                blocked_action_id: "transition.ems.02-enter-hyrule-castle-epona".into(),
                approach_id: "approach.ems.hyrule-castle.epona-oob".into(),
                active_when: PredicateExpression::Not {
                    term: Box::new(player_mount_is_epona()),
                },
                obligation_ids: vec![mount.into()],
                evidence: self.evidence.clone(),
            },
            Obstruction {
                id: "obstruction.ems.faron-twilight".into(),
                label: "Faron twilight blocks the castle approach".into(),
                scope: self.scope.clone(),
                blocked_action_id: "transition.ems.02-enter-hyrule-castle-standard".into(),
                approach_id: "approach.ems.hyrule-castle.standard".into(),
                active_when: twilight(),
                obligation_ids: vec![non_twilight.into()],
                evidence: self.evidence.clone(),
            },
            Obstruction {
                id: "obstruction.ems.standard-boundary".into(),
                label: "Standard EMS castle geometry".into(),
                scope: self.scope.clone(),
                blocked_action_id: "transition.ems.02-enter-hyrule-castle-standard".into(),
                approach_id: "approach.ems.hyrule-castle.standard".into(),
                active_when: PredicateExpression::True,
                obligation_ids: vec![standard_boundary.into()],
                evidence: self.evidence.clone(),
            },
            Obstruction {
                id: "obstruction.ems.wolf-form".into(),
                label: "Wolf form blocks the human EMS interaction".into(),
                scope: self.scope.clone(),
                blocked_action_id: "transition.ems.01-acquire-master-sword".into(),
                approach_id: "approach.ems.sacred-grove".into(),
                active_when: PredicateExpression::Not {
                    term: Box::new(player_form_is_human()),
                },
                obligation_ids: vec![human.into()],
                evidence: self.evidence.clone(),
            },
        ];
        let techniques = vec![
            Technique {
                id: "technique.ems.epona-oob".into(),
                label: "Epona out of bounds".into(),
                scope: self.scope.clone(),
                prerequisites: all(vec![
                    player_controlled(),
                    player_form_is_human(),
                    player_mount_is_epona(),
                    not_twilight(),
                ]),
                operations: Vec::new(),
                discharged_obligation_ids: vec![epona_execution.into()],
                introduced_obligation_ids: Vec::new(),
                cost: cost(7),
                evidence: self.evidence.clone(),
            },
            Technique {
                id: "technique.ems.rupee-clip".into(),
                label: "Rupee clip charge-attack replacement".into(),
                scope: self.scope.clone(),
                prerequisites: all(vec![
                    player_controlled(),
                    player_form_is_human(),
                    player_mount_is_epona(),
                    not_twilight(),
                ]),
                operations: Vec::new(),
                discharged_obligation_ids: vec![charge.into()],
                introduced_obligation_ids: Vec::new(),
                cost: cost(6),
                evidence: self.evidence.clone(),
            },
            Technique {
                id: "technique.ems.standard-setup".into(),
                label: "Standard Early Master Sword setup".into(),
                scope: self.scope.clone(),
                prerequisites: all(vec![
                    field_is("hero_clothes_owned", StateValue::Boolean(true)),
                    field_is("charge_attack_available", StateValue::Boolean(true)),
                    twilight(),
                ]),
                operations: vec![StateOperation::SetPlayerForm {
                    form: PlayerForm::Human,
                }],
                discharged_obligation_ids: vec![charge.into(), standard_boundary.into()],
                introduced_obligation_ids: Vec::new(),
                cost: cost(8),
                evidence: self.evidence.clone(),
            },
        ];
        let catalog = MechanicsCatalog {
            schema: MECHANICS_CATALOG_SCHEMA.into(),
            transitions,
            obligations,
            writers: Vec::new(),
            gates: Vec::new(),
            readers: Vec::new(),
            reconstruction_rules: Vec::new(),
            obstructions,
            resolvers: vec![
                ObstructionResolver {
                    id: "resolver.ems.epona-boundary-bypass".into(),
                    label: "Bypass the castle boundary with Epona OOB".into(),
                    scope: self.scope.clone(),
                    obstruction_id: "obstruction.ems.epona-boundary".into(),
                    resolution_kind: ResolutionKind::Bypass,
                    applicable_when: all(vec![
                        player_controlled(),
                        player_form_is_human(),
                        player_mount_is_epona(),
                        not_twilight(),
                    ]),
                    operations: Vec::new(),
                    evidence: self.evidence.clone(),
                },
                ObstructionResolver {
                    id: "resolver.ems.standard-boundary-bypass".into(),
                    label: "Bypass the castle boundary with standard EMS".into(),
                    scope: self.scope.clone(),
                    obstruction_id: "obstruction.ems.standard-boundary".into(),
                    resolution_kind: ResolutionKind::Bypass,
                    applicable_when: all(vec![
                        field_is("hero_clothes_owned", StateValue::Boolean(true)),
                        field_is("charge_attack_available", StateValue::Boolean(true)),
                        twilight(),
                    ]),
                    operations: Vec::new(),
                    evidence: self.evidence.clone(),
                },
            ],
            techniques,
            microtraces: Vec::new(),
            goals: vec![Goal {
                id: "goal.ems.defeat-ganon".into(),
                label: "Defeat Ganon through an EMS route".into(),
                predicate: field_is("ganon_defeated", StateValue::Boolean(true)),
            }],
        };
        catalog.validate()?;
        Ok(catalog)
    }
}

fn target(field: &str) -> ComponentFieldTarget {
    ComponentFieldTarget {
        component_id: EMS_PROGRESS_COMPONENT_ID.into(),
        field: field.into(),
    }
}

fn write(field: &str, value: StateValue) -> StateOperation {
    StateOperation::Write {
        target: target(field),
        value,
    }
}

fn field_is(field: &str, value: StateValue) -> PredicateExpression {
    equals(
        ValueReference::ComponentField {
            component_id: EMS_PROGRESS_COMPONENT_ID.into(),
            field: field.into(),
        },
        value,
    )
}

fn location_is(stage: &str) -> PredicateExpression {
    equals(
        ValueReference::LocationStage,
        StateValue::Text(stage.into()),
    )
}

fn twilight() -> PredicateExpression {
    equals(
        ValueReference::GateState {
            gate_id: "story.faron-twilight".into(),
        },
        StateValue::Boolean(true),
    )
}

fn not_twilight() -> PredicateExpression {
    PredicateExpression::Not {
        term: Box::new(twilight()),
    }
}

fn player_controlled() -> PredicateExpression {
    equals(ValueReference::PlayerControl, StateValue::Boolean(true))
}

fn player_form_is_human() -> PredicateExpression {
    equals(ValueReference::PlayerForm, StateValue::Text("human".into()))
}

fn player_mount_is_epona() -> PredicateExpression {
    equals(
        ValueReference::PlayerMount,
        StateValue::Text("epona".into()),
    )
}

fn equals(left: ValueReference, value: StateValue) -> PredicateExpression {
    PredicateExpression::Compare {
        left,
        operator: ComparisonOperator::Equal,
        right: ValueReference::Literal { value },
    }
}

fn all(terms: Vec<PredicateExpression>) -> PredicateExpression {
    PredicateExpression::All { terms }
}

fn predicate(predicate: PredicateExpression) -> ObligationDetail {
    ObligationDetail::Predicate { predicate }
}

fn unresolved(question: &str) -> ObligationDetail {
    ObligationDetail::Unresolved {
        research_question: question.into(),
    }
}

fn cost(difficulty: u64) -> RouteCost {
    RouteCost {
        axes: BTreeMap::from([("difficulty".into(), difficulty)]),
    }
}

fn castle_location() -> SceneLocation {
    SceneLocation {
        stage: "D_MN09".into(),
        room: 0,
        layer: 0,
        spawn: 0,
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
    use crate::snapshot::{STATE_SNAPSHOT_SCHEMA, StateSnapshot};
    use crate::solver::{ForwardSolver, SearchStatus, SolverOptions};
    use crate::state::{
        BackingAttachment, ComponentBinding, ComponentKind, ComponentPayload, ComponentProvenance,
        EXECUTION_ENVIRONMENT_SCHEMA, ExecutionContext, ExecutionEnvironment, PlayerMount,
        PlayerState, ProvenanceSourceKind, RuntimeFile, RuntimeFileLifecycle, RuntimeFileOrigin,
        SemanticLifetime, SerializationOwner, StateComponent,
    };

    fn snapshot(
        form: PlayerForm,
        mount: Option<PlayerMount>,
        twilight: bool,
    ) -> PlannerExecutionState {
        let runtime_configuration = RuntimeConfiguration {
            schema: RUNTIME_CONFIGURATION_SCHEMA.into(),
            content_sha256: Digest([1; 32]),
            language: "en".into(),
            settings: BTreeMap::new(),
        };
        let progress = StateComponent {
            id: EMS_PROGRESS_COMPONENT_ID.into(),
            component_kind: ComponentKind::Custom {
                id: "ems-validation".into(),
            },
            payload: ComponentPayload::Structured {
                fields: BTreeMap::from([
                    ("charge_attack_available".into(), StateValue::Boolean(true)),
                    ("ganon_defeated".into(), StateValue::Boolean(false)),
                    ("hero_clothes_owned".into(), StateValue::Boolean(true)),
                    ("master_sword_owned".into(), StateValue::Boolean(false)),
                ]),
            },
            binding: ComponentBinding::RuntimeFile {
                runtime_file_id: "file-1".into(),
            },
            lifetime: SemanticLifetime::RuntimeFile,
            serialization_owner: SerializationOwner::RuntimeFile {
                runtime_file_id: "file-1".into(),
            },
            provenance: vec![ComponentProvenance {
                source_kind: ProvenanceSourceKind::Initialized,
                source_id: "fixture.ems-start".into(),
                source_sha256: Some(Digest([3; 32])),
                transition_id: None,
            }],
        };
        let snapshot = StateSnapshot {
            schema: STATE_SNAPSHOT_SCHEMA.into(),
            id: "snapshot.ems-start".into(),
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
                    form,
                    mount,
                    position: [0.0; 3],
                    attention_position: None,
                    rotation: [0; 3],
                    has_control: Some(true),
                    action: "idle".into(),
                },
                components: vec![progress],
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
            .gate_states
            .insert("story.faron-twilight".into(), twilight);
        state
    }

    fn profile(state: &PlannerExecutionState) -> EmsValidationProfile {
        EmsValidationProfile {
            scope: ContextScope {
                selectors: vec![ContextSelector::Exact {
                    context: state
                        .snapshot
                        .environment
                        .runtime_configuration
                        .exact_context()
                        .unwrap(),
                }],
            },
            evidence: RuleEvidence {
                truth: TruthStatus::Contested,
                records: vec![EvidenceRecord {
                    id: "evidence.community.ems-validation".into(),
                    kind: EvidenceKind::CommunityReported,
                    source_sha256: Some(Digest([4; 32])),
                    note: "EMS route fixture pending exact geometry import.".into(),
                }],
            },
        }
    }

    fn facts() -> FactCatalog {
        FactCatalog {
            schema: FACT_CATALOG_SCHEMA.into(),
            aliases: Vec::new(),
            derived_facts: Vec::new(),
        }
    }

    fn options(mode: FeasibilityMode) -> SolverOptions {
        SolverOptions {
            max_depth: 8,
            max_states: 256,
            max_resolution_combinations: 16,
            feasibility_mode: mode,
            evidence_policy: EvidencePolicy::RESEARCH,
        }
    }

    fn solve(
        state: PlannerExecutionState,
        mechanics: &MechanicsCatalog,
        mode: FeasibilityMode,
    ) -> crate::solver::SearchResult {
        let facts = facts();
        ForwardSolver::new(&facts, mechanics, &[], options(mode))
            .unwrap()
            .solve(state, &mechanics.goals[0].predicate)
            .unwrap()
    }

    #[test]
    fn upper_bound_produces_an_ems_logic_path() {
        let state = snapshot(PlayerForm::Wolf, None, true);
        let mechanics = profile(&state).compile().unwrap();
        let result = solve(state, &mechanics, FeasibilityMode::UpperBound);
        assert_eq!(result.status, SearchStatus::Reached, "{result:#?}");
        assert!(
            result
                .steps
                .iter()
                .any(|step| { step.action_id == "transition.ems.03-defeat-ganon" })
        );
    }

    #[test]
    fn enabling_obstruction_knowledge_refines_the_upper_bound() {
        let state = snapshot(PlayerForm::Wolf, None, true);
        let mechanics = profile(&state).compile().unwrap();
        let modeled = solve(state.clone(), &mechanics, FeasibilityMode::Modeled);
        assert_ne!(modeled.status, SearchStatus::Reached);
        assert!(modeled.blocked_transition_witnesses.iter().any(|witness| {
            witness.transition_id == "transition.ems.02-enter-hyrule-castle-standard"
        }));

        let mut without_obstructions = mechanics;
        without_obstructions.obstructions.clear();
        without_obstructions.resolvers.clear();
        without_obstructions.validate().unwrap();
        let refined = solve(state, &without_obstructions, FeasibilityMode::Modeled);
        assert_eq!(refined.status, SearchStatus::Reached);
    }

    #[test]
    fn epona_and_rupee_techniques_compose_outside_twilight() {
        let mut state = snapshot(PlayerForm::Human, Some(PlayerMount::Epona), false);
        let progress = state
            .snapshot
            .environment
            .components
            .iter_mut()
            .find(|component| component.id == EMS_PROGRESS_COMPONENT_ID)
            .unwrap();
        let ComponentPayload::Structured { fields } = &mut progress.payload else {
            unreachable!()
        };
        fields.insert("charge_attack_available".into(), StateValue::Boolean(false));

        let mechanics = profile(&state).compile().unwrap();
        let result = solve(state, &mechanics, FeasibilityMode::Modeled);
        assert_eq!(result.status, SearchStatus::Reached, "{result:#?}");
        let selected = result
            .steps
            .iter()
            .flat_map(|step| step.selected_technique_ids.iter().map(String::as_str))
            .collect::<Vec<_>>();
        assert!(selected.contains(&"technique.ems.rupee-clip"));
        assert!(selected.contains(&"technique.ems.epona-oob"));
        assert!(
            result
                .steps
                .iter()
                .any(|step| { step.action_id == "transition.ems.02-enter-hyrule-castle-epona" })
        );
    }
}
