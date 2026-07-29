use super::*;
use crate::graph::{PlannerGraph, PlannerGraphRelation};
use crate::identity::{ContextSelector, ExactContext};
use crate::logic::{EvidenceKind, EvidenceRecord, FACT_CATALOG_SCHEMA, TruthStatus};
use crate::transition::{MECHANICS_CATALOG_SCHEMA, ObligationDetail, ObligationKind};

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

fn evidence(truth: TruthStatus) -> RuleEvidence {
    RuleEvidence {
        truth,
        records: vec![EvidenceRecord {
            id: "source.refinement".into(),
            kind: EvidenceKind::Theorycraft,
            source_sha256: Some(Digest([3; 32])),
            note: "Explicit theorycraft assumption.".into(),
        }],
    }
}

fn pack(id: &str, precedence: i32, operation: RefinementOperation) -> RefinementPack {
    RefinementPack {
        schema: REFINEMENT_PACK_SCHEMA.into(),
        manifest: RefinementPackManifest {
            id: id.into(),
            version: "1.0.0".into(),
            author: "Route research".into(),
            source: "Local theorycraft".into(),
            scope: scope(),
            precedence,
            dependencies: Vec::new(),
            conflicts: Vec::new(),
        },
        rules: vec![RefinementRule {
            id: format!("{id}.rule"),
            label: "Test rule".into(),
            operation,
            evidence: evidence(TruthStatus::Hypothetical),
        }],
    }
}

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

fn rule(id: &str, operation: RefinementOperation) -> RefinementRule {
    RefinementRule {
        id: id.into(),
        label: format!("Rule {id}"),
        operation,
        evidence: evidence(TruthStatus::Hypothetical),
    }
}

fn map_transition(id: &str, source_stage: &str, destination_stage: &str) -> CandidateTransition {
    CandidateTransition {
        id: id.into(),
        label: format!("{source_stage} to {destination_stage}"),
        scope: scope(),
        transition_kind: TransitionKind::EncodedMapExit,
        approach_id: format!("approach.{id}"),
        activation: crate::transition::ActivationContract {
            hard_guards: PredicateExpression::All {
                terms: vec![
                    PredicateExpression::Compare {
                        left: ValueReference::LocationStage,
                        operator: ComparisonOperator::Equal,
                        right: ValueReference::Literal {
                            value: StateValue::Text(source_stage.into()),
                        },
                    },
                    PredicateExpression::Compare {
                        left: ValueReference::LocationRoom,
                        operator: ComparisonOperator::Equal,
                        right: ValueReference::Literal {
                            value: StateValue::Signed(0),
                        },
                    },
                ],
            },
            physical_obligation_ids: vec!["obligation.reach-exit".into()],
            effects: vec![StateOperation::SetLocation {
                location: SceneLocation {
                    stage: destination_stage.into(),
                    room: 1,
                    layer: 0,
                    spawn: 2,
                },
            }],
            unknown_requirements: Vec::new(),
        },
        evidence: evidence(TruthStatus::Established),
    }
}

fn exit_obligation() -> FeasibilityObligation {
    FeasibilityObligation {
        id: "obligation.reach-exit".into(),
        label: "Reach the exit".into(),
        scope: scope(),
        obligation_kind: ObligationKind::Geometry,
        stage: crate::transition::ObligationStage::Reach,
        detail: ObligationDetail::Unresolved {
            research_question: "Can the exit be reached?".into(),
        },
        evidence: evidence(TruthStatus::Established),
    }
}

fn bound_obstruction(cardinality: MatchCardinality) -> AuthoredObstruction {
    AuthoredObstruction {
        id: "obstruction.bound-wall".into(),
        label: "Bound wall".into(),
        scope: scope(),
        action_selector: ObstructionActionSelector::Transition {
            transition_kind: Some(TransitionKind::EncodedMapExit),
            approach_id: None,
            source: None,
            destination: Some(SceneLocationSelector {
                stage: Some("DEST".into()),
                room: Some(1),
                layer: None,
                spawn: None,
            }),
        },
        match_cardinality: cardinality,
        active_when: PredicateExpression::True,
        obligation_ids: vec!["obligation.reach-exit".into()],
        evidence: evidence(TruthStatus::Established),
    }
}

