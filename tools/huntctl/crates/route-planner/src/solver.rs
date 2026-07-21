//! Bounded forward state search with explicit feasibility choices and proofs.

use crate::evaluation::{
    EvaluatedTruth, EvidencePolicy, FeasibilityMode, FeasibilitySelection, PredicateEvaluator,
    RuleClassification, TransitionClassification,
};
use crate::execution::PlannerExecutionState;
use crate::identity::EquivalenceSet;
use crate::logic::{FactCatalog, PredicateExpression};
use crate::transition::{MechanicsCatalog, StateOperation};
use crate::{PlannerContractError, artifact::Digest};
use std::collections::{BTreeSet, VecDeque};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SolverOptions {
    pub max_depth: usize,
    pub max_states: usize,
    pub max_resolution_combinations: usize,
    pub feasibility_mode: FeasibilityMode,
    pub evidence_policy: EvidencePolicy,
}

impl Default for SolverOptions {
    fn default() -> Self {
        Self {
            max_depth: 128,
            max_states: 100_000,
            max_resolution_combinations: 256,
            feasibility_mode: FeasibilityMode::Modeled,
            evidence_policy: EvidencePolicy::ESTABLISHED_ONLY,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SearchStatus {
    Reached,
    UnreachableUnderModel,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SearchActionKind {
    Transition,
    Technique,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchStep {
    pub action_kind: SearchActionKind,
    pub action_id: String,
    pub selected_resolver_ids: Vec<String>,
    pub selected_technique_ids: Vec<String>,
    pub source_state_sha256: Digest,
    pub result_state_sha256: Digest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchResult {
    pub status: SearchStatus,
    pub steps: Vec<SearchStep>,
    pub explored_states: usize,
    pub hit_search_limit: bool,
    pub unknown_transition_ids: Vec<String>,
    pub execution_error_ids: Vec<String>,
}

struct SearchNode {
    state: PlannerExecutionState,
    steps: Vec<SearchStep>,
    depth: usize,
}

pub struct ForwardSolver<'a> {
    facts: &'a FactCatalog,
    mechanics: &'a MechanicsCatalog,
    equivalence_sets: &'a [EquivalenceSet],
    options: SolverOptions,
}

impl<'a> ForwardSolver<'a> {
    pub fn new(
        facts: &'a FactCatalog,
        mechanics: &'a MechanicsCatalog,
        equivalence_sets: &'a [EquivalenceSet],
        options: SolverOptions,
    ) -> Result<Self, PlannerContractError> {
        facts.validate()?;
        mechanics.validate()?;
        for set in equivalence_sets {
            set.validate()?;
        }
        if options.max_depth == 0
            || options.max_states == 0
            || options.max_resolution_combinations == 0
        {
            return Err(PlannerContractError::new(
                "solver.options",
                "all search bounds must be nonzero",
            ));
        }
        Ok(Self {
            facts,
            mechanics,
            equivalence_sets,
            options,
        })
    }

    pub fn solve(
        &self,
        start: PlannerExecutionState,
        goal: &PredicateExpression,
    ) -> Result<SearchResult, PlannerContractError> {
        start.validate()?;
        let mut queue = VecDeque::from([SearchNode {
            state: start,
            steps: Vec::new(),
            depth: 0,
        }]);
        let mut visited = BTreeSet::new();
        let mut unknown_transition_ids = BTreeSet::new();
        let mut execution_error_ids = BTreeSet::new();
        let mut saw_unknown_goal = false;
        let mut hit_search_limit = false;
        let mut generated_id = 0_u64;

        while let Some(node) = queue.pop_front() {
            let state_identity = node.state.semantic_digest()?;
            if !visited.insert(state_identity) {
                continue;
            }
            if visited.len() > self.options.max_states {
                hit_search_limit = true;
                break;
            }

            let evaluator = PredicateEvaluator::new(
                &node.state.snapshot,
                self.facts,
                self.equivalence_sets,
                &node.state.gate_states,
                self.options.evidence_policy,
            )?;
            match evaluator.evaluate(goal) {
                EvaluatedTruth::True => {
                    return Ok(SearchResult {
                        status: SearchStatus::Reached,
                        steps: node.steps,
                        explored_states: visited.len(),
                        hit_search_limit: false,
                        unknown_transition_ids: unknown_transition_ids.into_iter().collect(),
                        execution_error_ids: execution_error_ids.into_iter().collect(),
                    });
                }
                EvaluatedTruth::Unknown => saw_unknown_goal = true,
                EvaluatedTruth::False => {}
            }
            if node.depth >= self.options.max_depth {
                if !self.mechanics.transitions.is_empty() || !self.mechanics.techniques.is_empty() {
                    hit_search_limit = true;
                }
                continue;
            }

            // Techniques with concrete state operations are also standalone
            // actions. Their obligation annotations are action-local and are
            // considered separately when combining a technique with a target
            // transition below.
            for technique in &self.mechanics.techniques {
                if evaluator.assess_technique(technique).classification
                    != RuleClassification::Active
                    || technique.operations.is_empty()
                {
                    continue;
                }
                let mut next = node.state.clone();
                generated_id = generated_id.saturating_add(1);
                if next
                    .apply_operations(
                        &technique.id,
                        &format!("search-state-{generated_id}"),
                        &technique.operations,
                    )
                    .is_err()
                {
                    execution_error_ids.insert(technique.id.clone());
                    continue;
                }
                self.enqueue_if_new(
                    &mut queue,
                    &visited,
                    &node,
                    next,
                    SearchStep {
                        action_kind: SearchActionKind::Technique,
                        action_id: technique.id.clone(),
                        selected_resolver_ids: Vec::new(),
                        selected_technique_ids: vec![technique.id.clone()],
                        source_state_sha256: state_identity,
                        result_state_sha256: Digest::ZERO,
                    },
                )?;
            }

            for transition in &self.mechanics.transitions {
                let applicable_resolver_ids = self
                    .mechanics
                    .resolvers
                    .iter()
                    .filter(|resolver| {
                        evaluator.assess_resolver(resolver).classification
                            == RuleClassification::Active
                    })
                    .filter(|resolver| {
                        self.mechanics.obstructions.iter().any(|obstruction| {
                            obstruction.id == resolver.obstruction_id
                                && obstruction.blocked_action_id == transition.id
                                && obstruction.approach_id == transition.approach_id
                        })
                    })
                    .map(|resolver| resolver.id.clone())
                    .collect::<Vec<_>>();
                let applicable_technique_ids =
                    self.mechanics
                        .techniques
                        .iter()
                        .filter(|technique| {
                            evaluator.assess_technique(technique).classification
                                == RuleClassification::Active
                        })
                        .filter(|technique| {
                            technique.discharged_obligation_ids.iter().any(|id| {
                                transition.activation.physical_obligation_ids.contains(id)
                            })
                        })
                        .map(|technique| technique.id.clone())
                        .collect::<Vec<_>>();
                let resolver_selections = bounded_subsets(
                    &applicable_resolver_ids,
                    self.options.max_resolution_combinations,
                );
                let technique_selections = bounded_subsets(
                    &applicable_technique_ids,
                    self.options.max_resolution_combinations,
                );
                let mut combinations = 0_usize;
                for selected_resolvers in &resolver_selections {
                    for selected_techniques in &technique_selections {
                        combinations += 1;
                        if combinations > self.options.max_resolution_combinations {
                            hit_search_limit = true;
                            break;
                        }
                        let resolution = evaluator.resolve_feasibility(
                            transition,
                            &self.mechanics.obstructions,
                            &self.mechanics.resolvers,
                            &self.mechanics.techniques,
                            FeasibilitySelection {
                                resolver_ids: selected_resolvers,
                                technique_ids: selected_techniques,
                                already_discharged: &BTreeSet::new(),
                            },
                        );
                        let unresolved_active_obstruction = resolution
                            .active_obstruction_ids
                            .iter()
                            .any(|obstruction_id| {
                                !self.mechanics.resolvers.iter().any(|resolver| {
                                    resolver.obstruction_id == *obstruction_id
                                        && resolution.applied_resolver_ids.contains(&resolver.id)
                                })
                            });
                        if self.options.feasibility_mode == FeasibilityMode::Modeled
                            && (!resolution.unknown_obstruction_ids.is_empty()
                                || unresolved_active_obstruction)
                        {
                            if !resolution.unknown_obstruction_ids.is_empty() {
                                unknown_transition_ids.insert(transition.id.clone());
                            }
                            continue;
                        }

                        let mut setup_operations = Vec::new();
                        append_selected_resolver_operations(
                            &mut setup_operations,
                            &self.mechanics.resolvers,
                            selected_resolvers,
                        );
                        append_selected_technique_operations(
                            &mut setup_operations,
                            &self.mechanics.techniques,
                            selected_techniques,
                        );
                        let mut next = node.state.clone();
                        if !setup_operations.is_empty() {
                            generated_id = generated_id.saturating_add(1);
                            if next
                                .apply_operations(
                                    &format!("setup.{}", transition.id),
                                    &format!("search-setup-{generated_id}"),
                                    &setup_operations,
                                )
                                .is_err()
                            {
                                execution_error_ids.insert(transition.id.clone());
                                continue;
                            }
                        }
                        let assessment = PredicateEvaluator::new(
                            &next.snapshot,
                            self.facts,
                            self.equivalence_sets,
                            &next.gate_states,
                            self.options.evidence_policy,
                        )?
                        .assess_transition(
                            transition,
                            &resolution.discharged_obligation_ids,
                            self.options.feasibility_mode,
                        );
                        match assessment.classification {
                            TransitionClassification::Executable => {}
                            TransitionClassification::FeasibilityUnknown => {
                                unknown_transition_ids.insert(transition.id.clone());
                                continue;
                            }
                            TransitionClassification::Inapplicable
                            | TransitionClassification::GuardBlocked
                            | TransitionClassification::Obstructed => continue,
                        }

                        generated_id = generated_id.saturating_add(1);
                        if next
                            .apply_operations(
                                &transition.id,
                                &format!("search-state-{generated_id}"),
                                &transition.activation.effects,
                            )
                            .is_err()
                        {
                            execution_error_ids.insert(transition.id.clone());
                            continue;
                        }
                        self.enqueue_if_new(
                            &mut queue,
                            &visited,
                            &node,
                            next,
                            SearchStep {
                                action_kind: SearchActionKind::Transition,
                                action_id: transition.id.clone(),
                                selected_resolver_ids: selected_resolvers.iter().cloned().collect(),
                                selected_technique_ids: selected_techniques
                                    .iter()
                                    .cloned()
                                    .collect(),
                                source_state_sha256: state_identity,
                                result_state_sha256: Digest::ZERO,
                            },
                        )?;
                    }
                    if combinations > self.options.max_resolution_combinations {
                        break;
                    }
                }
            }
        }

        let unknown = hit_search_limit
            || saw_unknown_goal
            || !unknown_transition_ids.is_empty()
            || !execution_error_ids.is_empty();
        Ok(SearchResult {
            status: if unknown {
                SearchStatus::Unknown
            } else {
                SearchStatus::UnreachableUnderModel
            },
            steps: Vec::new(),
            explored_states: visited.len(),
            hit_search_limit,
            unknown_transition_ids: unknown_transition_ids.into_iter().collect(),
            execution_error_ids: execution_error_ids.into_iter().collect(),
        })
    }

    fn enqueue_if_new(
        &self,
        queue: &mut VecDeque<SearchNode>,
        visited: &BTreeSet<Digest>,
        node: &SearchNode,
        next: PlannerExecutionState,
        mut step: SearchStep,
    ) -> Result<(), PlannerContractError> {
        let result = next.semantic_digest()?;
        if visited.contains(&result) {
            return Ok(());
        }
        step.result_state_sha256 = result;
        let mut steps = node.steps.clone();
        steps.push(step);
        queue.push_back(SearchNode {
            state: next,
            steps,
            depth: node.depth + 1,
        });
        Ok(())
    }
}

fn bounded_subsets(ids: &[String], maximum: usize) -> Vec<BTreeSet<String>> {
    let mut subsets = vec![BTreeSet::new()];
    for id in ids {
        let additions = subsets
            .iter()
            .take(maximum.saturating_sub(subsets.len()))
            .cloned()
            .map(|mut subset| {
                subset.insert(id.clone());
                subset
            })
            .collect::<Vec<_>>();
        subsets.extend(additions);
        if subsets.len() >= maximum {
            subsets.truncate(maximum);
            break;
        }
    }
    subsets
}

fn append_selected_resolver_operations(
    operations: &mut Vec<StateOperation>,
    resolvers: &[crate::transition::ObstructionResolver],
    selected: &BTreeSet<String>,
) {
    for resolver in resolvers
        .iter()
        .filter(|resolver| selected.contains(&resolver.id))
    {
        operations.extend(resolver.operations.iter().cloned());
    }
}

fn append_selected_technique_operations(
    operations: &mut Vec<StateOperation>,
    techniques: &[crate::transition::Technique],
    selected: &BTreeSet<String>,
) {
    for technique in techniques
        .iter()
        .filter(|technique| selected.contains(&technique.id))
    {
        operations.extend(technique.operations.iter().cloned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact::Digest;
    use crate::identity::{ContextSelector, RUNTIME_CONFIGURATION_SCHEMA, RuntimeConfiguration};
    use crate::logic::{
        ComparisonOperator, ContextScope, EvidenceKind, EvidenceRecord, FACT_CATALOG_SCHEMA,
        RuleEvidence, TruthStatus, ValueReference,
    };
    use crate::snapshot::{STATE_SNAPSHOT_SCHEMA, StateSnapshot};
    use crate::state::{
        BackingAttachment, EXECUTION_ENVIRONMENT_SCHEMA, ExecutionEnvironment, PlayerForm,
        PlayerState, RuntimeFile, RuntimeFileLifecycle, RuntimeFileOrigin, SceneLocation,
        StateValue,
    };
    use crate::transition::{
        ActivationContract, CandidateTransition, FeasibilityObligation, MECHANICS_CATALOG_SCHEMA,
        MechanicsCatalog, ObligationDetail, ObligationKind, Obstruction, ObstructionResolver,
        ResolutionKind, TransitionKind,
    };
    use std::collections::BTreeMap;

    fn snapshot() -> StateSnapshot {
        StateSnapshot {
            schema: STATE_SNAPSHOT_SCHEMA.into(),
            id: "snapshot.start".into(),
            sequence: 1,
            environment: ExecutionEnvironment {
                schema: EXECUTION_ENVIRONMENT_SCHEMA.into(),
                runtime_configuration: RuntimeConfiguration {
                    schema: RUNTIME_CONFIGURATION_SCHEMA.into(),
                    content_sha256: Digest([1; 32]),
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
                physical_slots: Vec::new(),
                physical_slot_observations: Vec::new(),
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
                    rotation: [0; 3],
                    has_control: Some(true),
                    action: "idle".into(),
                },
                components: Vec::new(),
                static_world_objects: Vec::new(),
                persisted_object_controls: Vec::new(),
                live_world_objects: Vec::new(),
            },
            semantic_observations: Vec::new(),
        }
    }

    fn scope(snapshot: &StateSnapshot) -> ContextScope {
        ContextScope {
            selectors: vec![ContextSelector::Exact {
                context: snapshot
                    .environment
                    .runtime_configuration
                    .exact_context()
                    .unwrap(),
            }],
        }
    }

    fn evidence(truth: TruthStatus) -> RuleEvidence {
        RuleEvidence {
            truth,
            records: if matches!(truth, TruthStatus::Established | TruthStatus::Contested) {
                vec![EvidenceRecord {
                    id: "source.solver-test".into(),
                    kind: EvidenceKind::SourceAudited,
                    source_sha256: Some(Digest([2; 32])),
                    note: "Solver test evidence.".into(),
                }]
            } else {
                Vec::new()
            },
        }
    }

    fn stage_is(stage: &str) -> PredicateExpression {
        PredicateExpression::Compare {
            left: ValueReference::LocationStage,
            operator: ComparisonOperator::Equal,
            right: ValueReference::Literal {
                value: StateValue::Text(stage.into()),
            },
        }
    }

    fn transition(
        snapshot: &StateSnapshot,
        id: &str,
        source: &str,
        destination: &str,
        obligations: Vec<String>,
    ) -> CandidateTransition {
        CandidateTransition {
            id: id.into(),
            label: format!("Travel from {source} to {destination}"),
            scope: scope(snapshot),
            transition_kind: TransitionKind::EncodedMapExit,
            approach_id: "approach.front".into(),
            activation: ActivationContract {
                hard_guards: stage_is(source),
                physical_obligation_ids: obligations,
                effects: vec![StateOperation::SetLocation {
                    location: SceneLocation {
                        stage: destination.into(),
                        room: 0,
                        layer: 0,
                        spawn: 0,
                    },
                }],
                unknown_requirements: Vec::new(),
            },
            evidence: evidence(TruthStatus::Established),
        }
    }

    fn catalog(transitions: Vec<CandidateTransition>) -> MechanicsCatalog {
        MechanicsCatalog {
            schema: MECHANICS_CATALOG_SCHEMA.into(),
            transitions,
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
        }
    }

    fn facts() -> FactCatalog {
        FactCatalog {
            schema: FACT_CATALOG_SCHEMA.into(),
            aliases: Vec::new(),
            derived_facts: Vec::new(),
        }
    }

    #[test]
    fn forward_search_reaches_a_goal_and_retains_the_transition_proof() {
        let snapshot = snapshot();
        let mechanics = catalog(vec![
            transition(
                &snapshot,
                "transition.a-to-b",
                "STAGE_A",
                "STAGE_B",
                Vec::new(),
            ),
            transition(
                &snapshot,
                "transition.b-to-c",
                "STAGE_B",
                "STAGE_C",
                Vec::new(),
            ),
        ]);
        let facts = facts();
        let solver = ForwardSolver::new(&facts, &mechanics, &[], SolverOptions::default()).unwrap();
        let result = solver
            .solve(
                PlannerExecutionState::new(snapshot).unwrap(),
                &stage_is("STAGE_C"),
            )
            .unwrap();
        assert_eq!(result.status, SearchStatus::Reached);
        assert_eq!(
            result
                .steps
                .iter()
                .map(|step| step.action_id.as_str())
                .collect::<Vec<_>>(),
            vec!["transition.a-to-b", "transition.b-to-c"]
        );
        assert_ne!(
            result.steps[0].source_state_sha256,
            result.steps[0].result_state_sha256
        );
    }

    #[test]
    fn missing_guard_state_returns_unknown_instead_of_unreachable() {
        let snapshot = snapshot();
        let mut candidate = transition(
            &snapshot,
            "transition.unknown",
            "STAGE_A",
            "STAGE_B",
            Vec::new(),
        );
        candidate.activation.hard_guards = PredicateExpression::Compare {
            left: ValueReference::ComponentField {
                component_id: "missing.component".into(),
                field: "flag".into(),
            },
            operator: ComparisonOperator::Equal,
            right: ValueReference::Literal {
                value: StateValue::Boolean(true),
            },
        };
        let mechanics = catalog(vec![candidate]);
        let facts = facts();
        let solver = ForwardSolver::new(&facts, &mechanics, &[], SolverOptions::default()).unwrap();
        let result = solver
            .solve(
                PlannerExecutionState::new(snapshot).unwrap(),
                &stage_is("STAGE_B"),
            )
            .unwrap();
        assert_eq!(result.status, SearchStatus::Unknown);
        assert_eq!(result.unknown_transition_ids, vec!["transition.unknown"]);
    }

    #[test]
    fn exhausted_known_graph_returns_unreachable_under_the_model() {
        let snapshot = snapshot();
        let mechanics = catalog(Vec::new());
        let facts = facts();
        let solver = ForwardSolver::new(&facts, &mechanics, &[], SolverOptions::default()).unwrap();
        let result = solver
            .solve(
                PlannerExecutionState::new(snapshot).unwrap(),
                &stage_is("STAGE_B"),
            )
            .unwrap();
        assert_eq!(result.status, SearchStatus::UnreachableUnderModel);
        assert!(!result.hit_search_limit);
    }

    #[test]
    fn resolver_choice_is_applied_to_the_specific_obstructed_edge() {
        let snapshot = snapshot();
        let mut mechanics = catalog(vec![transition(
            &snapshot,
            "transition.a-to-b",
            "STAGE_A",
            "STAGE_B",
            vec!["obligation.blocker".into()],
        )]);
        mechanics.transitions[0].activation.hard_guards = PredicateExpression::All {
            terms: vec![
                stage_is("STAGE_A"),
                PredicateExpression::Compare {
                    left: ValueReference::GateState {
                        gate_id: "gate.entrance-open".into(),
                    },
                    operator: ComparisonOperator::Equal,
                    right: ValueReference::Literal {
                        value: StateValue::Boolean(true),
                    },
                },
            ],
        };
        mechanics.obligations = vec![FeasibilityObligation {
            id: "obligation.blocker".into(),
            label: "Reach the loading zone past the blocker".into(),
            scope: scope(&snapshot),
            obligation_kind: ObligationKind::Geometry,
            detail: ObligationDetail::Geometry {
                approach_id: "approach.front".into(),
                source_region_id: "region.a".into(),
                destination_region_id: "region.exit".into(),
            },
            evidence: evidence(TruthStatus::Established),
        }];
        mechanics.obstructions = vec![Obstruction {
            id: "obstruction.npc".into(),
            label: "NPC blocks the transition".into(),
            scope: scope(&snapshot),
            blocked_action_id: "transition.a-to-b".into(),
            approach_id: "approach.front".into(),
            active_when: PredicateExpression::True,
            obligation_ids: vec!["obligation.blocker".into()],
            evidence: evidence(TruthStatus::Established),
        }];
        mechanics.resolvers = vec![ObstructionResolver {
            id: "resolver.text-state".into(),
            label: "Use displaced text state".into(),
            scope: scope(&snapshot),
            obstruction_id: "obstruction.npc".into(),
            resolution_kind: ResolutionKind::Bypass,
            applicable_when: PredicateExpression::True,
            operations: vec![StateOperation::SetGate {
                gate_id: "gate.entrance-open".into(),
            }],
            evidence: evidence(TruthStatus::Established),
        }];
        let facts = facts();
        let solver = ForwardSolver::new(&facts, &mechanics, &[], SolverOptions::default()).unwrap();
        let result = solver
            .solve(
                PlannerExecutionState::new(snapshot).unwrap(),
                &stage_is("STAGE_B"),
            )
            .unwrap();
        assert_eq!(result.status, SearchStatus::Reached);
        assert_eq!(
            result.steps[0].selected_resolver_ids,
            vec!["resolver.text-state"]
        );
    }
}
