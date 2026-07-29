
use super::*;
use crate::identity::{
    ContextSelector, ExactContext, RUNTIME_CONFIGURATION_SCHEMA, RuntimeConfiguration,
};
use crate::logic::{
    ContextScope, DerivedFact, EvidenceKind, EvidenceRecord, FACT_CATALOG_SCHEMA, RuleEvidence,
    TruthStatus,
};
use crate::relevance::{BACKWARD_RELEVANCE_SCHEMA, BackwardRelevance};
use crate::route_book::{
    CollapsePolicy, PlanMethod, PlanRegion, ROUTE_BOOK_SCHEMA, ReferenceStep, RouteActionRef,
    RouteBook, RouteBookManifest,
};
use crate::snapshot::{STATE_SNAPSHOT_SCHEMA, StateSnapshot};
use crate::solver::{ContinuationIdentity, ContinuationMergeProof};
use crate::state::{
    BackingAttachment, EXECUTION_ENVIRONMENT_SCHEMA, ExecutionEnvironment, PlayerForm, PlayerState,
    RuntimeFile, RuntimeFileLifecycle, RuntimeFileOrigin, SceneLocation, SpatialConnection,
    SpatialConnectionStatus,
};
use crate::transition::{
    ActivationContract, CandidateTransition, FeasibilityObligation, Goal, MECHANICS_CATALOG_SCHEMA,
    MechanicsCatalog, ObligationDetail, ObligationKind, RouteCost, StateOperation, Technique,
    TemporalRequirement, TemporalWindow, TransitionKind, WitnessedMicrotrace,
};

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

fn evidence() -> RuleEvidence {
    RuleEvidence {
        truth: TruthStatus::Established,
        records: vec![EvidenceRecord {
            id: "source.graph-test".into(),
            kind: EvidenceKind::SourceAudited,
            source_sha256: Some(Digest([3; 32])),
            note: "Graph projection test evidence.".into(),
        }],
    }
}

fn catalogs() -> (FactCatalog, MechanicsCatalog) {
    let facts = FactCatalog {
        schema: FACT_CATALOG_SCHEMA.into(),
        aliases: Vec::new(),
        derived_facts: vec![
            DerivedFact {
                id: "fact.can-return".into(),
                label: "Can return to Ordon".into(),
                scope: scope(),
                rule: PredicateExpression::All {
                    terms: vec![
                        PredicateExpression::Fact {
                            fact_id: "fact.has-route".into(),
                        },
                        PredicateExpression::True,
                    ],
                },
                evidence: evidence(),
            },
            DerivedFact {
                id: "fact.has-route".into(),
                label: "Has a route".into(),
                scope: scope(),
                rule: PredicateExpression::True,
                evidence: evidence(),
            },
        ],
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
        techniques: vec![Technique {
            id: "technique.ordon-return".into(),
            label: "Return to Ordon".into(),
            scope: scope(),
            prerequisites: PredicateExpression::True,
            operations: Vec::new(),
            discharged_obligation_ids: Vec::new(),
            introduced_obligation_ids: Vec::new(),
            cost: RouteCost {
                axes: BTreeMap::new(),
            },
            evidence: evidence(),
        }],
        microtraces: Vec::new(),
        goals: vec![Goal {
            id: "goal.ordon".into(),
            label: "Reach Ordon".into(),
            predicate: PredicateExpression::Any {
                terms: vec![
                    PredicateExpression::Fact {
                        fact_id: "fact.can-return".into(),
                    },
                    PredicateExpression::False,
                ],
            },
        }],
    };
    (facts, mechanics)
}