#[test]
fn theorycraft_absence_is_explicit_and_remains_hypothetical() {
    let pack = pack(
        "what-if.no-wall",
        50,
        RefinementOperation::AssumeObstructionAbsent {
            obstruction_id: "obstruction.ordon-wall".into(),
            when: PredicateExpression::True,
        },
    );
    pack.validate().unwrap();
    assert_eq!(pack.rules[0].evidence.truth, TruthStatus::Hypothetical);
    assert_ne!(pack.digest().unwrap(), Digest::ZERO);
}

#[test]
fn stack_precedence_is_deterministic_independent_of_input_order() {
    let low = pack(
        "community.base",
        10,
        RefinementOperation::SuppressWriter {
            writer_id: "writer.savmem".into(),
            when: PredicateExpression::False,
        },
    );
    let high = pack(
        "route.local",
        100,
        RefinementOperation::AssumeObstructionAbsent {
            obstruction_id: "obstruction.wall".into(),
            when: PredicateExpression::True,
        },
    );
    let first = RefinementStack::build(&[high.clone(), low.clone()]).unwrap();
    let second = RefinementStack::build(&[low, high]).unwrap();
    assert_eq!(first, second);
    assert_eq!(first.entries[0].pack_id, "community.base");
}

#[test]
fn conflicts_and_dependency_digest_mismatches_fail_closed() {
    let mut left = pack(
        "left",
        1,
        RefinementOperation::SuppressWriter {
            writer_id: "writer.a".into(),
            when: PredicateExpression::True,
        },
    );
    let right = pack(
        "right",
        2,
        RefinementOperation::SuppressWriter {
            writer_id: "writer.b".into(),
            when: PredicateExpression::True,
        },
    );
    left.manifest.conflicts = vec!["right".into()];
    assert_eq!(
        RefinementStack::build(&[left.clone(), right.clone()])
            .unwrap_err()
            .field(),
        "manifest.conflicts"
    );

    left.manifest.conflicts.clear();
    left.manifest.dependencies = vec![PackDependency {
        pack_id: "right".into(),
        pack_sha256: Digest([9; 32]),
    }];
    assert_eq!(
        RefinementStack::build(&[left, right]).unwrap_err().field(),
        "manifest.dependencies"
    );
}

#[test]
fn canonical_decode_rejects_browser_or_editor_junk_fields() {
    let pack = pack(
        "clean",
        1,
        RefinementOperation::AssumeObstructionAbsent {
            obstruction_id: "obstruction.wall".into(),
            when: PredicateExpression::True,
        },
    );
    let bytes = pack.canonical_bytes().unwrap();
    assert_eq!(RefinementPack::decode_canonical(&bytes).unwrap(), pack);
    let mut value = serde_json::to_value(pack).unwrap();
    value["browser_only"] = serde_json::json!(true);
    assert!(serde_json::from_value::<RefinementPack>(value).is_err());
}

#[test]
fn composed_catalog_accepts_only_additive_ephemeral_editor_overlays() {
    let (facts, mechanics) = empty_catalogs();
    let base = ComposedPlannerCatalog::compose(&facts, &mechanics, &[]).unwrap();
    let first = pack(
        "what-if.rebind",
        1_000,
        RefinementOperation::ComponentTransform {
            prerequisite: PredicateExpression::True,
            operations: vec![StateOperation::Preserve {
                selector: crate::state::ComponentSelector::Id {
                    component_id: "component.stage-bank".into(),
                },
            }],
        },
    );
    let extended = base
        .extend_ephemeral_what_if(std::slice::from_ref(&first))
        .unwrap();
    assert!(base.mechanics.techniques.is_empty());
    assert_eq!(extended.mechanics.techniques[0].id, "what-if.rebind.rule");
    assert_eq!(
        extended.refinement_stack.entries[0].layer,
        RefinementLayer::EphemeralWhatIf
    );

    let mut second = pack(
        "what-if.copy",
        1_001,
        RefinementOperation::ComponentTransform {
            prerequisite: PredicateExpression::True,
            operations: vec![StateOperation::Preserve {
                selector: crate::state::ComponentSelector::Id {
                    component_id: "component.session".into(),
                },
            }],
        },
    );
    second.manifest.dependencies.push(PackDependency {
        pack_id: first.manifest.id.clone(),
        pack_sha256: first.digest().unwrap(),
    });
    assert_eq!(
        extended
            .extend_ephemeral_what_if(std::slice::from_ref(&second))
            .unwrap()
            .mechanics
            .techniques
            .len(),
        2
    );

    let forbidden = pack(
        "what-if.writer",
        1_002,
        RefinementOperation::SuppressWriter {
            writer_id: "writer.any".into(),
            when: PredicateExpression::True,
        },
    );
    assert_eq!(
        extended
            .extend_ephemeral_what_if(&[forbidden])
            .unwrap_err()
            .field(),
        "rules.operation"
    );
}

