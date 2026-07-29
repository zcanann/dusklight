
use super::*;
use crate::identity::{ContextSelector, ExactContext};
use crate::logic::{
    DerivedFact, EvidenceKind, EvidenceRecord, FACT_CATALOG_SCHEMA, RuleEvidence, TruthStatus,
};
use crate::transition::{Goal, MECHANICS_CATALOG_SCHEMA, RouteCost, Technique};

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

fn alternate_scope() -> ContextScope {
    ContextScope {
        selectors: vec![ContextSelector::Exact {
            context: ExactContext {
                content_sha256: Digest([9; 32]),
                runtime_configuration_sha256: Digest([8; 32]),
            },
        }],
    }
}

fn evidence() -> RuleEvidence {
    RuleEvidence {
        truth: TruthStatus::Established,
        records: vec![EvidenceRecord {
            id: "source.route-book".into(),
            kind: EvidenceKind::CommunityReported,
            source_sha256: Some(Digest([3; 32])),
            note: "Documented route method.".into(),
        }],
    }
}

fn catalogs() -> (FactCatalog, MechanicsCatalog) {
    let facts = FactCatalog {
        schema: FACT_CATALOG_SCHEMA.into(),
        aliases: Vec::new(),
        derived_facts: vec![DerivedFact {
            id: "inventory.fishing-rod".into(),
            label: "Fishing Rod obtained".into(),
            scope: scope(),
            rule: PredicateExpression::True,
            evidence: evidence(),
        }],
    };
    let mechanics = MechanicsCatalog {
        schema: MECHANICS_CATALOG_SCHEMA.into(),
        transitions: Vec::new(),
        obligations: Vec::new(),
        writers: Vec::new(),
        gates: Vec::new(),
        readers: Vec::new(),
        reconstruction_rules: Vec::new(),
        obstructions: Vec::new(),
        resolvers: Vec::new(),
        techniques: vec![
            Technique {
                id: "technique.chicken-bypass".into(),
                label: "Chicken vine bypass".into(),
                scope: scope(),
                prerequisites: PredicateExpression::True,
                operations: Vec::new(),
                discharged_obligation_ids: Vec::new(),
                introduced_obligation_ids: Vec::new(),
                cost: RouteCost {
                    axes: BTreeMap::new(),
                },
                evidence: evidence(),
            },
            Technique {
                id: "technique.ordinary-rod-quest".into(),
                label: "Ordinary rod quest".into(),
                scope: scope(),
                prerequisites: PredicateExpression::True,
                operations: Vec::new(),
                discharged_obligation_ids: Vec::new(),
                introduced_obligation_ids: Vec::new(),
                cost: RouteCost {
                    axes: BTreeMap::new(),
                },
                evidence: evidence(),
            },
        ],
        microtraces: Vec::new(),
        goals: vec![Goal {
            id: "goal.obtain-fishing-rod".into(),
            label: "Obtain Fishing Rod".into(),
            predicate: PredicateExpression::Fact {
                fact_id: "inventory.fishing-rod".into(),
            },
        }],
    };
    (facts, mechanics)
}

fn route_book_fixture() -> RouteBook {
    RouteBook {
        schema: ROUTE_BOOK_SCHEMA.into(),
        manifest: RouteBookManifest {
            id: "route-book.rod-research".into(),
            version: "1.0.0".into(),
            label: "Fishing Rod research".into(),
            author: "Route researchers".into(),
            source: "Curated route references".into(),
            scope: scope(),
            refinement_stack_sha256: None,
        },
        goal_ids: vec!["goal.obtain-fishing-rod".into()],
        constraints: Vec::new(),
        directives: Vec::new(),
        steps: vec![
            ReferenceStep {
                id: "step.chicken-bypass".into(),
                label: "Bypass vine man with chicken".into(),
                scope: scope(),
                action: RouteActionRef::Technique {
                    technique_id: "technique.chicken-bypass".into(),
                },
                precondition: None,
                postcondition: None,
                region_id: Some("region.obtain-rod".into()),
                annotation_ids: Vec::new(),
            },
            ReferenceStep {
                id: "step.ordinary-quest".into(),
                label: "Complete ordinary rod quest".into(),
                scope: scope(),
                action: RouteActionRef::Technique {
                    technique_id: "technique.ordinary-rod-quest".into(),
                },
                precondition: None,
                postcondition: Some(PredicateExpression::Fact {
                    fact_id: "inventory.fishing-rod".into(),
                }),
                region_id: Some("region.obtain-rod".into()),
                annotation_ids: Vec::new(),
            },
        ],
        methods: vec![
            PlanMethod {
                id: "method.chicken-mix".into(),
                label: "Chicken bypass plus ordinary finish".into(),
                scope: scope(),
                region_id: "region.obtain-rod".into(),
                step_ids: vec!["step.chicken-bypass".into(), "step.ordinary-quest".into()],
            },
            PlanMethod {
                id: "method.ordinary".into(),
                label: "Ordinary quest".into(),
                scope: scope(),
                region_id: "region.obtain-rod".into(),
                step_ids: vec!["step.ordinary-quest".into()],
            },
        ],
        regions: vec![PlanRegion {
            id: "region.obtain-rod".into(),
            label: "Obtain Fishing Rod".into(),
            scope: scope(),
            parent_region_id: None,
            entry_predicate: None,
            outcome_predicate: PredicateExpression::Fact {
                fact_id: "inventory.fishing-rod".into(),
            },
            method_ids: vec!["method.chicken-mix".into(), "method.ordinary".into()],
            selected_method_id: None,
            collapse_policy: CollapsePolicy::OnlyContinuationEquivalent,
        }],
        annotations: Vec::new(),
    }
}