fn route_book() -> RouteBook {
    RouteBook {
        schema: ROUTE_BOOK_SCHEMA.into(),
        manifest: RouteBookManifest {
            id: "route-book.ordon".into(),
            version: "1.0.0".into(),
            label: "Ordon routes".into(),
            author: "Route research".into(),
            source: "Graph test".into(),
            scope: scope(),
            refinement_stack_sha256: None,
        },
        goal_ids: vec!["goal.ordon".into()],
        constraints: Vec::new(),
        directives: Vec::new(),
        steps: vec![ReferenceStep {
            id: "step.return".into(),
            label: "Return to Ordon".into(),
            scope: scope(),
            action: RouteActionRef::Technique {
                technique_id: "technique.ordon-return".into(),
            },
            precondition: None,
            postcondition: Some(PredicateExpression::Fact {
                fact_id: "fact.can-return".into(),
            }),
            region_id: Some("region.return".into()),
            annotation_ids: Vec::new(),
        }],
        methods: vec![PlanMethod {
            id: "method.return".into(),
            label: "Known return".into(),
            scope: scope(),
            region_id: "region.return".into(),
            step_ids: vec!["step.return".into()],
        }],
        regions: vec![PlanRegion {
            id: "region.return".into(),
            label: "Reach Ordon".into(),
            scope: scope(),
            parent_region_id: None,
            entry_predicate: None,
            outcome_predicate: PredicateExpression::Fact {
                fact_id: "fact.can-return".into(),
            },
            method_ids: vec!["method.return".into()],
            selected_method_id: Some("method.return".into()),
            collapse_policy: CollapsePolicy::OnlyContinuationEquivalent,
        }],
        annotations: Vec::new(),
    }
}

#[test]
fn projection_preserves_nested_and_or_requirements_in_collapsible_regions() {
    let (facts, mechanics) = catalogs();
    let graph = PlannerGraph::project(&facts, &mechanics).unwrap();
    assert!(graph.nodes.iter().any(|node| matches!(
        node.payload,
        PlannerNodePayload::Predicate {
            operator: PredicateOperator::All
        }
    )));
    assert!(graph.nodes.iter().any(|node| matches!(
        node.payload,
        PlannerNodePayload::Predicate {
            operator: PredicateOperator::Any
        }
    )));
    assert_eq!(
        graph
            .regions
            .iter()
            .filter(|region| region.region_kind == PlannerRegionKind::Predicate)
            .count(),
        4
    );
    assert!(
        graph
            .regions
            .iter()
            .filter(|region| { region.region_kind == PlannerRegionKind::Predicate })
            .all(|region| region.collapsed_by_default)
    );
    assert!(
        !graph
            .nodes
            .iter()
            .any(|node| matches!(node.payload, PlannerNodePayload::ExternalFact { .. }))
    );
    let bytes = graph.canonical_bytes().unwrap();
    assert_eq!(PlannerGraph::decode_canonical(&bytes).unwrap(), graph);
}

#[test]
fn temporal_witness_auto_binds_to_its_exact_obligation_in_the_graph() {
    let (facts, mut mechanics) = catalogs();
    let requirement = TemporalRequirement {
        action_id: "dialogue.auru".into(),
        window: TemporalWindow {
            earliest_frame: 0,
            latest_frame: 1,
            required_input: Some("sidehop".into()),
        },
    };
    mechanics.obligations = vec![FeasibilityObligation {
        id: "obligation.auru-window".into(),
        label: "Interrupt Auru during the item dialogue window".into(),
        scope: scope(),
        obligation_kind: ObligationKind::Timing,
        stage: crate::transition::ObligationStage::Interrupt,
        detail: ObligationDetail::Temporal {
            requirement: requirement.clone(),
            precondition: PredicateExpression::True,
        },
        evidence: evidence(),
    }];
    mechanics.microtraces = vec![WitnessedMicrotrace {
        id: "microtrace.auru-sidehop".into(),
        scope: scope(),
        precondition: PredicateExpression::True,
        operations: vec![StateOperation::Interrupt {
            action_id: requirement.action_id,
            window: TemporalWindow {
                earliest_frame: 1,
                latest_frame: 1,
                required_input: Some("sidehop".into()),
            },
        }],
        postcondition: PredicateExpression::True,
        timing: TemporalWindow {
            earliest_frame: 1,
            latest_frame: 1,
            required_input: Some("sidehop".into()),
        },
        evidence: evidence(),
    }];

    let graph = PlannerGraph::project(&facts, &mechanics).unwrap();
    assert!(graph.edges.iter().any(|edge| {
        edge.source_node_id == "microtrace/microtrace.auru-sidehop"
            && edge.target_node_id == "obligation/obligation.auru-window"
            && edge.relation == PlannerGraphRelation::Demonstrates
    }));

    mechanics.microtraces[0].scope = ContextScope {
        selectors: vec![ContextSelector::Exact {
            context: ExactContext {
                content_sha256: Digest([9; 32]),
                runtime_configuration_sha256: Digest([8; 32]),
            },
        }],
    };
    let disjoint = PlannerGraph::project(&facts, &mechanics).unwrap();
    assert!(!disjoint.edges.iter().any(|edge| {
        edge.source_node_id == "microtrace/microtrace.auru-sidehop"
            && edge.target_node_id == "obligation/obligation.auru-window"
            && edge.relation == PlannerGraphRelation::Demonstrates
    }));
}

