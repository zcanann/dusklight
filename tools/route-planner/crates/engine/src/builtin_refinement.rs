//! Bundled, opt-in refinement packs for common GZ2E01 techniques.
//!
//! These are ordinary refinement packs rather than hidden solver behavior. A
//! caller can export, inspect, compose, replace, or omit them exactly like an
//! authored pack, and every technique remains scoped to the exact supported
//! content/runtime identity.

use crate::PlannerContractError;
use crate::identity::{ContextSelector, ExactContext};
use crate::logic::{
    ComparisonOperator, ContextScope, EvidenceKind, EvidenceRecord, PredicateExpression,
    RuleEvidence, TruthStatus, ValueReference,
};
use crate::refinement::{
    REFINEMENT_PACK_SCHEMA, RefinementOperation, RefinementPack, RefinementPackManifest,
    RefinementRule,
};
use crate::return_place::{GZ2E01_CONTENT_SHA256, GZ2E01_EN_RUNTIME_SHA256};
use crate::state::{PlayerForm, StateValue};
use crate::transition::{
    FeasibilityObligation, ObligationDetail, ObligationKind, RouteCost, StateOperation, Technique,
};
use std::collections::BTreeMap;

pub const GZ2E01_ORDINARY_MOVEMENT_PACK_ID: &str = "builtin.gz2e01.ordinary-movement";
pub const GZ2E01_SEQUENCE_BREAK_PACK_ID: &str = "builtin.gz2e01.selected-sequence-breaks";

pub fn bundled_refinement_pack_ids() -> [&'static str; 2] {
    [
        GZ2E01_ORDINARY_MOVEMENT_PACK_ID,
        GZ2E01_SEQUENCE_BREAK_PACK_ID,
    ]
}

pub fn bundled_refinement_pack(id: &str) -> Result<RefinementPack, PlannerContractError> {
    match id {
        GZ2E01_ORDINARY_MOVEMENT_PACK_ID => ordinary_movement_pack(),
        GZ2E01_SEQUENCE_BREAK_PACK_ID => selected_sequence_break_pack(),
        _ => Err(PlannerContractError::new(
            "builtin_refinement.id",
            format!("unknown bundled refinement pack {id}"),
        )),
    }
}

fn ordinary_movement_pack() -> Result<RefinementPack, PlannerContractError> {
    let scope = exact_scope();
    let evidence = source_evidence(
        TruthStatus::Established,
        "builtin.gz2e01.ordinary-movement",
        EvidenceKind::SourceAudited,
        "Ordinary controlled locomotion is a selectable approach capability; transitions must opt into its named obligation rather than receiving an implicit solver bypass.",
    );
    let obligation_id = "obligation.gz2e01.ordinary-controlled-movement";
    let pack = RefinementPack {
        schema: REFINEMENT_PACK_SCHEMA.into(),
        manifest: manifest(
            GZ2E01_ORDINARY_MOVEMENT_PACK_ID,
            "Bundled ordinary controlled-movement technique",
        ),
        rules: vec![
            RefinementRule {
                id: "builtin.gz2e01.ordinary-movement.add-obligation".into(),
                label: "Add ordinary controlled-movement obligation".into(),
                operation: RefinementOperation::AddObligation {
                    obligation: FeasibilityObligation {
                        id: obligation_id.into(),
                        label: "Reach the authored approach by ordinary controlled movement".into(),
                        scope: scope.clone(),
                        obligation_kind: ObligationKind::Geometry,
                        stage: crate::transition::ObligationStage::Reach,
                        detail: ObligationDetail::Unresolved {
                            research_question: "Does the authored approach use ordinary traversable collision from the propagated source state?".into(),
                        },
                        evidence: evidence.clone(),
                    },
                },
                evidence: evidence.clone(),
            },
            RefinementRule {
                id: "builtin.gz2e01.ordinary-movement.add-technique".into(),
                label: "Add ordinary controlled movement".into(),
                operation: RefinementOperation::AddTechnique {
                    technique: Technique {
                        id: "technique.gz2e01.ordinary-controlled-movement".into(),
                        label: "Ordinary controlled movement".into(),
                        scope,
                        prerequisites: player_controlled(),
                        operations: Vec::new(),
                        discharged_obligation_ids: vec![obligation_id.into()],
                        introduced_obligation_ids: Vec::new(),
                        cost: RouteCost {
                            axes: BTreeMap::from([
                                ("difficulty".into(), 1),
                                ("time".into(), 1),
                            ]),
                        },
                        evidence: evidence.clone(),
                    },
                },
                evidence,
            },
        ],
    };
    pack.validate()?;
    Ok(pack)
}