#[test]
fn composition_compiles_obstructions_transforms_and_writer_suppression() {
    let (facts, mut mechanics) = empty_catalogs();
    mechanics.writers.push(WriterRule {
        id: "writer.savmem".into(),
        scope: scope(),
        activation: PredicateExpression::True,
        operation: StateOperation::SetGate {
            gate_id: "gate.return-place".into(),
        },
        evidence: evidence(TruthStatus::Established),
    });
    let pack = RefinementPack {
        schema: REFINEMENT_PACK_SCHEMA.into(),
        manifest: RefinementPackManifest {
            id: "research.ordon-wall".into(),
            version: "1.0.0".into(),
            author: "Route research".into(),
            source: "Local theorycraft".into(),
            scope: scope(),
            precedence: 10,
            dependencies: Vec::new(),
            conflicts: Vec::new(),
        },
        rules: vec![
            rule(
                "a.obligation",
                RefinementOperation::AddObligation {
                    obligation: FeasibilityObligation {
                        id: "obligation.reach-wall".into(),
                        label: "Reach the far side of the wall".into(),
                        scope: scope(),
                        obligation_kind: ObligationKind::Geometry,
                        stage: crate::transition::ObligationStage::Reach,
                        detail: ObligationDetail::Unresolved {
                            research_question: "Can the wall be crossed?".into(),
                        },
                        evidence: evidence(TruthStatus::Established),
                    },
                },
            ),
            rule(
                "b.obstruction",
                RefinementOperation::AddObstruction {
                    obstruction: Obstruction {
                        id: "obstruction.ordon-wall".into(),
                        label: "Ordon wall".into(),
                        scope: scope(),
                        blocked_action_id: "transition.ordon-return".into(),
                        approach_id: "approach.ordon-wall".into(),
                        active_when: PredicateExpression::True,
                        obligation_ids: vec!["obligation.reach-wall".into()],
                        evidence: evidence(TruthStatus::Established),
                    },
                },
            ),
            rule(
                "c.assume-absent",
                RefinementOperation::AssumeObstructionAbsent {
                    obstruction_id: "obstruction.ordon-wall".into(),
                    when: PredicateExpression::True,
                },
            ),
            rule(
                "d.component-transform",
                RefinementOperation::ComponentTransform {
                    prerequisite: PredicateExpression::True,
                    operations: vec![StateOperation::SetGate {
                        gate_id: "gate.what-if-transfer".into(),
                    }],
                },
            ),
            rule(
                "e.suppress-writer",
                RefinementOperation::SuppressWriter {
                    writer_id: "writer.savmem".into(),
                    when: PredicateExpression::True,
                },
            ),
        ],
    };

    let composed = ComposedPlannerCatalog::compose(&facts, &mechanics, &[pack]).unwrap();
    assert_eq!(composed.mechanics.obligations.len(), 1);
    assert_eq!(composed.mechanics.obstructions.len(), 1);
    assert_eq!(composed.mechanics.resolvers.len(), 1);
    assert_eq!(
        composed.mechanics.resolvers[0].resolution_kind,
        ResolutionKind::AssumeAbsent
    );
    assert_eq!(composed.mechanics.techniques.len(), 1);
    assert_eq!(
        composed.mechanics.gates[0].blocked_writer_ids,
        ["writer.savmem"]
    );
    let bytes = composed.canonical_bytes().unwrap();
    assert_eq!(
        ComposedPlannerCatalog::decode_canonical(&bytes).unwrap(),
        composed
    );
}