#[test]
fn feasibility_diff_separates_authorized_obstructed_and_unknown_edges() {
    let mut snapshot = StateSnapshot {
        schema: STATE_SNAPSHOT_SCHEMA.into(),
        id: "snapshot.graph-diff".into(),
        sequence: 1,
        environment: ExecutionEnvironment {
            schema: EXECUTION_ENVIRONMENT_SCHEMA.into(),
            runtime_configuration: RuntimeConfiguration {
                schema: RUNTIME_CONFIGURATION_SCHEMA.into(),
                content_sha256: Digest([4; 32]),
                language: "en".into(),
                settings: BTreeMap::new(),
            },
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
            execution_context: crate::state::ExecutionContext::World,
            location: SceneLocation {
                stage: "STAGE_A".into(),
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
            components: Vec::new(),
            static_world_objects: Vec::new(),
            spatial_volumes: Vec::new(),
            spatial_connections: vec![SpatialConnection {
                approach_id: "approach.front".into(),
                source_region_id: "region.before-wall".into(),
                destination_region_id: "region.exit".into(),
                status: SpatialConnectionStatus::Blocked,
                source_sha256: Digest([5; 32]),
            }],
            spatial_planes: Vec::new(),
            persisted_object_controls: Vec::new(),
            live_world_objects: Vec::new(),
        },
        semantic_observations: Vec::new(),
    };
    let exact_scope = ContextScope {
        selectors: vec![ContextSelector::Exact {
            context: snapshot
                .environment
                .runtime_configuration
                .exact_context()
                .unwrap(),
        }],
    };
    let facts = FactCatalog {
        schema: FACT_CATALOG_SCHEMA.into(),
        aliases: Vec::new(),
        derived_facts: Vec::new(),
    };
    let obligation = FeasibilityObligation {
        id: "obligation.wall".into(),
        label: "Reach the exit past the wall".into(),
        scope: exact_scope.clone(),
        obligation_kind: ObligationKind::Geometry,
        stage: crate::transition::ObligationStage::Reach,
        detail: ObligationDetail::Geometry {
            approach_id: "approach.front".into(),
            source_region_id: "region.before-wall".into(),
            destination_region_id: "region.exit".into(),
        },
        evidence: evidence(),
    };
    let transition = CandidateTransition {
        id: "transition.exit".into(),
        label: "Use the exit behind the wall".into(),
        scope: exact_scope,
        transition_kind: TransitionKind::EncodedMapExit,
        approach_id: "approach.front".into(),
        activation: ActivationContract {
            hard_guards: PredicateExpression::True,
            physical_obligation_ids: vec![obligation.id.clone()],
            effects: Vec::new(),
            unknown_requirements: Vec::new(),
        },
        evidence: evidence(),
    };
    let mechanics = MechanicsCatalog {
        schema: MECHANICS_CATALOG_SCHEMA.into(),
        transitions: vec![transition],
        obligations: vec![obligation],
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

    let blocked_state = PlannerExecutionState::new(snapshot.clone()).unwrap();
    let blocked = PlannerFeasibilityGraphDiff::project(
        &blocked_state,
        &facts,
        &mechanics,
        &[],
        EvidencePolicy::ESTABLISHED_ONLY,
    )
    .unwrap();
    assert_eq!(blocked.transitions.len(), 1);
    assert_eq!(
        blocked.transitions[0].upper_bound.classification,
        TransitionClassification::Executable
    );
    assert_eq!(
        blocked.transitions[0].modeled.classification,
        TransitionClassification::Obstructed
    );
    let canonical = blocked.canonical_bytes().unwrap();
    assert_eq!(
        PlannerFeasibilityGraphDiff::decode_canonical(&canonical).unwrap(),
        blocked
    );
    let mut different_gate_state = blocked_state.clone();
    different_gate_state
        .gate_states
        .insert("gate.unrelated".into(), true);
    let gated = PlannerFeasibilityGraphDiff::project(
        &different_gate_state,
        &facts,
        &mechanics,
        &[],
        EvidencePolicy::ESTABLISHED_ONLY,
    )
    .unwrap();
    assert_ne!(gated.execution_state_sha256, blocked.execution_state_sha256);

    snapshot.environment.spatial_connections.clear();
    let unknown_state = PlannerExecutionState::new(snapshot).unwrap();
    let unknown = PlannerFeasibilityGraphDiff::project(
        &unknown_state,
        &facts,
        &mechanics,
        &[],
        EvidencePolicy::ESTABLISHED_ONLY,
    )
    .unwrap();
    assert_eq!(
        unknown.transitions[0].modeled.classification,
        TransitionClassification::FeasibilityUnknown
    );
    assert_eq!(
        unknown.transitions[0].modeled.unknown_obligation_ids,
        vec!["obligation.wall"]
    );
}

#[test]
fn projection_is_deterministic_and_does_not_use_browser_state() {
    let (facts, mechanics) = catalogs();
    let first = PlannerGraph::project(&facts, &mechanics).unwrap();
    let second = PlannerGraph::project(&facts, &mechanics).unwrap();
    assert_eq!(first.digest().unwrap(), second.digest().unwrap());
    assert!(first.nodes.windows(2).all(|pair| pair[0].id < pair[1].id));
    assert!(first.edges.windows(2).all(|pair| pair[0].id < pair[1].id));
}

#[test]
fn route_book_projects_as_nested_preferences_without_replacing_mechanics() {
    let (facts, mechanics) = catalogs();
    let book = route_book();
    let graph = PlannerGraph::project_with_route_book(&facts, &mechanics, &book).unwrap();
    assert_eq!(graph.route_book_sha256, Some(book.digest().unwrap()));
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| matches!(node.payload, PlannerNodePayload::PlanRegion { .. }))
    );
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| matches!(node.payload, PlannerNodePayload::PlanMethod { .. }))
    );
    assert!(graph.edges.iter().any(|edge| {
        edge.relation == PlannerGraphRelation::SelectsAction
            && edge.target_node_id == "technique/technique.ordon-return"
    }));
    assert!(graph.edges.iter().any(|edge| {
        edge.relation == PlannerGraphRelation::Selected
            && edge.target_node_id == "plan-method/method.return"
    }));
    assert!(
        graph
            .regions
            .iter()
            .filter(|region| {
                region.region_kind == PlannerRegionKind::Plan && region.id != "region.plans"
            })
            .all(|region| !region.collapsed_by_default)
    );
}

