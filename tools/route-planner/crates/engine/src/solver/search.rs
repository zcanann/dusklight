//! Bounded forward-search traversal and result construction.

use super::*;

pub struct ForwardSolver<'a> {
    pub(super) facts: &'a FactCatalog,
    pub(super) mechanics: &'a MechanicsCatalog,
    pub(super) equivalence_sets: &'a [EquivalenceSet],
    pub(super) options: SolverOptions,
    pub(super) route_book: Option<&'a RouteBook>,
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
            route_book: None,
        })
    }

    pub fn new_with_route_book(
        facts: &'a FactCatalog,
        mechanics: &'a MechanicsCatalog,
        equivalence_sets: &'a [EquivalenceSet],
        options: SolverOptions,
        route_book: &'a RouteBook,
    ) -> Result<Self, PlannerContractError> {
        route_book.validate_against(facts, mechanics)?;
        let mut solver = Self::new(facts, mechanics, equivalence_sets, options)?;
        solver.route_book = Some(route_book);
        Ok(solver)
    }

    pub fn solve(
        &self,
        start: PlannerExecutionState,
        goal: &PredicateExpression,
    ) -> Result<SearchResult, PlannerContractError> {
        self.solve_internal(start, goal, &[], None, 1)
    }

    /// Complete the bounded search and return up to `max_plans` deterministic
    /// Pareto plans. Depth and every named route-cost axis are minimized while
    /// route-book preference score is maximized. Incomparable tradeoffs remain
    /// alternatives; strictly dominated goal plans are omitted.
    pub fn solve_alternatives(
        &self,
        start: PlannerExecutionState,
        goal: &PredicateExpression,
        max_plans: usize,
    ) -> Result<SearchResult, PlannerContractError> {
        if max_plans == 0 {
            return Err(PlannerContractError::new(
                "solver.max_plans",
                "must be nonzero",
            ));
        }
        self.solve_internal(start, goal, &[], None, max_plans)
    }

    /// Enumerate the bounded, permissive authorization graph without a goal-
    /// directed slice. Unknown guards, readers, requirements, and disallowed
    /// evidence remain blocked; physical obligations and obstructions are the
    /// only constraints relaxed by upper-bound evaluation.
    pub fn authorization_graph(
        &self,
        start: PlannerExecutionState,
    ) -> Result<AuthorizationGraph, PlannerContractError> {
        if self.options.feasibility_mode != FeasibilityMode::UpperBound {
            return Err(PlannerContractError::new(
                "authorization_graph.feasibility_mode",
                "requires upper-bound evaluation",
            ));
        }
        if self.route_book.is_some() {
            return Err(PlannerContractError::new(
                "authorization_graph.route_book",
                "cannot apply route-specific pruning or preferences",
            ));
        }
        start.validate()?;
        let initial_state_sha256 = start.semantic_digest()?;
        let initial_execution_state_sha256 = start.digest()?;
        let mut action_roots =
            self.mechanics
                .transitions
                .iter()
                .map(|transition| RouteActionRef::Transition {
                    transition_id: transition.id.clone(),
                })
                .chain(
                    self.mechanics
                        .writers
                        .iter()
                        .map(|writer| RouteActionRef::Writer {
                            writer_id: writer.id.clone(),
                        }),
                )
                .chain(self.mechanics.techniques.iter().map(|technique| {
                    RouteActionRef::Technique {
                        technique_id: technique.id.clone(),
                    }
                }))
                .collect::<Vec<_>>();
        action_roots.sort();
        action_roots.dedup();
        let mut recorder = AuthorizationRecorder::default();
        let search = self.solve_internal(
            start,
            &PredicateExpression::False,
            &action_roots,
            Some(&mut recorder),
            1,
        )?;
        let mut equivalence_set_sha256 = self
            .equivalence_sets
            .iter()
            .map(EquivalenceSet::digest)
            .collect::<Result<Vec<_>, _>>()?;
        equivalence_set_sha256.sort();
        equivalence_set_sha256.dedup();
        AuthorizationGraph::finish(
            recorder,
            initial_state_sha256,
            initial_execution_state_sha256,
            self.facts,
            self.mechanics,
            equivalence_set_sha256,
            self.options.evidence_policy,
            self.options,
            &search,
        )
    }

    fn solve_internal(
        &self,
        start: PlannerExecutionState,
        goal: &PredicateExpression,
        additional_action_roots: &[RouteActionRef],
        mut authorization: Option<&mut AuthorizationRecorder>,
        max_plans: usize,
    ) -> Result<SearchResult, PlannerContractError> {
        start.validate()?;
        let initial_state_sha256 = start.semantic_digest()?;
        let start_evaluator = PredicateEvaluator::new(
            &start.snapshot,
            self.facts,
            self.equivalence_sets,
            &start.gate_states,
            self.options.evidence_policy,
        )?;
        let route_policy = self
            .route_book
            .map(|book| compile_route_policy(book, &start_evaluator, self.options.evidence_policy))
            .transpose()?;
        let mut backward_predicate_roots = Vec::new();
        let mut backward_action_roots = BTreeSet::new();
        if let Some(policy) = &route_policy {
            backward_predicate_roots.extend(policy.required_predicates.iter().cloned());
            backward_predicate_roots.extend(policy.maintained_predicates.iter().cloned());
            backward_action_roots.extend(policy.required_actions.iter().cloned());
            for sequence in &policy.required_sequences {
                for step in &sequence.steps {
                    backward_action_roots.insert(step.action.clone());
                    backward_predicate_roots.extend(step.precondition.iter().cloned());
                    backward_predicate_roots.extend(step.postcondition.iter().cloned());
                }
            }
        }
        backward_action_roots.extend(additional_action_roots.iter().cloned());
        let backward_action_roots = backward_action_roots.into_iter().collect::<Vec<_>>();
        let backward_relevance = BackwardRelevance::analyze_with_roots(
            self.facts,
            self.mechanics,
            goal,
            &backward_predicate_roots,
            &backward_action_roots,
        )?;
        let backward_pruning_applied = true;
        let search_evidence_policy = route_policy
            .as_ref()
            .map_or(self.options.evidence_policy, |policy| {
                policy.evidence_policy
            });
        let initial_node = SearchNode {
            state: start,
            steps: Vec::new(),
            depth: 0,
            satisfied_required_actions: BTreeSet::new(),
            required_sequence_progress: vec![
                0;
                route_policy.as_ref().map_or(0, |policy| policy
                    .required_sequences
                    .len())
            ],
            banned_sequence_progress: vec![
                0;
                route_policy
                    .as_ref()
                    .map_or(0, |policy| policy.banned_sequences.len())
            ],
            preferred_sequence_progress: vec![
                0;
                route_policy.as_ref().map_or(0, |policy| policy
                    .method_preferences
                    .len())
            ],
            satisfied_preference_ids: BTreeSet::new(),
            preference_score: 0,
            route_condition_unknown: false,
            route_costs: BTreeMap::new(),
        };
        let mut queue = BinaryHeap::from([QueueEntry {
            node: initial_node,
            insertion_order: 0,
        }]);
        let mut visited = BTreeSet::new();
        let mut resource_frontier =
            BTreeMap::<ContinuationIdentity, Vec<SearchResourceLabel>>::new();
        let mut continuation_merge_proofs = Vec::new();
        let mut unknown_transition_ids = BTreeSet::new();
        let mut unknown_writer_ids = BTreeSet::new();
        let mut execution_error_ids = BTreeSet::new();
        let mut blocked_transition_witnesses = BTreeMap::new();
        let mut blocked_writer_witnesses = BTreeMap::new();
        let mut blocked_technique_witnesses = BTreeMap::<String, BlockedTechniqueWitness>::new();
        let mut blocked_resolver_witnesses = BTreeMap::<String, BlockedResolverWitness>::new();
        let mut blocked_reconstruction_witnesses =
            BTreeMap::<String, BlockedReconstructionWitness>::new();
        let mut executed_actions = BTreeSet::new();
        let mut saw_unknown_goal = false;
        let mut hit_search_limit = false;
        let mut generated_id = 0_u64;
        let mut reached_plans = Vec::new();
        let mut goal_coverage = GoalTruthCoverage::new(goal);

        while let Some(QueueEntry { node, .. }) = queue.pop() {
            let state_identity = node.state.semantic_digest()?;
            let continuation = continuation_identity(&node, state_identity);
            let search_identity = SearchIdentity {
                continuation: continuation.clone(),
                route_costs: node.route_costs.clone(),
            };
            let candidate_resources = resource_label(node.depth, &node.route_costs);
            if let Some(dominating) = resource_frontier
                .get(&continuation)
                .and_then(|labels| {
                    labels
                        .iter()
                        .find(|label| strictly_dominates(label, &candidate_resources))
                })
                .cloned()
            {
                if continuation_merge_proofs.len() == self.options.max_states {
                    hit_search_limit = true;
                    break;
                }
                let proof = ContinuationMergeProof {
                    continuation,
                    dominating,
                    dominated: candidate_resources,
                };
                proof.validate()?;
                continuation_merge_proofs.push(proof);
                continue;
            }
            if visited.contains(&search_identity) {
                continue;
            }
            if visited.len() == self.options.max_states {
                hit_search_limit = true;
                break;
            }
            visited.insert(search_identity);
            let labels = resource_frontier.entry(continuation.clone()).or_default();
            labels.retain(|label| !strictly_dominates(&candidate_resources, label));
            labels.push(candidate_resources);
            if let Some(recorder) = authorization.as_deref_mut() {
                recorder.observe_state(
                    state_identity,
                    node.state.digest()?,
                    node.state.snapshot.digest()?,
                    node.depth,
                    true,
                );
            }

            let evaluator = PredicateEvaluator::new(
                &node.state.snapshot,
                self.facts,
                self.equivalence_sets,
                &node.state.gate_states,
                search_evidence_policy,
            )?;
            if let Some(policy) = &route_policy {
                let mut forbidden = false;
                for predicate in &policy.forbidden_predicates {
                    match evaluator.evaluate(predicate) {
                        EvaluatedTruth::True => {
                            forbidden = true;
                            break;
                        }
                        EvaluatedTruth::Unknown => {
                            saw_unknown_goal = true;
                            forbidden = true;
                            break;
                        }
                        EvaluatedTruth::False => {}
                    }
                }
                if forbidden {
                    continue;
                }
                let mut invariant_failed = false;
                for predicate in &policy.maintained_predicates {
                    match evaluator.evaluate(predicate) {
                        EvaluatedTruth::True => {}
                        EvaluatedTruth::False => {
                            invariant_failed = true;
                            break;
                        }
                        EvaluatedTruth::Unknown => {
                            saw_unknown_goal = true;
                            invariant_failed = true;
                            break;
                        }
                    }
                }
                if invariant_failed {
                    continue;
                }
            }
            let required_predicates = route_policy
                .as_ref()
                .map_or(EvaluatedTruth::True, |policy| {
                    evaluate_all(&evaluator, &policy.required_predicates)
                });
            let required_actions_satisfied = route_policy.as_ref().is_none_or(|policy| {
                policy
                    .required_actions
                    .is_subset(&node.satisfied_required_actions)
            });
            let required_sequences_satisfied = route_policy.as_ref().is_none_or(|policy| {
                policy
                    .required_sequences
                    .iter()
                    .zip(&node.required_sequence_progress)
                    .all(|(sequence, progress)| *progress == sequence.steps.len())
            });
            goal_coverage.observe(&evaluator);
            match evaluator.evaluate(goal) {
                EvaluatedTruth::True
                    if required_predicates == EvaluatedTruth::True
                        && required_actions_satisfied
                        && required_sequences_satisfied
                        && !node.route_condition_unknown =>
                {
                    if max_plans > 1 {
                        let plan = SearchPlan {
                            result_state_sha256: state_identity,
                            continuation: continuation.clone(),
                            steps: node.steps,
                            preference_score: node.preference_score,
                            satisfied_preference_ids: node
                                .satisfied_preference_ids
                                .into_iter()
                                .collect(),
                            route_costs: node.route_costs,
                        };
                        plan.validate()?;
                        retain_nondominated_plan(&mut reached_plans, plan);
                        continue;
                    }
                    return Ok(SearchResult {
                        backward_relevance,
                        backward_pruning_applied,
                        status: SearchStatus::Reached,
                        steps: node.steps,
                        explored_states: visited.len(),
                        hit_search_limit: false,
                        preference_score: node.preference_score,
                        satisfied_preference_ids: node
                            .satisfied_preference_ids
                            .into_iter()
                            .collect(),
                        route_costs: node.route_costs,
                        result_continuation: Some(continuation),
                        alternative_plans: Vec::new(),
                        minimum_evidence: route_policy
                            .as_ref()
                            .and_then(|policy| policy.minimum_evidence),
                        unknown_transition_ids: unknown_transition_ids.into_iter().collect(),
                        unknown_writer_ids: unknown_writer_ids.into_iter().collect(),
                        execution_error_ids: execution_error_ids.into_iter().collect(),
                        blocked_transition_witnesses: Vec::new(),
                        blocked_writer_witnesses: Vec::new(),
                        blocked_technique_witnesses: Vec::new(),
                        blocked_resolver_witnesses: Vec::new(),
                        blocked_reconstruction_witnesses: Vec::new(),
                        continuation_merge_proofs,
                        failed_producer_cuts: Vec::new(),
                        failed_producer_cut_sets: Vec::new(),
                        failed_producer_cut_sets_complete: true,
                    });
                }
                EvaluatedTruth::True => {
                    if required_predicates == EvaluatedTruth::Unknown
                        || node.route_condition_unknown
                    {
                        saw_unknown_goal = true;
                    }
                }
                EvaluatedTruth::Unknown => saw_unknown_goal = true,
                EvaluatedTruth::False => {}
            }
            if node.depth >= self.options.max_depth {
                if (!backward_pruning_applied
                    && (!self.mechanics.transitions.is_empty()
                        || !self.mechanics.techniques.is_empty()
                        || !self.mechanics.writers.is_empty()))
                    || (backward_pruning_applied
                        && (!backward_relevance.transition_ids.is_empty()
                            || !backward_relevance.technique_ids.is_empty()
                            || !backward_relevance.writer_ids.is_empty()))
                {
                    hit_search_limit = true;
                }
                continue;
            }

            // Resolvers execute only as transition setup, but their own
            // applicability can still be the closest reason a state-producing
            // resolver was unavailable. Retain that rule witness independently
            // from the blocked consumer transition.
            for resolver in &self.mechanics.resolvers {
                if backward_pruning_applied
                    && backward_relevance
                        .resolver_ids
                        .binary_search(&resolver.id)
                        .is_err()
                {
                    continue;
                }
                let assessment = evaluator.assess_resolver(resolver);
                if assessment.classification == RuleClassification::Active
                    || resolver.operations.is_empty()
                {
                    continue;
                }
                let evidence_dependencies = resolver_evidence_dependencies(self.facts, resolver);
                let candidate = BlockedResolverWitness {
                    resolver_id: resolver.id.clone(),
                    obstruction_id: resolver.obstruction_id.clone(),
                    source_state_sha256: state_identity,
                    classification: assessment.classification,
                    applicability: assessment.applicability,
                    applicability_expression: resolver.applicable_when.clone(),
                    operations: resolver.operations.clone(),
                    weakest_evidence: weakest_evidence(&evidence_dependencies),
                    evidence_dependencies,
                };
                let replace = blocked_resolver_witnesses
                    .get(&resolver.id)
                    .is_none_or(|current| {
                        rule_blocker_rank(candidate.classification, candidate.source_state_sha256)
                            < rule_blocker_rank(current.classification, current.source_state_sha256)
                    });
                if replace {
                    blocked_resolver_witnesses.insert(resolver.id.clone(), candidate);
                }
            }

            // Reconstruction rules describe actor-instantiation semantics, not
            // freely executable search actions. Preserve their closest rule
            // assessment so a producer cut can name the missing instantiation
            // boundary instead of silently suppressing the explanation.
            for rule in &self.mechanics.reconstruction_rules {
                if backward_pruning_applied
                    && backward_relevance
                        .reconstruction_rule_ids
                        .binary_search(&rule.id)
                        .is_err()
                {
                    continue;
                }
                if rule.initialization_operations.is_empty() {
                    continue;
                }
                let assessment = evaluator.assess_reconstruction(rule);
                let evidence_dependencies = reconstruction_evidence_dependencies(self.facts, rule);
                let candidate = BlockedReconstructionWitness {
                    reconstruction_rule_id: rule.id.clone(),
                    source_state_sha256: state_identity,
                    classification: assessment.classification,
                    activation: assessment.activation,
                    instantiate_when: rule.instantiate_when.clone(),
                    initialization_operations: rule.initialization_operations.clone(),
                    weakest_evidence: weakest_evidence(&evidence_dependencies),
                    evidence_dependencies,
                };
                let replace =
                    blocked_reconstruction_witnesses
                        .get(&rule.id)
                        .is_none_or(|current| {
                            rule_blocker_rank(
                                candidate.classification,
                                candidate.source_state_sha256,
                            ) < rule_blocker_rank(
                                current.classification,
                                current.source_state_sha256,
                            )
                        });
                if replace {
                    blocked_reconstruction_witnesses.insert(rule.id.clone(), candidate);
                }
            }

            // Writer records are standalone engine actions. Their activation
            // and every gate that names them are reevaluated against each
            // concrete state before the operation is applied.
            for writer in &self.mechanics.writers {
                if backward_pruning_applied && !backward_relevance.contains_writer(&writer.id) {
                    continue;
                }
                let action = RouteActionRef::Writer {
                    writer_id: writer.id.clone(),
                };
                if route_policy
                    .as_ref()
                    .is_some_and(|policy| policy.banned_actions.contains(&action))
                {
                    continue;
                }
                let assessment = evaluator.assess_writer(writer, &self.mechanics.gates);
                if assessment.classification != WriterClassification::Executable {
                    if matches!(
                        assessment.classification,
                        WriterClassification::ActivationUnknown | WriterClassification::GateUnknown
                    ) {
                        unknown_writer_ids.insert(writer.id.clone());
                    }
                    if matches!(
                        assessment.classification,
                        WriterClassification::ActivationUnknown
                            | WriterClassification::GateBlocked
                            | WriterClassification::GateUnknown
                    ) {
                        let evidence_dependencies = writer_evidence_dependencies(
                            self.facts,
                            self.mechanics,
                            writer,
                            &assessment,
                        );
                        blocked_writer_witnesses.insert(
                            writer.id.clone(),
                            BlockedWriterWitness {
                                writer_id: writer.id.clone(),
                                source_state_sha256: state_identity,
                                classification: assessment.classification,
                                activation: assessment.activation,
                                active_gate_ids: assessment.active_gate_ids,
                                unknown_gate_ids: assessment.unknown_gate_ids,
                                weakest_evidence: weakest_evidence(&evidence_dependencies),
                                evidence_dependencies,
                                activation_expression: writer.activation.clone(),
                                operation: writer.operation.clone(),
                                gate_derivations: self
                                    .mechanics
                                    .gates
                                    .iter()
                                    .filter(|gate| gate.blocked_writer_ids.contains(&writer.id))
                                    .cloned()
                                    .collect(),
                            },
                        );
                    }
                    continue;
                }
                let mut next = node.state.clone();
                generated_id = generated_id.saturating_add(1);
                if next
                    .apply_operations(
                        &writer.id,
                        &format!("search-state-{generated_id}"),
                        std::slice::from_ref(&writer.operation),
                    )
                    .is_err()
                {
                    execution_error_ids.insert(writer.id.clone());
                    continue;
                }
                let boundary = AppliedActionBoundary {
                    action: action.clone(),
                    before: node.state.clone(),
                    after: next.clone(),
                };
                executed_actions.insert(action);
                let evidence_dependencies =
                    writer_evidence_dependencies(self.facts, self.mechanics, writer, &assessment);
                let weakest_evidence = weakest_evidence(&evidence_dependencies);
                saw_unknown_goal |= self.enqueue_if_new(
                    &mut queue,
                    &visited,
                    &node,
                    next,
                    std::slice::from_ref(&boundary),
                    route_policy.as_ref(),
                    generated_id,
                    authorization.as_deref_mut(),
                    SearchStep {
                        action_kind: SearchActionKind::Writer,
                        action_id: writer.id.clone(),
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
                        evidence_dependencies,
                        weakest_evidence,
                        action_derivations: Vec::new(),
                        obligation_derivations: Vec::new(),
                        source_state_sha256: state_identity,
                        result_state_sha256: Digest::ZERO,
                    },
                )?;
            }

            // Techniques with concrete state operations are also standalone
            // actions. Their obligation annotations are action-local and are
            // considered separately when combining a technique with a target
            // transition below.
            for technique in &self.mechanics.techniques {
                if backward_pruning_applied && !backward_relevance.contains_technique(&technique.id)
                {
                    continue;
                }
                let action = RouteActionRef::Technique {
                    technique_id: technique.id.clone(),
                };
                if route_policy
                    .as_ref()
                    .is_some_and(|policy| policy.banned_actions.contains(&action))
                {
                    continue;
                }
                let assessment = evaluator.assess_technique(technique);
                if assessment.classification != RuleClassification::Active {
                    if !technique.operations.is_empty() {
                        let evidence_dependencies =
                            technique_evidence_dependencies(self.facts, self.mechanics, technique);
                        let candidate = BlockedTechniqueWitness {
                            technique_id: technique.id.clone(),
                            source_state_sha256: state_identity,
                            classification: assessment.classification,
                            prerequisites: assessment.prerequisites,
                            prerequisites_expression: technique.prerequisites.clone(),
                            operations: technique.operations.clone(),
                            weakest_evidence: weakest_evidence(&evidence_dependencies),
                            evidence_dependencies,
                        };
                        let replace =
                            blocked_technique_witnesses
                                .get(&technique.id)
                                .is_none_or(|current| {
                                    rule_blocker_rank(
                                        candidate.classification,
                                        candidate.source_state_sha256,
                                    ) < rule_blocker_rank(
                                        current.classification,
                                        current.source_state_sha256,
                                    )
                                });
                        if replace {
                            blocked_technique_witnesses.insert(technique.id.clone(), candidate);
                        }
                    }
                    continue;
                }
                if technique.operations.is_empty() {
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
                let boundary = AppliedActionBoundary {
                    action: action.clone(),
                    before: node.state.clone(),
                    after: next.clone(),
                };
                executed_actions.insert(action);
                let evidence_dependencies =
                    technique_evidence_dependencies(self.facts, self.mechanics, technique);
                let weakest_evidence = weakest_evidence(&evidence_dependencies);
                saw_unknown_goal |= self.enqueue_if_new(
                    &mut queue,
                    &visited,
                    &node,
                    next,
                    std::slice::from_ref(&boundary),
                    route_policy.as_ref(),
                    generated_id,
                    authorization.as_deref_mut(),
                    SearchStep {
                        action_kind: SearchActionKind::Technique,
                        action_id: technique.id.clone(),
                        selected_resolver_ids: Vec::new(),
                        selected_technique_ids: vec![technique.id.clone()],
                        active_obstruction_ids: Vec::new(),
                        unknown_obstruction_ids: Vec::new(),
                        discharged_obligation_ids: technique.discharged_obligation_ids.clone(),
                        outstanding_obligation_ids: Vec::new(),
                        unknown_obligation_ids: Vec::new(),
                        supporting_microtrace_ids: Vec::new(),
                        introduced_obligation_ids: technique.introduced_obligation_ids.clone(),
                        reader_results: Vec::new(),
                        unknown_reader_ids: Vec::new(),
                        evidence_dependencies,
                        weakest_evidence,
                        action_derivations: Vec::new(),
                        obligation_derivations: Vec::new(),
                        source_state_sha256: state_identity,
                        result_state_sha256: Digest::ZERO,
                    },
                )?;
            }

            for transition in &self.mechanics.transitions {
                if backward_pruning_applied
                    && !backward_relevance.contains_transition(&transition.id)
                {
                    continue;
                }
                let transition_action = RouteActionRef::Transition {
                    transition_id: transition.id.clone(),
                };
                if route_policy
                    .as_ref()
                    .is_some_and(|policy| policy.banned_actions.contains(&transition_action))
                {
                    continue;
                }
                let applicable_resolver_ids = self
                    .mechanics
                    .resolvers
                    .iter()
                    .filter(|resolver| {
                        evaluator.assess_resolver(resolver).classification
                            == RuleClassification::Active
                    })
                    .filter(|resolver| {
                        !route_policy.as_ref().is_some_and(|policy| {
                            policy.banned_actions.contains(&RouteActionRef::Resolver {
                                resolver_id: resolver.id.clone(),
                            })
                        })
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
                            !route_policy.as_ref().is_some_and(|policy| {
                                policy.banned_actions.contains(&RouteActionRef::Technique {
                                    technique_id: technique.id.clone(),
                                })
                            })
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
                        let mut resolution = evaluator.resolve_feasibility(
                            transition,
                            &self.mechanics.obligations,
                            &self.mechanics.obstructions,
                            &self.mechanics.resolvers,
                            &self.mechanics.techniques,
                            FeasibilitySelection {
                                resolver_ids: selected_resolvers,
                                technique_ids: selected_techniques,
                                already_discharged: &BTreeSet::new(),
                                microtraces: &self.mechanics.microtraces,
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
                            let preliminary = evaluator.assess_transition(
                                transition,
                                &resolution.discharged_obligation_ids,
                                &resolution.unknown_obligation_ids,
                                self.options.feasibility_mode,
                            );
                            let (reader_results, unknown_reader_ids) = assess_transition_readers(
                                &evaluator,
                                self.mechanics,
                                &transition.id,
                            );
                            let evidence_dependencies = transition_evidence_dependencies(
                                self.facts,
                                self.mechanics,
                                transition,
                                &resolution,
                                &preliminary,
                                &reader_results,
                                &unknown_reader_ids,
                            );
                            let weakest_evidence = weakest_evidence(&evidence_dependencies);
                            record_blocked_transition_witness(
                                &mut blocked_transition_witnesses,
                                BlockedTransitionWitness {
                                    transition_id: transition.id.clone(),
                                    source_state_sha256: state_identity,
                                    classification: if !resolution
                                        .unknown_obstruction_ids
                                        .is_empty()
                                    {
                                        TransitionClassification::FeasibilityUnknown
                                    } else {
                                        TransitionClassification::Obstructed
                                    },
                                    hard_guard: preliminary.hard_guard,
                                    selected_resolver_ids: resolution.applied_resolver_ids.clone(),
                                    selected_technique_ids: resolution
                                        .applicable_technique_ids
                                        .clone(),
                                    active_obstruction_ids: resolution
                                        .active_obstruction_ids
                                        .clone(),
                                    unknown_obstruction_ids: resolution
                                        .unknown_obstruction_ids
                                        .clone(),
                                    discharged_obligation_ids: resolution
                                        .discharged_obligation_ids
                                        .iter()
                                        .cloned()
                                        .collect(),
                                    outstanding_obligation_ids: preliminary
                                        .outstanding_obligation_ids,
                                    unknown_obligation_ids: preliminary.unknown_obligation_ids,
                                    supporting_microtrace_ids: resolution
                                        .supporting_microtrace_ids
                                        .iter()
                                        .cloned()
                                        .collect(),
                                    unknown_requirement_ids: preliminary.unknown_requirement_ids,
                                    reader_results,
                                    unknown_reader_ids,
                                    evidence_dependencies,
                                    weakest_evidence,
                                    hard_guard_expression: transition
                                        .activation
                                        .hard_guards
                                        .clone(),
                                    effect_operations: transition.activation.effects.clone(),
                                    obligation_derivations: self
                                        .mechanics
                                        .obligations
                                        .iter()
                                        .filter(|obligation| {
                                            transition
                                                .activation
                                                .physical_obligation_ids
                                                .contains(&obligation.id)
                                        })
                                        .cloned()
                                        .collect(),
                                    unknown_requirements: transition
                                        .activation
                                        .unknown_requirements
                                        .clone(),
                                },
                            );
                            continue;
                        }

                        let mut next = node.state.clone();
                        let mut action_boundaries = Vec::new();
                        let mut setup_failed = false;
                        for resolver_id in &resolution.applied_resolver_ids {
                            let resolver = self
                                .mechanics
                                .resolvers
                                .iter()
                                .find(|resolver| resolver.id == *resolver_id)
                                .ok_or_else(|| {
                                    PlannerContractError::new(
                                        "solver.resolver",
                                        "feasibility selected an unknown resolver",
                                    )
                                })?;
                            let before = next.clone();
                            generated_id = generated_id.saturating_add(1);
                            if !resolver.operations.is_empty()
                                && next
                                    .apply_operations(
                                        &resolver.id,
                                        &format!("search-setup-{generated_id}"),
                                        &resolver.operations,
                                    )
                                    .is_err()
                            {
                                execution_error_ids.insert(resolver.id.clone());
                                setup_failed = true;
                                break;
                            }
                            action_boundaries.push(AppliedActionBoundary {
                                action: RouteActionRef::Resolver {
                                    resolver_id: resolver.id.clone(),
                                },
                                before,
                                after: next.clone(),
                            });
                        }
                        if setup_failed {
                            continue;
                        }
                        for technique_id in &resolution.applicable_technique_ids {
                            let technique = self
                                .mechanics
                                .techniques
                                .iter()
                                .find(|technique| technique.id == *technique_id)
                                .ok_or_else(|| {
                                    PlannerContractError::new(
                                        "solver.technique",
                                        "feasibility selected an unknown technique",
                                    )
                                })?;
                            let before = next.clone();
                            generated_id = generated_id.saturating_add(1);
                            if !technique.operations.is_empty()
                                && next
                                    .apply_operations(
                                        &technique.id,
                                        &format!("search-setup-{generated_id}"),
                                        &technique.operations,
                                    )
                                    .is_err()
                            {
                                execution_error_ids.insert(technique.id.clone());
                                setup_failed = true;
                                break;
                            }
                            action_boundaries.push(AppliedActionBoundary {
                                action: RouteActionRef::Technique {
                                    technique_id: technique.id.clone(),
                                },
                                before,
                                after: next.clone(),
                            });
                        }
                        if setup_failed {
                            continue;
                        }
                        let post_setup_evaluator = PredicateEvaluator::new(
                            &next.snapshot,
                            self.facts,
                            self.equivalence_sets,
                            &next.gate_states,
                            search_evidence_policy,
                        )?;
                        post_setup_evaluator.refresh_obligation_assessments(
                            transition,
                            &self.mechanics.obligations,
                            &self.mechanics.microtraces,
                            &mut resolution,
                        );
                        let assessment = post_setup_evaluator.assess_transition(
                            transition,
                            &resolution.discharged_obligation_ids,
                            &resolution.unknown_obligation_ids,
                            self.options.feasibility_mode,
                        );
                        let (reader_results, unknown_reader_ids) = assess_transition_readers(
                            &post_setup_evaluator,
                            self.mechanics,
                            &transition.id,
                        );
                        if !unknown_reader_ids.is_empty() {
                            unknown_transition_ids.insert(transition.id.clone());
                            let mut witness = blocked_witness(
                                self.facts,
                                self.mechanics,
                                transition,
                                &next,
                                &resolution,
                                &assessment,
                                (&reader_results, &unknown_reader_ids),
                            )?;
                            witness.classification = TransitionClassification::FeasibilityUnknown;
                            record_blocked_transition_witness(
                                &mut blocked_transition_witnesses,
                                witness,
                            );
                            continue;
                        }
                        match assessment.classification {
                            TransitionClassification::Executable => {}
                            TransitionClassification::FeasibilityUnknown => {
                                unknown_transition_ids.insert(transition.id.clone());
                                record_blocked_transition_witness(
                                    &mut blocked_transition_witnesses,
                                    blocked_witness(
                                        self.facts,
                                        self.mechanics,
                                        transition,
                                        &next,
                                        &resolution,
                                        &assessment,
                                        (&reader_results, &unknown_reader_ids),
                                    )?,
                                );
                                continue;
                            }
                            TransitionClassification::Inapplicable
                            | TransitionClassification::GuardBlocked
                            | TransitionClassification::Obstructed => {
                                record_blocked_transition_witness(
                                    &mut blocked_transition_witnesses,
                                    blocked_witness(
                                        self.facts,
                                        self.mechanics,
                                        transition,
                                        &next,
                                        &resolution,
                                        &assessment,
                                        (&reader_results, &unknown_reader_ids),
                                    )?,
                                );
                                continue;
                            }
                        }

                        generated_id = generated_id.saturating_add(1);
                        let transition_before = next.clone();
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
                        action_boundaries.push(AppliedActionBoundary {
                            action: transition_action.clone(),
                            before: transition_before,
                            after: next.clone(),
                        });
                        executed_actions.extend(
                            action_boundaries
                                .iter()
                                .map(|boundary| boundary.action.clone()),
                        );
                        let evidence_dependencies = transition_evidence_dependencies(
                            self.facts,
                            self.mechanics,
                            transition,
                            &resolution,
                            &assessment,
                            &reader_results,
                            &unknown_reader_ids,
                        );
                        let weakest_evidence = weakest_evidence(&evidence_dependencies);
                        saw_unknown_goal |= self.enqueue_if_new(
                            &mut queue,
                            &visited,
                            &node,
                            next,
                            &action_boundaries,
                            route_policy.as_ref(),
                            generated_id,
                            authorization.as_deref_mut(),
                            SearchStep {
                                action_kind: SearchActionKind::Transition,
                                action_id: transition.id.clone(),
                                selected_resolver_ids: resolution.applied_resolver_ids.clone(),
                                selected_technique_ids: resolution.applicable_technique_ids.clone(),
                                active_obstruction_ids: resolution.active_obstruction_ids.clone(),
                                unknown_obstruction_ids: resolution.unknown_obstruction_ids.clone(),
                                discharged_obligation_ids: resolution
                                    .discharged_obligation_ids
                                    .iter()
                                    .cloned()
                                    .collect(),
                                outstanding_obligation_ids: assessment
                                    .outstanding_obligation_ids
                                    .clone(),
                                unknown_obligation_ids: assessment.unknown_obligation_ids.clone(),
                                supporting_microtrace_ids: resolution
                                    .supporting_microtrace_ids
                                    .iter()
                                    .cloned()
                                    .collect(),
                                introduced_obligation_ids: resolution
                                    .applicable_technique_ids
                                    .iter()
                                    .filter_map(|technique_id| {
                                        self.mechanics
                                            .techniques
                                            .iter()
                                            .find(|technique| technique.id == *technique_id)
                                    })
                                    .flat_map(|technique| {
                                        technique.introduced_obligation_ids.iter().cloned()
                                    })
                                    .collect::<BTreeSet<_>>()
                                    .into_iter()
                                    .collect(),
                                reader_results,
                                unknown_reader_ids,
                                evidence_dependencies,
                                weakest_evidence,
                                action_derivations: Vec::new(),
                                obligation_derivations: Vec::new(),
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

        if !reached_plans.is_empty() {
            order_search_plans(&mut reached_plans);
            reached_plans.truncate(max_plans);
            let primary = reached_plans.remove(0);
            return Ok(SearchResult {
                backward_relevance,
                backward_pruning_applied,
                status: SearchStatus::Reached,
                steps: primary.steps,
                explored_states: visited.len(),
                hit_search_limit,
                preference_score: primary.preference_score,
                satisfied_preference_ids: primary.satisfied_preference_ids,
                route_costs: primary.route_costs,
                result_continuation: Some(primary.continuation),
                alternative_plans: reached_plans,
                minimum_evidence: route_policy
                    .as_ref()
                    .and_then(|policy| policy.minimum_evidence),
                unknown_transition_ids: unknown_transition_ids.into_iter().collect(),
                unknown_writer_ids: unknown_writer_ids.into_iter().collect(),
                execution_error_ids: execution_error_ids.into_iter().collect(),
                blocked_transition_witnesses: Vec::new(),
                blocked_writer_witnesses: Vec::new(),
                blocked_technique_witnesses: Vec::new(),
                blocked_resolver_witnesses: Vec::new(),
                blocked_reconstruction_witnesses: Vec::new(),
                continuation_merge_proofs,
                failed_producer_cuts: Vec::new(),
                failed_producer_cut_sets: Vec::new(),
                failed_producer_cut_sets_complete: true,
            });
        }

        let unknown = hit_search_limit
            || saw_unknown_goal
            || !unknown_transition_ids.is_empty()
            || !unknown_writer_ids.is_empty()
            || !execution_error_ids.is_empty();
        let failed_producer_cuts = if hit_search_limit {
            Vec::new()
        } else {
            failed_producer_cuts(
                &backward_relevance,
                self.mechanics,
                initial_state_sha256,
                &executed_actions,
                &blocked_transition_witnesses,
                &blocked_writer_witnesses,
                &blocked_technique_witnesses,
                &blocked_resolver_witnesses,
                &blocked_reconstruction_witnesses,
            )?
        };
        let (failed_producer_cut_sets, failed_producer_cut_sets_complete) = if hit_search_limit {
            (Vec::new(), false)
        } else {
            failed_producer_cut_sets(&goal_coverage, self.facts, &failed_producer_cuts, 256)?
        };
        Ok(SearchResult {
            backward_relevance,
            backward_pruning_applied,
            status: if unknown {
                SearchStatus::Unknown
            } else {
                SearchStatus::UnreachableUnderModel
            },
            steps: Vec::new(),
            explored_states: visited.len(),
            hit_search_limit,
            preference_score: 0,
            satisfied_preference_ids: Vec::new(),
            route_costs: BTreeMap::new(),
            result_continuation: None,
            alternative_plans: Vec::new(),
            minimum_evidence: route_policy
                .as_ref()
                .and_then(|policy| policy.minimum_evidence),
            unknown_transition_ids: unknown_transition_ids.into_iter().collect(),
            unknown_writer_ids: unknown_writer_ids.into_iter().collect(),
            execution_error_ids: execution_error_ids.into_iter().collect(),
            blocked_transition_witnesses: blocked_transition_witnesses.into_values().collect(),
            blocked_writer_witnesses: blocked_writer_witnesses.into_values().collect(),
            blocked_technique_witnesses: blocked_technique_witnesses.into_values().collect(),
            blocked_resolver_witnesses: blocked_resolver_witnesses.into_values().collect(),
            blocked_reconstruction_witnesses: blocked_reconstruction_witnesses
                .into_values()
                .collect(),
            continuation_merge_proofs,
            failed_producer_cuts,
            failed_producer_cut_sets,
            failed_producer_cut_sets_complete,
        })
    }
}