fn selected_sequence_break_pack() -> Result<RefinementPack, PlannerContractError> {
    let scope = exact_scope();
    let ems_evidence = source_evidence(
        TruthStatus::Established,
        "builtin.gz2e01.sequence-break.ems",
        EvidenceKind::RouteWitnessed,
        "Early Master Sword permits the modeled human-form state while the Faron twilight story gate remains active.",
    );
    let epona_evidence = source_evidence(
        TruthStatus::Contested,
        "builtin.gz2e01.sequence-break.epona-oob",
        EvidenceKind::CommunityReported,
        "Epona out-of-bounds is opt-in research evidence and discharges only its explicitly named collision-boundary obligation.",
    );
    let rupee_evidence = source_evidence(
        TruthStatus::Contested,
        "builtin.gz2e01.sequence-break.rupee-clip",
        EvidenceKind::CommunityReported,
        "Rupee clip is opt-in research evidence and replaces only its explicitly named charge-attack approach obligation.",
    );
    let epona_obligation = "obligation.gz2e01.epona-oob.collision-boundary";
    let rupee_obligation = "obligation.gz2e01.rupee-clip.charge-attack-approach";
    let mut rules = vec![
        RefinementRule {
            id: "builtin.gz2e01.sequence-break.ems.add-technique".into(),
            label: "Add Early Master Sword human-form technique".into(),
            operation: RefinementOperation::AddTechnique {
                technique: Technique {
                    id: "technique.gz2e01.ems-human-in-faron-twilight".into(),
                    label: "Early Master Sword human form in Faron twilight".into(),
                    scope: scope.clone(),
                    prerequisites: gate_is("story.faron-twilight", true),
                    operations: vec![StateOperation::SetPlayerForm {
                        form: PlayerForm::Human,
                    }],
                    discharged_obligation_ids: Vec::new(),
                    introduced_obligation_ids: Vec::new(),
                    cost: RouteCost {
                        axes: BTreeMap::from([("difficulty".into(), 8)]),
                    },
                    evidence: ems_evidence.clone(),
                },
            },
            evidence: ems_evidence,
        },
        technique_obligation_rule(
            "builtin.gz2e01.sequence-break.epona-oob.add-obligation",
            epona_obligation,
            "Cross the selected Epona collision boundary",
            "Which witnessed Epona OOB setup crosses this authored collision boundary?",
            &scope,
            &epona_evidence,
        ),
        RefinementRule {
            id: "builtin.gz2e01.sequence-break.epona-oob.add-technique".into(),
            label: "Add Epona out-of-bounds technique".into(),
            operation: RefinementOperation::AddTechnique {
                technique: Technique {
                    id: "technique.gz2e01.epona-oob".into(),
                    label: "Epona out of bounds".into(),
                    scope: scope.clone(),
                    prerequisites: PredicateExpression::All {
                        terms: vec![
                            player_controlled(),
                            equals(ValueReference::PlayerForm, StateValue::Text("human".into())),
                            equals(
                                ValueReference::PlayerMount,
                                StateValue::Text("epona".into()),
                            ),
                            PredicateExpression::Not {
                                term: Box::new(gate_is("story.faron-twilight", true)),
                            },
                        ],
                    },
                    operations: Vec::new(),
                    discharged_obligation_ids: vec![epona_obligation.into()],
                    introduced_obligation_ids: Vec::new(),
                    cost: RouteCost {
                        axes: BTreeMap::from([("difficulty".into(), 7)]),
                    },
                    evidence: epona_evidence.clone(),
                },
            },
            evidence: epona_evidence,
        },
        technique_obligation_rule(
            "builtin.gz2e01.sequence-break.rupee-clip.add-obligation",
            rupee_obligation,
            "Replace the charge-attack approach with rupee clip",
            "Which witnessed rupee-clip setup replaces this authored charge-attack approach?",
            &scope,
            &rupee_evidence,
        ),
        RefinementRule {
            id: "builtin.gz2e01.sequence-break.rupee-clip.add-technique".into(),
            label: "Add rupee-clip approach replacement".into(),
            operation: RefinementOperation::AddTechnique {
                technique: Technique {
                    id: "technique.gz2e01.rupee-clip".into(),
                    label: "Rupee clip".into(),
                    scope,
                    prerequisites: PredicateExpression::All {
                        terms: vec![
                            player_controlled(),
                            equals(ValueReference::PlayerForm, StateValue::Text("human".into())),
                            equals(
                                ValueReference::PlayerMount,
                                StateValue::Text("epona".into()),
                            ),
                            PredicateExpression::Not {
                                term: Box::new(gate_is("story.faron-twilight", true)),
                            },
                        ],
                    },
                    operations: Vec::new(),
                    discharged_obligation_ids: vec![rupee_obligation.into()],
                    introduced_obligation_ids: Vec::new(),
                    cost: RouteCost {
                        axes: BTreeMap::from([("difficulty".into(), 6)]),
                    },
                    evidence: rupee_evidence.clone(),
                },
            },
            evidence: rupee_evidence,
        },
    ];
    rules.sort_by(|left, right| left.id.cmp(&right.id));
    let pack = RefinementPack {
        schema: REFINEMENT_PACK_SCHEMA.into(),
        manifest: manifest(
            GZ2E01_SEQUENCE_BREAK_PACK_ID,
            "Bundled selected GZ2E01 sequence-break techniques",
        ),
        rules,
    };
    pack.validate()?;
    Ok(pack)
}