#[test]
fn solver_proof_regions_collapse_only_safe_continuations_and_keep_residuals() {
    let (facts, mechanics) = catalogs();
    let mut graph = PlannerGraph::project(&facts, &mechanics).unwrap();
    let initial = Digest([10; 32]);
    let reached = Digest([11; 32]);
    let residual = Digest([12; 32]);
    let step = |result_state_sha256| SearchStep {
        action_kind: SearchActionKind::Technique,
        action_id: "technique.ordon-return".into(),
        selected_resolver_ids: Vec::new(),
        selected_technique_ids: Vec::new(),
        active_obstruction_ids: Vec::new(),
        unknown_obstruction_ids: Vec::new(),
        discharged_obligation_ids: Vec::new(),
        outstanding_obligation_ids: Vec::new(),
        unknown_obligation_ids: Vec::new(),
        supporting_microtrace_ids: Vec::new(),
        introduced_obligation_ids: Vec::new(),
        reader_results: Vec::new(),
        unknown_reader_ids: Vec::new(),
        evidence_dependencies: Vec::new(),
        weakest_evidence: Some(TruthStatus::Established),
        action_derivations: Vec::new(),
        obligation_derivations: Vec::new(),
        source_state_sha256: initial,
        result_state_sha256,
    };
    let continuation = |state_sha256| ContinuationIdentity {
        state_sha256,
        satisfied_required_actions: Vec::new(),
        required_sequence_progress: Vec::new(),
        banned_sequence_progress: Vec::new(),
        preferred_sequence_progress: Vec::new(),
        satisfied_preference_ids: Vec::new(),
        route_condition_unknown: false,
    };
    let alternative = |result_state_sha256| SearchPlan {
        result_state_sha256,
        continuation: continuation(result_state_sha256),
        steps: vec![step(result_state_sha256)],
        preference_score: 0,
        satisfied_preference_ids: Vec::new(),
        route_costs: BTreeMap::new(),
    };
    let mut continuation_distinct = alternative(reached);
    continuation_distinct.continuation.banned_sequence_progress = vec![1];
    let result = SearchResult {
        backward_relevance: BackwardRelevance {
            schema: BACKWARD_RELEVANCE_SCHEMA.into(),
            dependencies: Vec::new(),
            frontier_dependencies: Vec::new(),
            transition_ids: Vec::new(),
            writer_ids: Vec::new(),
            technique_ids: vec!["technique.ordon-return".into()],
            obstruction_ids: Vec::new(),
            resolver_ids: Vec::new(),
            obligation_ids: Vec::new(),
            gate_ids: Vec::new(),
            reader_ids: Vec::new(),
            reconstruction_rule_ids: Vec::new(),
            microtrace_ids: Vec::new(),
        },
        backward_pruning_applied: true,
        status: SearchStatus::Reached,
        steps: vec![step(reached)],
        explored_states: 4,
        hit_search_limit: false,
        preference_score: 0,
        satisfied_preference_ids: Vec::new(),
        route_costs: BTreeMap::new(),
        result_continuation: Some(continuation(reached)),
        alternative_plans: vec![
            alternative(reached),
            continuation_distinct,
            alternative(residual),
        ],
        minimum_evidence: Some(TruthStatus::Established),
        unknown_transition_ids: Vec::new(),
        unknown_writer_ids: Vec::new(),
        execution_error_ids: Vec::new(),
        blocked_transition_witnesses: Vec::new(),
        blocked_writer_witnesses: Vec::new(),
        blocked_technique_witnesses: Vec::new(),
        blocked_resolver_witnesses: Vec::new(),
        blocked_reconstruction_witnesses: Vec::new(),
        continuation_merge_proofs: vec![ContinuationMergeProof {
            continuation: ContinuationIdentity {
                state_sha256: reached,
                satisfied_required_actions: Vec::new(),
                required_sequence_progress: Vec::new(),
                banned_sequence_progress: Vec::new(),
                preferred_sequence_progress: Vec::new(),
                satisfied_preference_ids: Vec::new(),
                route_condition_unknown: false,
            },
            dominating: SearchResourceLabel {
                depth: 1,
                route_costs: BTreeMap::new(),
            },
            dominated: SearchResourceLabel {
                depth: 2,
                route_costs: BTreeMap::new(),
            },
        }],
        failed_producer_cuts: Vec::new(),
        failed_producer_cut_sets: Vec::new(),
        failed_producer_cut_sets_complete: true,
    };

    graph.attach_solver_proof(initial, &result).unwrap();
    let equivalent = graph
        .regions
        .iter()
        .find(|region| region.id == "region.proof.plan.alternative-000")
        .unwrap();
    assert!(equivalent.collapsed_by_default);
    assert!(matches!(
        equivalent.collapse_evidence,
        Some(PlannerCollapseEvidence::ContinuationEquivalent { .. })
    ));
    let distinct = graph
        .regions
        .iter()
        .find(|region| region.id == "region.proof.plan.alternative-001")
        .unwrap();
    assert!(!distinct.collapsed_by_default);
    assert!(matches!(
        distinct.collapse_evidence,
        Some(PlannerCollapseEvidence::ResidualDifferences { .. })
    ));
    let Some(PlannerCollapseEvidence::ResidualDifferences { differences, .. }) =
        &distinct.collapse_evidence
    else {
        unreachable!()
    };
    assert!(differences.iter().any(|difference| matches!(
        difference,
        PlannerResidualDifference::BannedSequenceProgress {
            primary,
            alternative
        } if primary.is_empty() && alternative == &[1]
    )));
    assert!(graph.regions.iter().any(|region| {
        region.id == "region.proof.continuation-merges"
            && region.collapsed_by_default
            && matches!(
                region.collapse_evidence,
                Some(PlannerCollapseEvidence::ProvenContinuationMerges { merge_count: 1 })
            )
    }));
    assert!(graph.nodes.iter().any(|node| {
        node.id == "proof-state/alternative-002/0001"
            && matches!(
                node.payload,
                PlannerNodePayload::ProofState { state_sha256, .. }
                    if state_sha256 == residual
            )
    }));
    assert_eq!(
        PlannerGraph::decode_canonical(&graph.canonical_bytes().unwrap()).unwrap(),
        graph
    );
}