#[test]
fn authored_obstruction_selector_binds_and_projects_the_block_dependency() {
    let (facts, mut mechanics) = empty_catalogs();
    mechanics.obligations.push(exit_obligation());
    mechanics
        .transitions
        .push(map_transition("transition.a", "SOURCE_A", "DEST"));
    let mut obstruction = bound_obstruction(MatchCardinality::ExactlyOne);
    obstruction.action_selector = ObstructionActionSelector::Transition {
        transition_kind: Some(TransitionKind::EncodedMapExit),
        approach_id: None,
        source: Some(SceneLocationSelector {
            stage: Some("SOURCE_A".into()),
            room: Some(0),
            layer: None,
            spawn: None,
        }),
        destination: Some(SceneLocationSelector {
            stage: Some("DEST".into()),
            room: Some(1),
            layer: None,
            spawn: None,
        }),
    };
    let pack = RefinementPack {
        schema: REFINEMENT_PACK_SCHEMA.into(),
        manifest: RefinementPackManifest {
            id: "binding.wall".into(),
            version: "1.0.0".into(),
            author: "Route research".into(),
            source: "Local theorycraft".into(),
            scope: scope(),
            precedence: 1,
            dependencies: Vec::new(),
            conflicts: Vec::new(),
        },
        rules: vec![
            rule(
                "a.bind",
                RefinementOperation::BindObstruction { obstruction },
            ),
            rule(
                "b.resolve",
                RefinementOperation::AssumeObstructionAbsent {
                    obstruction_id: "obstruction.bound-wall".into(),
                    when: PredicateExpression::True,
                },
            ),
        ],
    };

    let composed =
        ComposedPlannerCatalog::compose(&facts, &mechanics, std::slice::from_ref(&pack)).unwrap();
    assert_eq!(composed.mechanics.obstructions.len(), 1);
    assert_eq!(
        composed.mechanics.obstructions[0].blocked_action_id,
        "transition.a"
    );
    assert_eq!(
        composed.mechanics.obstructions[0].approach_id,
        "approach.transition.a"
    );
    assert_eq!(
        composed.mechanics.resolvers[0].obstruction_id,
        "obstruction.bound-wall"
    );
    assert_eq!(composed.obstruction_bindings.len(), 1);
    assert_eq!(
        composed.obstruction_bindings[0].source_pack_id,
        "binding.wall"
    );
    assert_eq!(composed.obstruction_bindings[0].source_rule_id, "a.bind");
    let graph = PlannerGraph::project_composed(&composed).unwrap();
    assert!(graph.edges.iter().any(|edge| {
        edge.source_node_id == "obstruction/obstruction.bound-wall"
            && edge.target_node_id == "transition/transition.a"
            && edge.relation == PlannerGraphRelation::Blocks
    }));

    mechanics.transitions.clear();
    let error = ComposedPlannerCatalog::compose(&facts, &mechanics, &[pack]).unwrap_err();
    assert_eq!(error.field(), "rules.obstruction.action_selector");
    assert!(error.detail().contains("matched no candidate actions"));
}

#[test]
fn plural_obstruction_selector_expands_bindings_and_resolvers_deterministically() {
    let (facts, mut mechanics) = empty_catalogs();
    mechanics.obligations.push(exit_obligation());
    mechanics
        .transitions
        .push(map_transition("transition.a", "SOURCE_A", "DEST"));
    mechanics
        .transitions
        .push(map_transition("transition.b", "SOURCE_B", "DEST"));
    let plural_pack = RefinementPack {
        schema: REFINEMENT_PACK_SCHEMA.into(),
        manifest: RefinementPackManifest {
            id: "binding.plural-wall".into(),
            version: "1.0.0".into(),
            author: "Route research".into(),
            source: "Local theorycraft".into(),
            scope: scope(),
            precedence: 1,
            dependencies: Vec::new(),
            conflicts: Vec::new(),
        },
        rules: vec![
            rule(
                "a.bind",
                RefinementOperation::BindObstruction {
                    obstruction: bound_obstruction(MatchCardinality::OneOrMore),
                },
            ),
            rule(
                "b.resolve",
                RefinementOperation::AssumeObstructionAbsent {
                    obstruction_id: "obstruction.bound-wall".into(),
                    when: PredicateExpression::True,
                },
            ),
        ],
    };

    let first =
        ComposedPlannerCatalog::compose(&facts, &mechanics, std::slice::from_ref(&plural_pack))
            .unwrap();
    let second =
        ComposedPlannerCatalog::compose(&facts, &mechanics, std::slice::from_ref(&plural_pack))
            .unwrap();
    assert_eq!(first, second);
    assert_eq!(first.mechanics.obstructions.len(), 2);
    assert_eq!(first.mechanics.resolvers.len(), 2);
    assert_eq!(first.obstruction_bindings.len(), 2);
    assert!(
        first
            .mechanics
            .obstructions
            .iter()
            .all(|record| record.id.starts_with("binding.obstruction."))
    );
    for obstruction in &first.mechanics.obstructions {
        assert!(first.mechanics.resolvers.iter().any(|resolver| {
            resolver.obstruction_id == obstruction.id
                && resolver.id.starts_with("binding.resolver.")
        }));
    }

    let singular_pack = pack(
        "binding.ambiguous-wall",
        1,
        RefinementOperation::BindObstruction {
            obstruction: bound_obstruction(MatchCardinality::ExactlyOne),
        },
    );
    let error = ComposedPlannerCatalog::compose(&facts, &mechanics, &[singular_pack]).unwrap_err();
    assert!(error.detail().contains("expected exactly one"));
}