fn technique_obligation_rule(
    id: &str,
    obligation_id: &str,
    label: &str,
    question: &str,
    scope: &ContextScope,
    evidence: &RuleEvidence,
) -> RefinementRule {
    RefinementRule {
        id: id.into(),
        label: label.into(),
        operation: RefinementOperation::AddObligation {
            obligation: FeasibilityObligation {
                id: obligation_id.into(),
                label: label.into(),
                scope: scope.clone(),
                obligation_kind: ObligationKind::Geometry,
                stage: crate::transition::ObligationStage::Reach,
                detail: ObligationDetail::Unresolved {
                    research_question: question.into(),
                },
                evidence: evidence.clone(),
            },
        },
        evidence: evidence.clone(),
    }
}

fn manifest(id: &str, source: &str) -> RefinementPackManifest {
    RefinementPackManifest {
        id: id.into(),
        version: "1.0.0".into(),
        author: "Dusklight route research".into(),
        source: source.into(),
        scope: exact_scope(),
        precedence: -100,
        dependencies: Vec::new(),
        conflicts: Vec::new(),
    }
}

fn exact_scope() -> ContextScope {
    ContextScope {
        selectors: vec![ContextSelector::Exact {
            context: ExactContext {
                content_sha256: GZ2E01_CONTENT_SHA256,
                runtime_configuration_sha256: GZ2E01_EN_RUNTIME_SHA256,
            },
        }],
    }
}

fn source_evidence(truth: TruthStatus, id: &str, kind: EvidenceKind, note: &str) -> RuleEvidence {
    RuleEvidence {
        truth,
        records: vec![EvidenceRecord {
            id: id.into(),
            kind,
            source_sha256: (kind != EvidenceKind::CommunityReported)
                .then_some(GZ2E01_CONTENT_SHA256),
            note: note.into(),
        }],
    }
}

fn player_controlled() -> PredicateExpression {
    equals(ValueReference::PlayerControl, StateValue::Boolean(true))
}

