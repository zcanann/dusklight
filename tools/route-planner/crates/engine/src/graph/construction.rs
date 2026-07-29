use super::*;

impl PlannerGraph {
    pub fn project(
        facts: &FactCatalog,
        mechanics: &MechanicsCatalog,
    ) -> Result<Self, PlannerContractError> {
        Self::project_with_context(facts, mechanics, None, None)
    }

    pub fn project_composed(
        catalog: &ComposedPlannerCatalog,
    ) -> Result<Self, PlannerContractError> {
        catalog.validate()?;
        Self::project_with_context(
            &catalog.facts,
            &catalog.mechanics,
            Some(catalog.refinement_stack.digest()?),
            None,
        )
    }

    pub fn project_with_route_book(
        facts: &FactCatalog,
        mechanics: &MechanicsCatalog,
        book: &RouteBook,
    ) -> Result<Self, PlannerContractError> {
        Self::project_with_context(facts, mechanics, None, Some(book))
    }

    pub fn project_composed_with_route_book(
        catalog: &ComposedPlannerCatalog,
        book: &RouteBook,
    ) -> Result<Self, PlannerContractError> {
        catalog.validate()?;
        book.validate_against_composed(catalog)?;
        Self::project_with_context(
            &catalog.facts,
            &catalog.mechanics,
            Some(catalog.refinement_stack.digest()?),
            Some(book),
        )
    }