#[test]
fn duplicate_additions_require_an_explicit_replacement() {
    let (facts, mut mechanics) = empty_catalogs();
    let writer = WriterRule {
        id: "writer.savmem".into(),
        scope: scope(),
        activation: PredicateExpression::True,
        operation: StateOperation::SetGate {
            gate_id: "gate.return-place".into(),
        },
        evidence: evidence(TruthStatus::Established),
    };
    mechanics.writers.push(writer.clone());
    let duplicate = pack(
        "duplicate.writer",
        10,
        RefinementOperation::AddWriter { writer },
    );
    assert_eq!(
        ComposedPlannerCatalog::compose(&facts, &mechanics, &[duplicate])
            .unwrap_err()
            .field(),
        "writers"
    );
}

#[test]
fn replacement_and_disable_precedence_is_deterministic() {
    let (facts, mut mechanics) = empty_catalogs();
    mechanics.goals.push(Goal {
        id: "goal.original".into(),
        label: "Original goal".into(),
        predicate: PredicateExpression::True,
    });
    let replacement = RefinementPack {
        schema: REFINEMENT_PACK_SCHEMA.into(),
        manifest: RefinementPackManifest {
            id: "replace.goal".into(),
            version: "1.0.0".into(),
            author: "Route research".into(),
            source: "Local theorycraft".into(),
            scope: scope(),
            precedence: 10,
            dependencies: Vec::new(),
            conflicts: Vec::new(),
        },
        rules: vec![
            rule(
                "a.replace",
                RefinementOperation::ReplaceRecord {
                    target_id: "goal.original".into(),
                    replacement_kind: ReplacementKind::Replace,
                    replacement_rule_id: Some("b.goal".into()),
                },
            ),
            rule(
                "b.goal",
                RefinementOperation::AddGoal {
                    goal: Goal {
                        id: "goal.replacement".into(),
                        label: "Replacement goal".into(),
                        predicate: PredicateExpression::True,
                    },
                },
            ),
        ],
    };
    let disable = pack(
        "disable.goal",
        20,
        RefinementOperation::ReplaceRecord {
            target_id: "goal.replacement".into(),
            replacement_kind: ReplacementKind::Disable,
            replacement_rule_id: None,
        },
    );

    let first = ComposedPlannerCatalog::compose(
        &facts,
        &mechanics,
        &[disable.clone(), replacement.clone()],
    )
    .unwrap();
    let second =
        ComposedPlannerCatalog::compose(&facts, &mechanics, &[replacement, disable]).unwrap();
    assert_eq!(first, second);
    assert!(first.mechanics.goals.is_empty());
}