fn gate_is(id: &str, value: bool) -> PredicateExpression {
    equals(
        ValueReference::GateState { gate_id: id.into() },
        StateValue::Boolean(value),
    )
}

fn equals(left: ValueReference, value: StateValue) -> PredicateExpression {
    PredicateExpression::Compare {
        left,
        operator: ComparisonOperator::Equal,
        right: ValueReference::Literal { value },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::TruthStatus;
    use crate::logic::{FACT_CATALOG_SCHEMA, FactCatalog};
    use crate::refinement::{ComposedPlannerCatalog, RefinementOperation};
    use crate::transition::{MECHANICS_CATALOG_SCHEMA, MechanicsCatalog};

    fn empty_catalogs() -> (FactCatalog, MechanicsCatalog) {
        (
            FactCatalog {
                schema: FACT_CATALOG_SCHEMA.into(),
                aliases: Vec::new(),
                derived_facts: Vec::new(),
            },
            MechanicsCatalog {
                schema: MECHANICS_CATALOG_SCHEMA.into(),
                transitions: Vec::new(),
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
            },
        )
    }

    #[test]
    fn registry_exports_canonical_composable_packs() {
        let (facts, mechanics) = empty_catalogs();
        for id in bundled_refinement_pack_ids() {
            let pack = bundled_refinement_pack(id).unwrap();
            assert_eq!(
                pack,
                RefinementPack::decode_canonical(&pack.canonical_bytes().unwrap()).unwrap()
            );
            let composed = ComposedPlannerCatalog::compose(&facts, &mechanics, &[pack]).unwrap();
            assert_eq!(composed.refinement_stack.entries[0].pack_id, id);
        }
    }

    #[test]
    fn sequence_break_evidence_and_effects_remain_method_specific() {
        let pack = bundled_refinement_pack(GZ2E01_SEQUENCE_BREAK_PACK_ID).unwrap();
        let techniques = pack
            .rules
            .iter()
            .filter_map(|rule| match &rule.operation {
                RefinementOperation::AddTechnique { technique } => Some(technique),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(techniques.len(), 3);
        let ems = techniques
            .iter()
            .find(|technique| technique.id.contains("ems-human"))
            .unwrap();
        assert_eq!(ems.evidence.truth, TruthStatus::Established);
        assert_eq!(
            ems.operations,
            vec![StateOperation::SetPlayerForm {
                form: PlayerForm::Human,
            }]
        );
        for technique in techniques
            .iter()
            .filter(|technique| !technique.id.contains("ems-human"))
        {
            assert_eq!(technique.evidence.truth, TruthStatus::Contested);
            assert_eq!(technique.discharged_obligation_ids.len(), 1);
            assert!(technique.operations.is_empty());
        }
        for technique in techniques.iter().filter(|technique| {
            technique.id.contains("epona-oob") || technique.id.contains("rupee-clip")
        }) {
            let PredicateExpression::All { terms } = &technique.prerequisites else {
                panic!("Epona-backed techniques require conjunctive setup");
            };
            assert!(terms.iter().any(|term| matches!(
                term,
                PredicateExpression::Compare {
                    left: ValueReference::PlayerMount,
                    operator: ComparisonOperator::Equal,
                    right,
                } if *right == ValueReference::Literal {
                    value: StateValue::Text("epona".into())
                }
            )));
            assert!(terms.iter().any(|term| matches!(
                term,
                PredicateExpression::Not { term }
                    if matches!(
                        term.as_ref(),
                        PredicateExpression::Compare {
                            left: ValueReference::GateState { gate_id },
                            operator: ComparisonOperator::Equal,
                            right,
                        } if gate_id == "story.faron-twilight"
                            && *right == ValueReference::Literal {
                                value: StateValue::Boolean(true)
                            }
                    )
            )));
        }
    }

    #[test]
    fn unknown_builtin_id_fails_closed() {
        assert!(bundled_refinement_pack("builtin.gz2e01.imaginary").is_err());
    }
}