    fn project_with_context(
        facts: &FactCatalog,
        mechanics: &MechanicsCatalog,
        refinement_stack_sha256: Option<Digest>,
        route_book: Option<&RouteBook>,
    ) -> Result<Self, PlannerContractError> {
        facts.validate()?;
        mechanics.validate()?;
        if let Some(book) = route_book {
            book.validate_against(facts, mechanics)?;
        }
        let mut builder = GraphBuilder::new();
        builder.project_facts(facts)?;
        builder.project_mechanics(mechanics)?;
        if let Some(book) = route_book {
            builder.project_route_book(book)?;
        }
        builder.nodes.sort_by(|left, right| left.id.cmp(&right.id));
        builder.edges.sort_by(|left, right| left.id.cmp(&right.id));
        builder
            .regions
            .sort_by(|left, right| left.id.cmp(&right.id));
        let graph = Self {
            schema: PLANNER_GRAPH_SCHEMA.into(),
            fact_catalog_sha256: facts.digest()?,
            mechanics_catalog_sha256: mechanics.digest()?,
            refinement_stack_sha256,
            route_book_sha256: route_book.map(RouteBook::digest).transpose()?,
            nodes: builder.nodes,
            edges: builder.edges,
            regions: builder.regions,
        };
        graph.validate()?;
        Ok(graph)
    }

    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != PLANNER_GRAPH_SCHEMA {
            return Err(PlannerContractError::new("schema", "is unsupported"));
        }
        if self.fact_catalog_sha256 == Digest::ZERO {
            return Err(PlannerContractError::new(
                "fact_catalog_sha256",
                "must be nonzero",
            ));
        }
        if self.mechanics_catalog_sha256 == Digest::ZERO {
            return Err(PlannerContractError::new(
                "mechanics_catalog_sha256",
                "must be nonzero",
            ));
        }
        if self.refinement_stack_sha256 == Some(Digest::ZERO) {
            return Err(PlannerContractError::new(
                "refinement_stack_sha256",
                "must be absent or nonzero",
            ));
        }
        if self.route_book_sha256 == Some(Digest::ZERO) {
            return Err(PlannerContractError::new(
                "route_book_sha256",
                "must be absent or nonzero",
            ));
        }
        let region_ids = validate_regions(&self.regions)?;
        let node_ids = validate_nodes(&self.nodes, &region_ids)?;
        validate_edges(&self.edges, &node_ids)?;
        for region in &self.regions {
            if let Some(owner) = &region.owner_node_id
                && !node_ids.contains(owner.as_str())
            {
                return Err(PlannerContractError::new(
                    "regions.owner_node_id",
                    format!("references unknown node {owner}"),
                ));
            }
        }
        Ok(())
    }

    pub fn attach_authored_execution_path(
        &mut self,
        states: &[PlannerExecutionPathState],
    ) -> Result<(), PlannerContractError> {
        if states.is_empty() || states[0].route_step_id.is_some() {
            return Err(PlannerContractError::new(
                "execution_path",
                "must begin with exactly one route-start state",
            ));
        }
        let authored_steps = states
            .iter()
            .skip(1)
            .map(|state| {
                state.route_step_id.as_deref().ok_or_else(|| {
                    PlannerContractError::new(
                        "execution_path.route_step_id",
                        "every non-start state must identify its producing route step",
                    )
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let step_nodes = authored_steps
            .iter()
            .map(|step_id| {
                self.nodes
                    .iter()
                    .find(|node| {
                        matches!(
                            &node.payload,
                            PlannerNodePayload::ReferenceStep { step_id: candidate }
                                if candidate == step_id
                        )
                    })
                    .cloned()
                    .ok_or_else(|| {
                        PlannerContractError::new(
                            "execution_path.route_step_id",
                            format!("references unprojected route step {step_id}"),
                        )
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let region_id = step_nodes
            .first()
            .and_then(|node| node.region_id.clone())
            .or_else(|| {
                self.regions
                    .iter()
                    .find(|region| region.id == "region.plans")
                    .map(|region| region.id.clone())
            })
            .unwrap_or_else(|| "region.mechanics".into());
        let state_node_id = |state: &PlannerExecutionPathState| match &state.route_step_id {
            Some(step_id) => format!("execution-state/after/{step_id}"),
            None => "execution-state/route-start".into(),
        };
        for state in states {
            validate_label("execution_path.label", &state.label)?;
            if state.execution_state_sha256 == Digest::ZERO || state.snapshot_sha256 == Digest::ZERO
            {
                return Err(PlannerContractError::new(
                    "execution_path",
                    "state identities must be nonzero",
                ));
            }
            self.nodes.push(PlannerGraphNode {
                id: state_node_id(state),
                label: state.label.clone(),
                region_id: Some(region_id.clone()),
                payload: PlannerNodePayload::ExecutionState {
                    execution_state_sha256: state.execution_state_sha256,
                    snapshot_sha256: state.snapshot_sha256,
                    route_step_id: state.route_step_id.clone(),
                },
            });
        }
        for (index, (before, after)) in states.iter().zip(states.iter().skip(1)).enumerate() {
            let step = &step_nodes[index];
            let step_id = authored_steps[index];
            self.edges.push(PlannerGraphEdge {
                id: format!("edge.execution-path/{step_id}/precondition"),
                source_node_id: state_node_id(before),
                target_node_id: step.id.clone(),
                relation: PlannerGraphRelation::RoutePrecondition,
                ordinal: Some(index as u32),
            });
            self.edges.push(PlannerGraphEdge {
                id: format!("edge.execution-path/{step_id}/result"),
                source_node_id: step.id.clone(),
                target_node_id: state_node_id(after),
                relation: PlannerGraphRelation::RouteResult,
                ordinal: Some(index as u32),
            });
        }
        self.nodes.sort_by(|left, right| left.id.cmp(&right.id));
        self.edges.sort_by(|left, right| left.id.cmp(&right.id));
        self.validate()
    }

    /// Projects reached plans and exact continuation-merge proofs into nested
    /// proof regions. Alternative regions collapse only when their terminal
    /// continuation identity matches the primary plan and the primary resource
    /// label is no worse on depth or any cost axis. Otherwise the region stays
    /// expanded with explicit residual differences.
    pub fn attach_solver_proof(
        &mut self,
        initial_state_sha256: Digest,
        result: &SearchResult,
    ) -> Result<(), PlannerContractError> {
        if initial_state_sha256 == Digest::ZERO {
            return Err(PlannerContractError::new(
                "solver_proof.initial_state_sha256",
                "must be nonzero",
            ));
        }
        if self
            .regions
            .iter()
            .any(|region| region.id == "region.proof")
        {
            return Err(PlannerContractError::new(
                "solver_proof",
                "is already attached to this graph",
            ));
        }
        self.regions.push(PlannerGraphRegion {
            id: "region.proof".into(),
            label: "Solver proof".into(),
            parent_region_id: None,
            owner_node_id: None,
            region_kind: PlannerRegionKind::Proof,
            collapsed_by_default: false,
            collapse_evidence: None,
        });

        let mut plans = Vec::new();
        if result.status == SearchStatus::Reached {
            let continuation = result.result_continuation.clone().ok_or_else(|| {
                PlannerContractError::new(
                    "solver_proof.result_continuation",
                    "a reached result must retain its exact terminal continuation",
                )
            })?;
            let primary = SearchPlan {
                result_state_sha256: result
                    .steps
                    .last()
                    .map_or(initial_state_sha256, |step| step.result_state_sha256),
                continuation,
                steps: result.steps.clone(),
                preference_score: result.preference_score,
                satisfied_preference_ids: result.satisfied_preference_ids.clone(),
                route_costs: result.route_costs.clone(),
            };
            primary.validate()?;
            plans.push(("primary".to_owned(), true, primary));
            for (index, plan) in result.alternative_plans.iter().enumerate() {
                plan.validate()?;
                plans.push((format!("alternative-{index:03}"), false, plan.clone()));
            }
        } else if result.result_continuation.is_some()
            || !result.steps.is_empty()
            || !result.alternative_plans.is_empty()
        {
            return Err(PlannerContractError::new(
                "solver_proof",
                "an unreached result cannot contain reached plan steps",
            ));
        }

        let primary = plans.first().map(|(_, _, plan)| plan.clone());
        for (plan_id, is_primary, plan) in &plans {
            validate_search_step_chain(initial_state_sha256, &plan.steps)?;
            let weakest_evidence = plan
                .steps
                .iter()
                .filter_map(|step| step.weakest_evidence)
                .max();
            let resource_label = SearchResourceLabel {
                depth: plan.steps.len(),
                route_costs: plan.route_costs.clone(),
            };
            resource_label.validate()?;
            let (collapsed_by_default, collapse_evidence) = if *is_primary {
                (false, None)
            } else {
                let reference = primary.as_ref().expect("an alternative has a primary plan");
                proof_plan_collapse(reference, plan, weakest_evidence)
            };
            let region_id = format!("region.proof.plan.{plan_id}");
            let plan_node_id = format!("proof-plan/{plan_id}");
            self.nodes.push(PlannerGraphNode {
                id: plan_node_id.clone(),
                label: if *is_primary {
                    "Primary solver plan".into()
                } else {
                    format!("Alternative solver plan {}", &plan_id[12..])
                },
                region_id: Some(region_id.clone()),
                payload: PlannerNodePayload::ProofPlan {
                    plan_id: plan_id.clone(),
                    primary: *is_primary,
                    result_state_sha256: plan.result_state_sha256,
                    continuation: plan.continuation.clone(),
                    preference_score: plan.preference_score,
                    satisfied_preference_ids: plan.satisfied_preference_ids.clone(),
                    route_costs: plan.route_costs.clone(),
                    weakest_evidence,
                },
            });
            self.regions.push(PlannerGraphRegion {
                id: region_id.clone(),
                label: if *is_primary {
                    "Primary plan".into()
                } else {
                    format!("Alternative {}", &plan_id[12..])
                },
                parent_region_id: Some("region.proof".into()),
                owner_node_id: Some(plan_node_id.clone()),
                region_kind: PlannerRegionKind::Proof,
                collapsed_by_default,
                collapse_evidence,
            });

            let mut state_sha256 = initial_state_sha256;
            for ordinal in 0..=plan.steps.len() {
                let state_node_id = format!("proof-state/{plan_id}/{ordinal:04}");
                self.nodes.push(PlannerGraphNode {
                    id: state_node_id.clone(),
                    label: if ordinal == 0 {
                        "Plan start".into()
                    } else {
                        format!("State after step {ordinal}")
                    },
                    region_id: Some(region_id.clone()),
                    payload: PlannerNodePayload::ProofState {
                        plan_id: plan_id.clone(),
                        ordinal: ordinal as u32,
                        state_sha256,
                    },
                });
                push_graph_edge(
                    &mut self.edges,
                    &plan_node_id,
                    &state_node_id,
                    PlannerGraphRelation::Contains,
                    Some((ordinal * 2) as u32),
                )?;
                let Some(step) = plan.steps.get(ordinal) else {
                    continue;
                };
                let step_node_id = format!("proof-step/{plan_id}/{ordinal:04}");
                self.nodes.push(PlannerGraphNode {
                    id: step_node_id.clone(),
                    label: format!(
                        "{} · {}",
                        action_kind_label(step.action_kind),
                        step.action_id
                    ),
                    region_id: Some(region_id.clone()),
                    payload: PlannerNodePayload::ProofStep {
                        plan_id: plan_id.clone(),
                        ordinal: ordinal as u32,
                        action_kind: step.action_kind,
                        action_id: step.action_id.clone(),
                        source_state_sha256: step.source_state_sha256,
                        result_state_sha256: step.result_state_sha256,
                    },
                });
                let action_node = search_action_node_id(step.action_kind, &step.action_id);
                if !self.nodes.iter().any(|node| node.id == action_node) {
                    return Err(PlannerContractError::new(
                        "solver_proof.steps.action_id",
                        format!("references unprojected action {}", step.action_id),
                    ));
                }
                push_graph_edge(
                    &mut self.edges,
                    &plan_node_id,
                    &step_node_id,
                    PlannerGraphRelation::Contains,
                    Some((ordinal * 2 + 1) as u32),
                )?;
                push_graph_edge(
                    &mut self.edges,
                    &state_node_id,
                    &step_node_id,
                    PlannerGraphRelation::RoutePrecondition,
                    Some(ordinal as u32),
                )?;
                push_graph_edge(
                    &mut self.edges,
                    &step_node_id,
                    &format!("proof-state/{plan_id}/{:04}", ordinal + 1),
                    PlannerGraphRelation::RouteResult,
                    Some(ordinal as u32),
                )?;
                push_graph_edge(
                    &mut self.edges,
                    &step_node_id,
                    &action_node,
                    PlannerGraphRelation::SelectsAction,
                    None,
                )?;
                state_sha256 = step.result_state_sha256;
            }
        }

        if !result.continuation_merge_proofs.is_empty() {
            let region_id = "region.proof.continuation-merges";
            self.regions.push(PlannerGraphRegion {
                id: region_id.into(),
                label: "Proven continuation merges".into(),
                parent_region_id: Some("region.proof".into()),
                owner_node_id: None,
                region_kind: PlannerRegionKind::Proof,
                collapsed_by_default: true,
                collapse_evidence: Some(PlannerCollapseEvidence::ProvenContinuationMerges {
                    merge_count: result.continuation_merge_proofs.len(),
                }),
            });
            for (index, proof) in result.continuation_merge_proofs.iter().enumerate() {
                proof.validate()?;
                self.nodes.push(PlannerGraphNode {
                    id: format!("continuation-merge/{index:04}"),
                    label: format!("Dominated frontier label {}", index + 1),
                    region_id: Some(region_id.into()),
                    payload: PlannerNodePayload::ContinuationMerge {
                        state_sha256: proof.continuation.state_sha256,
                        dominating: proof.dominating.clone(),
                        dominated: proof.dominated.clone(),
                        satisfied_preference_ids: proof
                            .continuation
                            .satisfied_preference_ids
                            .clone(),
                    },
                });
            }
        }

        self.nodes.sort_by(|left, right| left.id.cmp(&right.id));
        self.edges.sort_by(|left, right| left.id.cmp(&right.id));
        self.regions.sort_by(|left, right| left.id.cmp(&right.id));
        self.validate()
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PlannerContractError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, PlannerContractError> {
        let graph: Self = serde_json::from_slice(bytes)?;
        graph.validate()?;
        if graph.canonical_bytes()? != bytes {
            return Err(PlannerContractError::new(
                "planner_graph",
                "is not canonical JSON",
            ));
        }
        Ok(graph)
    }

    pub fn digest(&self) -> Result<Digest, PlannerContractError> {
        Ok(Digest(Sha256::digest(self.canonical_bytes()?).into()))
    }
}

impl PlannerFeasibilityGraphDiff {
    pub fn project(
        state: &PlannerExecutionState,
        facts: &FactCatalog,
        mechanics: &MechanicsCatalog,
        equivalence_sets: &[EquivalenceSet],
        evidence_policy: EvidencePolicy,
    ) -> Result<Self, PlannerContractError> {
        state.validate()?;
        let snapshot = &state.snapshot;
        snapshot.validate()?;
        facts.validate()?;
        mechanics.validate()?;
        let evaluator = PredicateEvaluator::new(
            snapshot,
            facts,
            equivalence_sets,
            &state.gate_states,
            evidence_policy,
        )?;
        let empty = BTreeSet::new();
        let mut transitions = Vec::new();
        for transition in &mechanics.transitions {
            let upper_bound = evaluator.assess_transition(
                transition,
                &empty,
                &empty,
                FeasibilityMode::UpperBound,
            );
            let resolution = evaluator.resolve_feasibility(
                transition,
                &mechanics.obligations,
                &mechanics.obstructions,
                &mechanics.resolvers,
                &mechanics.techniques,
                FeasibilitySelection {
                    resolver_ids: &empty,
                    technique_ids: &empty,
                    already_discharged: &empty,
                    microtraces: &mechanics.microtraces,
                },
            );
            let mut modeled = evaluator.assess_transition(
                transition,
                &resolution.discharged_obligation_ids,
                &resolution.unknown_obligation_ids,
                FeasibilityMode::Modeled,
            );
            if matches!(
                modeled.classification,
                TransitionClassification::Executable | TransitionClassification::Obstructed
            ) {
                if !resolution.unknown_obstruction_ids.is_empty() {
                    modeled.classification = TransitionClassification::FeasibilityUnknown;
                } else if !resolution.active_obstruction_ids.is_empty() {
                    modeled.classification = TransitionClassification::Obstructed;
                }
            }
            if upper_bound != modeled
                || !resolution.active_obstruction_ids.is_empty()
                || !resolution.unknown_obstruction_ids.is_empty()
                || !resolution.supporting_microtrace_ids.is_empty()
            {
                transitions.push(TransitionFeasibilityDelta {
                    transition_id: transition.id.clone(),
                    upper_bound,
                    modeled,
                    active_obstruction_ids: resolution.active_obstruction_ids,
                    unknown_obstruction_ids: resolution.unknown_obstruction_ids,
                    discharged_obligation_ids: resolution
                        .discharged_obligation_ids
                        .into_iter()
                        .collect(),
                    supporting_microtrace_ids: resolution
                        .supporting_microtrace_ids
                        .into_iter()
                        .collect(),
                });
            }
        }
        let diff = Self {
            schema: PLANNER_FEASIBILITY_DIFF_SCHEMA.into(),
            execution_state_sha256: state.semantic_digest()?,
            snapshot_sha256: snapshot.digest()?,
            fact_catalog_sha256: facts.digest()?,
            mechanics_catalog_sha256: mechanics.digest()?,
            transitions,
        };
        diff.validate()?;
        Ok(diff)
    }

    pub fn project_composed(
        state: &PlannerExecutionState,
        catalog: &ComposedPlannerCatalog,
        equivalence_sets: &[EquivalenceSet],
        evidence_policy: EvidencePolicy,
    ) -> Result<Self, PlannerContractError> {
        catalog.validate()?;
        Self::project(
            state,
            &catalog.facts,
            &catalog.mechanics,
            equivalence_sets,
            evidence_policy,
        )
    }

    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != PLANNER_FEASIBILITY_DIFF_SCHEMA {
            return Err(PlannerContractError::new("schema", "is unsupported"));
        }
        if self.execution_state_sha256 == Digest::ZERO
            || self.snapshot_sha256 == Digest::ZERO
            || self.fact_catalog_sha256 == Digest::ZERO
            || self.mechanics_catalog_sha256 == Digest::ZERO
        {
            return Err(PlannerContractError::new(
                "feasibility_graph_diff",
                "contains a zero source digest",
            ));
        }
        let mut previous = None;
        for transition in &self.transitions {
            validate_stable_id("transitions.transition_id", &transition.transition_id)?;
            if previous.is_some_and(|id: &str| id >= transition.transition_id.as_str()) {
                return Err(PlannerContractError::new(
                    "transitions",
                    "must be unique and sorted by transition ID",
                ));
            }
            if transition.upper_bound.transition_id != transition.transition_id
                || transition.modeled.transition_id != transition.transition_id
            {
                return Err(PlannerContractError::new(
                    "transitions.assessment.transition_id",
                    "must match the enclosing transition ID",
                ));
            }
            validate_transition_assessment(&transition.upper_bound)?;
            validate_transition_assessment(&transition.modeled)?;
            validate_sorted_ids(
                "transitions.active_obstruction_ids",
                &transition.active_obstruction_ids,
            )?;
            validate_sorted_ids(
                "transitions.unknown_obstruction_ids",
                &transition.unknown_obstruction_ids,
            )?;
            validate_sorted_ids(
                "transitions.discharged_obligation_ids",
                &transition.discharged_obligation_ids,
            )?;
            validate_sorted_ids(
                "transitions.supporting_microtrace_ids",
                &transition.supporting_microtrace_ids,
            )?;
            previous = Some(transition.transition_id.as_str());
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PlannerContractError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, PlannerContractError> {
        let diff: Self = serde_json::from_slice(bytes)?;
        diff.validate()?;
        if diff.canonical_bytes()? != bytes {
            return Err(PlannerContractError::new(
                "feasibility_graph_diff",
                "is not canonical JSON",
            ));
        }
        Ok(diff)
    }

    pub fn digest(&self) -> Result<Digest, PlannerContractError> {
        Ok(Digest(Sha256::digest(self.canonical_bytes()?).into()))
    }
}

struct GraphBuilder {
    nodes: Vec<PlannerGraphNode>,
    edges: Vec<PlannerGraphEdge>,
    regions: Vec<PlannerGraphRegion>,
    node_ids: BTreeSet<String>,
    edge_ids: BTreeSet<String>,
}

impl GraphBuilder {
    fn new() -> Self {
        Self {
            nodes: Vec::new(),
            edges: Vec::new(),
            regions: vec![
                PlannerGraphRegion {
                    id: "region.facts".into(),
                    label: "Facts".into(),
                    parent_region_id: None,
                    owner_node_id: None,
                    region_kind: PlannerRegionKind::Facts,
                    collapsed_by_default: true,
                    collapse_evidence: None,
                },
                PlannerGraphRegion {
                    id: "region.mechanics".into(),
                    label: "Mechanics".into(),
                    parent_region_id: None,
                    owner_node_id: None,
                    region_kind: PlannerRegionKind::Mechanics,
                    collapsed_by_default: false,
                    collapse_evidence: None,
                },
            ],
            node_ids: BTreeSet::new(),
            edge_ids: BTreeSet::new(),
        }
    }

    fn project_facts(&mut self, facts: &FactCatalog) -> Result<(), PlannerContractError> {
        for alias in &facts.aliases {
            self.add_node(PlannerGraphNode {
                id: fact_node_id(&alias.id),
                label: alias.label.clone(),
                region_id: Some("region.facts".into()),
                payload: PlannerNodePayload::Alias {
                    fact_id: alias.id.clone(),
                },
            })?;
        }
        for fact in &facts.derived_facts {
            self.add_node(PlannerGraphNode {
                id: fact_node_id(&fact.id),
                label: fact.label.clone(),
                region_id: Some("region.facts".into()),
                payload: PlannerNodePayload::DerivedFact {
                    fact_id: fact.id.clone(),
                },
            })?;
        }
        for fact in &facts.derived_facts {
            let owner = fact_node_id(&fact.id);
            self.project_predicate(
                &owner,
                "derived",
                &fact.rule,
                PlannerGraphRelation::Requires,
            )?;
        }
        Ok(())
    }

    fn project_mechanics(
        &mut self,
        mechanics: &MechanicsCatalog,
    ) -> Result<(), PlannerContractError> {
        let transitions = mechanics
            .transitions
            .iter()
            .map(|record| record.id.as_str())
            .collect::<BTreeSet<_>>();
        for record in &mechanics.obligations {
            let owner = format!("obligation/{}", record.id);
            self.add_record_node(
                &owner,
                &record.label,
                PlannerNodePayload::Obligation {
                    obligation_id: record.id.clone(),
                },
            )?;
            match &record.detail {
                ObligationDetail::Predicate { predicate }
                | ObligationDetail::Temporal {
                    precondition: predicate,
                    ..
                } => {
                    self.project_predicate(
                        &owner,
                        "requirement",
                        predicate,
                        PlannerGraphRelation::Requires,
                    )?;
                }
                ObligationDetail::Interaction { pose_predicate, .. } => {
                    self.project_predicate(
                        &owner,
                        "pose",
                        pose_predicate,
                        PlannerGraphRelation::Requires,
                    )?;
                }
                ObligationDetail::CompoundInteraction { branches, .. } => {
                    for (index, branch) in branches.iter().enumerate() {
                        self.project_predicate(
                            &owner,
                            &format!("branch-{index}-when"),
                            &branch.when,
                            PlannerGraphRelation::Requires,
                        )?;
                        self.project_predicate(
                            &owner,
                            &format!("branch-{index}-pose"),
                            &branch.pose_predicate,
                            PlannerGraphRelation::Requires,
                        )?;
                    }
                }
                ObligationDetail::Geometry { .. }
                | ObligationDetail::PlaneSide { .. }
                | ObligationDetail::Facing { .. }
                | ObligationDetail::Unresolved { .. } => {}
            }
        }
        for record in &mechanics.transitions {
            let owner = format!("transition/{}", record.id);
            self.add_record_node(
                &owner,
                &record.label,
                PlannerNodePayload::Transition {
                    transition_id: record.id.clone(),
                },
            )?;
            self.project_predicate(
                &owner,
                "guard",
                &record.activation.hard_guards,
                PlannerGraphRelation::Requires,
            )?;
            for (index, obligation) in record.activation.physical_obligation_ids.iter().enumerate()
            {
                self.add_edge(
                    &owner,
                    &format!("obligation/{obligation}"),
                    PlannerGraphRelation::Requires,
                    Some(index as u32),
                )?;
            }
        }
        for record in &mechanics.writers {
            let owner = format!("writer/{}", record.id);
            self.add_record_node(
                &owner,
                &record.id,
                PlannerNodePayload::Writer {
                    writer_id: record.id.clone(),
                },
            )?;
            self.project_predicate(
                &owner,
                "activation",
                &record.activation,
                PlannerGraphRelation::Requires,
            )?;
        }
        for record in &mechanics.gates {
            let owner = format!("gate/{}", record.id);
            self.add_record_node(
                &owner,
                &record.id,
                PlannerNodePayload::Gate {
                    gate_id: record.id.clone(),
                },
            )?;
            self.project_predicate(
                &owner,
                "active",
                &record.active_when,
                PlannerGraphRelation::Requires,
            )?;
            for (index, writer) in record.blocked_writer_ids.iter().enumerate() {
                self.add_edge(
                    &owner,
                    &format!("writer/{writer}"),
                    PlannerGraphRelation::Suppresses,
                    Some(index as u32),
                )?;
            }
        }
        for record in &mechanics.readers {
            let owner = format!("reader/{}", record.id);
            self.add_record_node(
                &owner,
                &record.id,
                PlannerNodePayload::Reader {
                    reader_id: record.id.clone(),
                },
            )?;
            self.add_edge(
                &owner,
                &format!("transition/{}", record.consuming_transition_id),
                PlannerGraphRelation::ConsumedBy,
                None,
            )?;
            if let Some(fact) = &record.interpretation_fact_id {
                let target = self.ensure_fact_node(fact)?;
                self.add_edge(&owner, &target, PlannerGraphRelation::Interprets, None)?;
            }
        }
        for record in &mechanics.reconstruction_rules {
            let owner = format!("reconstruction/{}", record.id);
            self.add_record_node(
                &owner,
                &record.label,
                PlannerNodePayload::Reconstruction {
                    reconstruction_rule_id: record.id.clone(),
                },
            )?;
            self.project_predicate(
                &owner,
                "instantiate",
                &record.instantiate_when,
                PlannerGraphRelation::ReconstructsWhen,
            )?;
        }
        for record in &mechanics.obstructions {
            let owner = format!("obstruction/{}", record.id);
            self.add_record_node(
                &owner,
                &record.label,
                PlannerNodePayload::Obstruction {
                    obstruction_id: record.id.clone(),
                },
            )?;
            self.project_predicate(
                &owner,
                "active",
                &record.active_when,
                PlannerGraphRelation::Requires,
            )?;
            let action = if transitions.contains(record.blocked_action_id.as_str()) {
                format!("transition/{}", record.blocked_action_id)
            } else {
                self.ensure_external_action(&record.blocked_action_id)?
            };
            self.add_edge(&owner, &action, PlannerGraphRelation::Blocks, None)?;
            for (index, obligation) in record.obligation_ids.iter().enumerate() {
                self.add_edge(
                    &owner,
                    &format!("obligation/{obligation}"),
                    PlannerGraphRelation::Requires,
                    Some(index as u32),
                )?;
            }
        }
        for record in &mechanics.resolvers {
            let owner = format!("resolver/{}", record.id);
            self.add_record_node(
                &owner,
                &record.label,
                PlannerNodePayload::Resolver {
                    resolver_id: record.id.clone(),
                    resolution_kind: record.resolution_kind,
                },
            )?;
            self.project_predicate(
                &owner,
                "applicable",
                &record.applicable_when,
                PlannerGraphRelation::Requires,
            )?;
            self.add_edge(
                &owner,
                &format!("obstruction/{}", record.obstruction_id),
                PlannerGraphRelation::Resolves,
                None,
            )?;
        }
        for record in &mechanics.techniques {
            let owner = format!("technique/{}", record.id);
            self.add_record_node(
                &owner,
                &record.label,
                PlannerNodePayload::Technique {
                    technique_id: record.id.clone(),
                },
            )?;
            self.project_predicate(
                &owner,
                "prerequisite",
                &record.prerequisites,
                PlannerGraphRelation::Requires,
            )?;
            for (index, obligation) in record.discharged_obligation_ids.iter().enumerate() {
                self.add_edge(
                    &owner,
                    &format!("obligation/{obligation}"),
                    PlannerGraphRelation::Discharges,
                    Some(index as u32),
                )?;
            }
            for (index, obligation) in record.introduced_obligation_ids.iter().enumerate() {
                self.add_edge(
                    &owner,
                    &format!("obligation/{obligation}"),
                    PlannerGraphRelation::Introduces,
                    Some(index as u32),
                )?;
            }
        }
        for record in &mechanics.microtraces {
            let owner = format!("microtrace/{}", record.id);
            self.add_record_node(
                &owner,
                &record.id,
                PlannerNodePayload::Microtrace {
                    microtrace_id: record.id.clone(),
                },
            )?;
            self.project_predicate(
                &owner,
                "precondition",
                &record.precondition,
                PlannerGraphRelation::Requires,
            )?;
            self.project_predicate(
                &owner,
                "postcondition",
                &record.postcondition,
                PlannerGraphRelation::Demonstrates,
            )?;
        }
        for obligation in &mechanics.obligations {
            let requirement = match &obligation.detail {
                ObligationDetail::Interaction {
                    temporal_requirement: Some(requirement),
                    ..
                }
                | ObligationDetail::CompoundInteraction {
                    temporal_requirement: Some(requirement),
                    ..
                }
                | ObligationDetail::Temporal { requirement, .. } => Some(requirement),
                _ => None,
            };
            let Some(requirement) = requirement else {
                continue;
            };
            for (index, trace) in mechanics
                .microtraces
                .iter()
                .filter(|trace| {
                    trace.witnesses(requirement)
                        && obligation
                            .scope
                            .selectors
                            .iter()
                            .any(|selector| trace.scope.selectors.contains(selector))
                })
                .enumerate()
            {
                self.add_edge(
                    &format!("microtrace/{}", trace.id),
                    &format!("obligation/{}", obligation.id),
                    PlannerGraphRelation::Demonstrates,
                    Some(index as u32),
                )?;
            }
        }
        for record in &mechanics.goals {
            let owner = format!("goal/{}", record.id);
            self.add_record_node(
                &owner,
                &record.label,
                PlannerNodePayload::Goal {
                    goal_id: record.id.clone(),
                },
            )?;
            self.project_predicate(
                &owner,
                "predicate",
                &record.predicate,
                PlannerGraphRelation::Requires,
            )?;
        }
        Ok(())
    }

    fn project_route_book(&mut self, book: &RouteBook) -> Result<(), PlannerContractError> {
        self.regions.push(PlannerGraphRegion {
            id: "region.plans".into(),
            label: book.manifest.label.clone(),
            parent_region_id: None,
            owner_node_id: None,
            region_kind: PlannerRegionKind::Plan,
            collapsed_by_default: false,
            collapse_evidence: None,
        });
        for region in &book.regions {
            let node_id = format!("plan-region/{}", region.id);
            let graph_region_id = plan_region_graph_id(&region.id);
            self.add_node(PlannerGraphNode {
                id: node_id.clone(),
                label: region.label.clone(),
                region_id: Some(graph_region_id.clone()),
                payload: PlannerNodePayload::PlanRegion {
                    plan_region_id: region.id.clone(),
                    collapse_policy: region.collapse_policy,
                },
            })?;
            self.regions.push(PlannerGraphRegion {
                id: graph_region_id,
                label: region.label.clone(),
                parent_region_id: Some(
                    region
                        .parent_region_id
                        .as_deref()
                        .map(plan_region_graph_id)
                        .unwrap_or_else(|| "region.plans".into()),
                ),
                owner_node_id: Some(node_id),
                region_kind: PlannerRegionKind::Plan,
                // A route book may request collapse, but only a plan/proof
                // projection can prove continuation equivalence or attach
                // residual differences. The catalog projection stays expanded.
                collapsed_by_default: false,
                collapse_evidence: None,
            });
        }
        for method in &book.methods {
            let owner = format!("plan-method/{}", method.id);
            self.add_node(PlannerGraphNode {
                id: owner.clone(),
                label: method.label.clone(),
                region_id: Some(plan_region_graph_id(&method.region_id)),
                payload: PlannerNodePayload::PlanMethod {
                    method_id: method.id.clone(),
                },
            })?;
            self.add_edge(
                &format!("plan-region/{}", method.region_id),
                &owner,
                PlannerGraphRelation::Alternative,
                book.regions
                    .iter()
                    .find(|region| region.id == method.region_id)
                    .and_then(|region| {
                        region
                            .method_ids
                            .iter()
                            .position(|id| id == &method.id)
                            .map(|index| index as u32)
                    }),
            )?;
        }
        for step in &book.steps {
            let owner = format!("plan-step/{}", step.id);
            self.add_node(PlannerGraphNode {
                id: owner.clone(),
                label: step.label.clone(),
                region_id: Some(
                    step.region_id
                        .as_deref()
                        .map(plan_region_graph_id)
                        .unwrap_or_else(|| "region.plans".into()),
                ),
                payload: PlannerNodePayload::ReferenceStep {
                    step_id: step.id.clone(),
                },
            })?;
            self.add_edge(
                &owner,
                &action_node_id(&step.action),
                PlannerGraphRelation::SelectsAction,
                None,
            )?;
            let parent = step
                .region_id
                .as_deref()
                .map(plan_region_graph_id)
                .unwrap_or_else(|| "region.plans".into());
            if let Some(predicate) = &step.precondition {
                self.project_predicate_in_region(
                    &owner,
                    "precondition",
                    predicate,
                    PlannerGraphRelation::Requires,
                    &parent,
                )?;
            }
            if let Some(predicate) = &step.postcondition {
                self.project_predicate_in_region(
                    &owner,
                    "postcondition",
                    predicate,
                    PlannerGraphRelation::Demonstrates,
                    &parent,
                )?;
            }
        }
        for method in &book.methods {
            for (index, step) in method.step_ids.iter().enumerate() {
                self.add_edge(
                    &format!("plan-method/{}", method.id),
                    &format!("plan-step/{step}"),
                    PlannerGraphRelation::Contains,
                    Some(index as u32),
                )?;
            }
        }
        for region in &book.regions {
            let owner = format!("plan-region/{}", region.id);
            let parent = plan_region_graph_id(&region.id);
            if let Some(predicate) = &region.entry_predicate {
                self.project_predicate_in_region(
                    &owner,
                    "entry",
                    predicate,
                    PlannerGraphRelation::Requires,
                    &parent,
                )?;
            }
            self.project_predicate_in_region(
                &owner,
                "outcome",
                &region.outcome_predicate,
                PlannerGraphRelation::Demonstrates,
                &parent,
            )?;
            if let Some(selected) = &region.selected_method_id {
                self.add_edge(
                    &owner,
                    &format!("plan-method/{selected}"),
                    PlannerGraphRelation::Selected,
                    None,
                )?;
            }
        }
        Ok(())
    }

    fn add_record_node(
        &mut self,
        id: &str,
        label: &str,
        payload: PlannerNodePayload,
    ) -> Result<(), PlannerContractError> {
        self.add_node(PlannerGraphNode {
            id: id.into(),
            label: label.into(),
            region_id: Some("region.mechanics".into()),
            payload,
        })
    }

    fn project_predicate(
        &mut self,
        owner: &str,
        role: &str,
        expression: &PredicateExpression,
        relation: PlannerGraphRelation,
    ) -> Result<(), PlannerContractError> {
        let parent = if owner.starts_with("fact/") {
            "region.facts"
        } else {
            "region.mechanics"
        };
        self.project_predicate_in_region(owner, role, expression, relation, parent)
    }

    fn project_predicate_in_region(
        &mut self,
        owner: &str,
        role: &str,
        expression: &PredicateExpression,
        relation: PlannerGraphRelation,
        parent_region_id: &str,
    ) -> Result<(), PlannerContractError> {
        let region_id = format!("region.predicate.{owner}.{role}").replace('/', ".");
        self.regions.push(PlannerGraphRegion {
            id: region_id.clone(),
            label: format!("{role} requirements"),
            parent_region_id: Some(parent_region_id.into()),
            owner_node_id: Some(owner.into()),
            region_kind: PlannerRegionKind::Predicate,
            collapsed_by_default: true,
            collapse_evidence: None,
        });
        let root = self.add_predicate_node(owner, role, "root", expression, &region_id)?;
        self.add_edge(owner, &root, relation, None)
    }

    fn add_predicate_node(
        &mut self,
        owner: &str,
        role: &str,
        path: &str,
        expression: &PredicateExpression,
        region_id: &str,
    ) -> Result<String, PlannerContractError> {
        let id = format!("predicate/{owner}/{role}/{path}");
        let (label, operator, children): (String, PredicateOperator, &[PredicateExpression]) =
            match expression {
                PredicateExpression::True => ("Always".into(), PredicateOperator::True, &[]),
                PredicateExpression::False => ("Never".into(), PredicateOperator::False, &[]),
                PredicateExpression::Fact { fact_id } => (
                    format!("Fact: {fact_id}"),
                    PredicateOperator::Fact {
                        fact_id: fact_id.clone(),
                    },
                    &[],
                ),
                PredicateExpression::Compare {
                    left,
                    operator,
                    right,
                } => (
                    comparison_label(*operator),
                    PredicateOperator::Compare {
                        left: left.clone(),
                        operator: *operator,
                        right: right.clone(),
                    },
                    &[],
                ),
                PredicateExpression::All { terms } => {
                    ("All requirements".into(), PredicateOperator::All, terms)
                }
                PredicateExpression::Any { terms } => {
                    ("Any requirement".into(), PredicateOperator::Any, terms)
                }
                PredicateExpression::Not { term } => (
                    "Not".into(),
                    PredicateOperator::Not,
                    std::slice::from_ref(term.as_ref()),
                ),
            };
        self.add_node(PlannerGraphNode {
            id: id.clone(),
            label,
            region_id: Some(region_id.into()),
            payload: PlannerNodePayload::Predicate { operator },
        })?;
        if let PredicateExpression::Fact { fact_id } = expression {
            let target = self.ensure_fact_node(fact_id)?;
            self.add_edge(&id, &target, PlannerGraphRelation::References, None)?;
        }
        for (index, child) in children.iter().enumerate() {
            let child_id =
                self.add_predicate_node(owner, role, &format!("{path}.{index}"), child, region_id)?;
            self.add_edge(
                &id,
                &child_id,
                PlannerGraphRelation::Operand,
                Some(index as u32),
            )?;
        }
        Ok(id)
    }

    fn ensure_fact_node(&mut self, fact_id: &str) -> Result<String, PlannerContractError> {
        let known = fact_node_id(fact_id);
        if self.node_ids.contains(&known) {
            return Ok(known);
        }
        let external = format!("external/fact/{fact_id}");
        if !self.node_ids.contains(&external) {
            self.add_node(PlannerGraphNode {
                id: external.clone(),
                label: format!("External fact: {fact_id}"),
                region_id: Some("region.facts".into()),
                payload: PlannerNodePayload::ExternalFact {
                    fact_id: fact_id.into(),
                },
            })?;
        }
        Ok(external)
    }

    fn ensure_external_action(&mut self, action_id: &str) -> Result<String, PlannerContractError> {
        let id = format!("external/action/{action_id}");
        if !self.node_ids.contains(&id) {
            self.add_record_node(
                &id,
                &format!("External action: {action_id}"),
                PlannerNodePayload::ExternalAction {
                    action_id: action_id.into(),
                },
            )?;
        }
        Ok(id)
    }

    fn add_node(&mut self, node: PlannerGraphNode) -> Result<(), PlannerContractError> {
        if !self.node_ids.insert(node.id.clone()) {
            return Err(PlannerContractError::new(
                "nodes.id",
                format!("duplicate projected node {}", node.id),
            ));
        }
        self.nodes.push(node);
        Ok(())
    }

    fn add_edge(
        &mut self,
        source: &str,
        target: &str,
        relation: PlannerGraphRelation,
        ordinal: Option<u32>,
    ) -> Result<(), PlannerContractError> {
        let identity = serde_json::to_vec(&(source, target, relation, ordinal))?;
        let digest = Sha256::digest(identity);
        let id = format!("edge.{}", encode_hex(&digest));
        if !self.edge_ids.insert(id.clone()) {
            return Err(PlannerContractError::new(
                "edges.id",
                format!("duplicate projected edge {id}"),
            ));
        }
        self.edges.push(PlannerGraphEdge {
            id,
            source_node_id: source.into(),
            target_node_id: target.into(),
            relation,
            ordinal,
        });
        Ok(())
    }
}