#[test]
fn route_book_collapses_interchangeable_methods_without_authoring_effects() {
    let (facts, mechanics) = catalogs();
    let book = route_book_fixture();
    book.validate_against(&facts, &mechanics).unwrap();
    assert_eq!(book.regions[0].method_ids.len(), 2);
    assert_eq!(
        book.regions[0].collapse_policy,
        CollapsePolicy::OnlyContinuationEquivalent
    );
    let bytes = book.canonical_bytes().unwrap();
    assert_eq!(RouteBook::decode_canonical(&bytes).unwrap(), book);
}

#[test]
fn unknown_actions_fail_against_catalog_without_becoming_mechanics() {
    let (facts, mechanics) = catalogs();
    let mut book = route_book_fixture();
    book.steps[0].action = RouteActionRef::Technique {
        technique_id: "technique.imaginary".into(),
    };
    assert_eq!(
        book.validate_against(&facts, &mechanics)
            .unwrap_err()
            .field(),
        "action.technique_id"
    );
}

#[test]
fn region_cycles_and_zero_weight_preferences_fail_closed() {
    let mut book = route_book_fixture();
    book.regions[0].parent_region_id = Some("region.obtain-rod".into());
    assert_eq!(
        book.validate().unwrap_err().field(),
        "regions.parent_region_id"
    );

    let mut book = route_book_fixture();
    book.directives.push(RouteDirective {
        id: "directive.prefer".into(),
        scope: scope(),
        directive: RouteDirectiveKind::PreferMethod {
            method_id: "method.ordinary".into(),
            weight: 0,
        },
    });
    assert_eq!(book.validate().unwrap_err().field(), "directives.weight");
}

#[test]
fn step_context_cannot_leak_an_action_into_an_unsupported_build() {
    let (facts, mechanics) = catalogs();
    let mut book = route_book_fixture();
    book.manifest
        .scope
        .selectors
        .extend(alternate_scope().selectors);
    book.regions[0].scope = book.manifest.scope.clone();
    book.methods.remove(0);
    book.regions[0].method_ids = vec!["method.ordinary".into()];
    book.steps[0].scope = alternate_scope();
    book.validate().unwrap();
    assert_eq!(
        book.validate_against(&facts, &mechanics)
            .unwrap_err()
            .field(),
        "steps.scope"
    );
}

#[test]
fn revision_checked_edit_batches_are_atomic_and_revalidated() {
    let (facts, mechanics) = catalogs();
    let book = route_book_fixture();
    let batch = RouteBookEditBatch {
        schema: ROUTE_BOOK_EDIT_BATCH_SCHEMA.into(),
        expected_route_book_sha256: book.digest().unwrap(),
        edits: vec![
            RouteBookEdit::SetSelectedMethod {
                region_id: "region.obtain-rod".into(),
                method_id: Some("method.ordinary".into()),
            },
            RouteBookEdit::SetCollapsePolicy {
                region_id: "region.obtain-rod".into(),
                collapse_policy: CollapsePolicy::ShowResidualDifferences,
            },
        ],
    };
    let edited = batch.apply(&book, &facts, &mechanics).unwrap();
    assert_eq!(
        edited.regions[0].selected_method_id.as_deref(),
        Some("method.ordinary")
    );
    assert_eq!(
        edited.regions[0].collapse_policy,
        CollapsePolicy::ShowResidualDifferences
    );
    assert_ne!(edited.digest().unwrap(), book.digest().unwrap());

    let mut stale = batch;
    stale.expected_route_book_sha256 = Digest([9; 32]);
    assert_eq!(
        stale.apply(&book, &facts, &mechanics).unwrap_err().field(),
        "expected_route_book_sha256"
    );
}

#[test]
fn invalid_edit_batch_does_not_partially_mutate_the_source_book() {
    let (facts, mechanics) = catalogs();
    let book = route_book_fixture();
    let original_digest = book.digest().unwrap();
    let batch = RouteBookEditBatch {
        schema: ROUTE_BOOK_EDIT_BATCH_SCHEMA.into(),
        expected_route_book_sha256: original_digest,
        edits: vec![RouteBookEdit::RemoveStep {
            step_id: "step.ordinary-quest".into(),
        }],
    };
    assert_eq!(
        batch.apply(&book, &facts, &mechanics).unwrap_err().field(),
        "methods.step_ids"
    );
    assert_eq!(book.digest().unwrap(), original_digest);
}