#[test]
fn route_local_and_ephemeral_layers_override_precedence_and_remove_cleanly() {
    let (facts, mut mechanics) = empty_catalogs();
    mechanics.goals.push(Goal {
        id: "goal.base".into(),
        label: "Base goal".into(),
        predicate: PredicateExpression::True,
    });
    let replacement_pack = |pack_id: &str, precedence: i32, from: &str, to: &str| RefinementPack {
        schema: REFINEMENT_PACK_SCHEMA.into(),
        manifest: RefinementPackManifest {
            id: pack_id.into(),
            version: "1.0.0".into(),
            author: "Route research".into(),
            source: "Layering regression fixture".into(),
            scope: scope(),
            precedence,
            dependencies: Vec::new(),
            conflicts: Vec::new(),
        },
        rules: vec![
            rule(
                &format!("{pack_id}.a-replace"),
                RefinementOperation::ReplaceRecord {
                    target_id: from.into(),
                    replacement_kind: ReplacementKind::Replace,
                    replacement_rule_id: Some(format!("{pack_id}.b-goal")),
                },
            ),
            rule(
                &format!("{pack_id}.b-goal"),
                RefinementOperation::AddGoal {
                    goal: Goal {
                        id: to.into(),
                        label: format!("Goal from {pack_id}"),
                        predicate: PredicateExpression::True,
                    },
                },
            ),
        ],
    };
    let enabled = replacement_pack("enabled.goal", 10_000, "goal.base", "goal.enabled");
    let mut route = replacement_pack("route.goal", -10_000, "goal.enabled", "goal.route");
    route.manifest.dependencies = vec![PackDependency {
        pack_id: enabled.manifest.id.clone(),
        pack_sha256: enabled.digest().unwrap(),
    }];
    let ephemeral = replacement_pack("what-if.goal", -20_000, "goal.route", "goal.what-if");
    let layers = RefinementLayers {
        enabled_packs: vec![enabled.clone()],
        route_local_overlays: vec![route.clone()],
        ephemeral_what_if_overlays: vec![ephemeral.clone()],
    };
    let composed = ComposedPlannerCatalog::compose_layered(&facts, &mechanics, &layers).unwrap();
    assert_eq!(
        composed
            .refinement_stack
            .entries
            .iter()
            .map(|entry| (entry.layer, entry.pack_id.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (RefinementLayer::EnabledPack, "enabled.goal"),
            (RefinementLayer::RouteLocal, "route.goal"),
            (RefinementLayer::EphemeralWhatIf, "what-if.goal"),
        ]
    );
    assert_eq!(composed.mechanics.goals[0].id, "goal.what-if");

    let without_what_if = ComposedPlannerCatalog::compose_layered(
        &facts,
        &mechanics,
        &RefinementLayers {
            enabled_packs: vec![enabled.clone()],
            route_local_overlays: vec![route],
            ephemeral_what_if_overlays: Vec::new(),
        },
    )
    .unwrap();
    assert_eq!(without_what_if.mechanics.goals[0].id, "goal.route");
    let enabled_only = ComposedPlannerCatalog::compose(&facts, &mechanics, &[enabled]).unwrap();
    assert_eq!(enabled_only.mechanics.goals[0].id, "goal.enabled");

    let mut invalid_dependency =
        replacement_pack("enabled.depends-on-what-if", 0, "goal.base", "goal.invalid");
    invalid_dependency.manifest.dependencies = vec![PackDependency {
        pack_id: ephemeral.manifest.id.clone(),
        pack_sha256: ephemeral.digest().unwrap(),
    }];
    let error = RefinementStack::build_layered(&RefinementLayers {
        enabled_packs: vec![invalid_dependency],
        route_local_overlays: Vec::new(),
        ephemeral_what_if_overlays: vec![ephemeral],
    })
    .unwrap_err();
    assert_eq!(error.field(), "manifest.dependencies");
    assert!(error.detail().contains("later-layer"));
}

#[test]
fn diagnostics_accumulate_rule_shape_dependency_and_conflict_fixes() {
    let operation = RefinementOperation::AddGoal {
        goal: Goal {
            id: "goal.diagnostic".into(),
            label: "Diagnostic goal".into(),
            predicate: PredicateExpression::True,
        },
    };
    let mut malformed = pack("diagnostic.malformed", 0, operation.clone());
    malformed.schema = "old-schema".into();
    let mut duplicate = malformed.rules[0].clone();
    duplicate.label.clear();
    malformed.rules.push(duplicate);
    malformed.manifest.dependencies.push(PackDependency {
        pack_id: "diagnostic.missing".into(),
        pack_sha256: Digest([7; 32]),
    });
    let mut conflicting = pack("diagnostic.conflicting", 1, operation);
    conflicting.manifest.conflicts = vec![malformed.manifest.id.clone()];

    let report = diagnose_refinement_packs(&[malformed, conflicting]);
    assert!(!report.valid);
    assert!(report.diagnostics.len() >= 5);
    assert!(report.diagnostics.iter().any(|row| row.field == "schema"));
    assert!(
        report
            .diagnostics
            .iter()
            .any(|row| row.detail.contains("duplicate rule ID"))
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|row| row.detail.contains("missing valid pack"))
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|row| row.detail.contains("conflict"))
    );
    assert!(
        report
            .diagnostics
            .iter()
            .all(|row| !row.suggestion.is_empty())
    );
}
