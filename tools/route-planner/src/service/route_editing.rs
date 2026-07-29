use super::*;

const AUTHORED_REGION_ID: &str = "region.authored-route";
const AUTHORED_METHOD_ID: &str = "method.authored-route";

pub(super) fn assess_and_apply_transition(
    state: &mut PlannerExecutionState,
    catalog: &ComposedPlannerCatalog,
    equivalence_sets: &[EquivalenceSet],
    transition_id: &str,
    evidence_mode: crate::RuntimeEvidenceMode,
    application_id: &str,
) -> Result<TransitionEvaluationResult, dusklight_route_planner::PlannerContractError> {
    let transition = catalog
        .mechanics
        .transitions
        .iter()
        .find(|transition| transition.id == transition_id)
        .ok_or_else(|| {
            dusklight_route_planner::PlannerContractError::new(
                "transition_id",
                "does not name a transition in the composed catalog",
            )
        })?;
    let policy = match evidence_mode {
        crate::RuntimeEvidenceMode::EstablishedOnly => EvidencePolicy::ESTABLISHED_ONLY,
        crate::RuntimeEvidenceMode::Research => EvidencePolicy::RESEARCH,
    };
    let empty = BTreeSet::new();
    let (assessment, diagnostics) = {
        let evaluator = PredicateEvaluator::new(
            &state.snapshot,
            &catalog.facts,
            equivalence_sets,
            &state.gate_states,
            policy,
        )?;
        let resolution = evaluator.resolve_feasibility(
            transition,
            &catalog.mechanics.obligations,
            &catalog.mechanics.obstructions,
            &catalog.mechanics.resolvers,
            &catalog.mechanics.techniques,
            FeasibilitySelection {
                resolver_ids: &empty,
                technique_ids: &empty,
                already_discharged: &empty,
                microtraces: &catalog.mechanics.microtraces,
            },
        );
        let assessment = evaluator.assess_transition(
            transition,
            &resolution.discharged_obligation_ids,
            &resolution.unknown_obligation_ids,
            FeasibilityMode::Modeled,
        );
        let diagnostics = TransitionJoinDiagnostics {
            active_obstruction_ids: resolution.active_obstruction_ids,
            unknown_obstruction_ids: resolution.unknown_obstruction_ids,
            applied_resolver_ids: resolution.applied_resolver_ids,
            applicable_technique_ids: resolution.applicable_technique_ids,
        };
        (assessment, diagnostics)
    };
    if assessment.classification == TransitionClassification::Executable {
        state.apply_operations(
            application_id,
            &format!("{application_id}.after"),
            &transition.activation.effects,
        )?;
    }
    Ok(TransitionEvaluationResult {
        assessment,
        diagnostics,
    })
}

pub(super) struct TransitionEvaluationResult {
    pub(super) assessment: TransitionAssessment,
    pub(super) diagnostics: TransitionJoinDiagnostics,
}

pub(super) fn inspect_route_frontier(
    mut state: PlannerExecutionState,
    catalog: &ComposedPlannerCatalog,
    equivalence_sets: &[EquivalenceSet],
    route_book: Option<RouteBook>,
    evidence_mode: crate::RuntimeEvidenceMode,
) -> Result<PlannerServicePayload, dusklight_route_planner::PlannerContractError> {
    let path_state = |state: &PlannerExecutionState,
                      route_step_id: Option<String>|
     -> Result<
        PlannerExecutionPathState,
        dusklight_route_planner::PlannerContractError,
    > {
        let location = &state.snapshot.environment.location;
        Ok(PlannerExecutionPathState {
            label: match &route_step_id {
                Some(step_id) => format!(
                    "After {step_id}: {} r{} l{} s{}",
                    location.stage, location.room, location.layer, location.spawn
                ),
                None => format!(
                    "Route start: {} r{} l{} s{}",
                    location.stage, location.room, location.layer, location.spawn
                ),
            },
            execution_state_sha256: state.digest()?,
            snapshot_sha256: state.snapshot.digest()?,
            route_step_id,
        })
    };
    let mut execution_path = vec![path_state(&state, None)?];
    let mut execution_states = vec![inspect_state(
        &state,
        &catalog.facts,
        equivalence_sets,
        evidence_mode,
    )?];
    if let Some(route_book) = &route_book {
        route_book.validate_against_composed(catalog)?;
        if let Some(method) = route_book
            .methods
            .iter()
            .find(|method| method.id == AUTHORED_METHOD_ID)
        {
            for (index, step_id) in method.step_ids.iter().enumerate() {
                let step = route_book
                    .steps
                    .iter()
                    .find(|step| &step.id == step_id)
                    .ok_or_else(|| {
                        dusklight_route_planner::PlannerContractError::new(
                            "route_book.methods.step_ids",
                            "references a missing authored step",
                        )
                    })?;
                let RouteActionRef::Transition { transition_id } = &step.action else {
                    return Err(dusklight_route_planner::PlannerContractError::new(
                        "route_book.steps.action",
                        "route-frontier inspection currently requires transition steps",
                    ));
                };
                let evaluation = assess_and_apply_transition(
                    &mut state,
                    catalog,
                    equivalence_sets,
                    transition_id,
                    evidence_mode,
                    &format!("route.frontier-replay-{index:04}"),
                )?;
                if evaluation.assessment.classification != TransitionClassification::Executable {
                    return Err(dusklight_route_planner::PlannerContractError::new(
                        "route_book.methods.step_ids",
                        format!(
                            "authored step {step_id} is {:?} at its replay boundary",
                            evaluation.assessment.classification
                        ),
                    ));
                }
                execution_path.push(path_state(&state, Some(step_id.clone()))?);
                execution_states.push(inspect_state(
                    &state,
                    &catalog.facts,
                    equivalence_sets,
                    evidence_mode,
                )?);
            }
        }
    }
    let frontier = execution_states
        .last()
        .cloned()
        .expect("start state inspected");
    let frontier_state = state.to_document()?;
    let mut transitions = Vec::with_capacity(catalog.mechanics.transitions.len());
    for transition in &catalog.mechanics.transitions {
        let mut candidate_state = state.clone();
        let evaluation = assess_and_apply_transition(
            &mut candidate_state,
            catalog,
            equivalence_sets,
            &transition.id,
            evidence_mode,
            &format!("route.frontier-candidate.{}", transition.id),
        )?;
        transitions.push(RouteFrontierTransition {
            transition_id: transition.id.clone(),
            assessment: evaluation.assessment,
            diagnostics: evaluation.diagnostics,
        });
    }
    let mut graph = if let Some(route_book) = &route_book {
        PlannerGraph::project_composed_with_route_book(catalog, route_book)?
    } else {
        PlannerGraph::project_composed(catalog)?
    };
    graph.attach_authored_execution_path(&execution_path)?;
    let graph_sha256 = graph.digest()?;
    Ok(PlannerServicePayload::RouteFrontier {
        graph: Box::new(graph),
        graph_sha256,
        frontier_state: Box::new(frontier_state),
        frontier: Box::new(frontier),
        execution_states,
        transitions,
    })
}

pub(super) fn inspect_authored_route(
    mut state: PlannerExecutionState,
    catalog: &ComposedPlannerCatalog,
    equivalence_sets: &[EquivalenceSet],
    route_book: RouteBook,
    evidence_mode: crate::RuntimeEvidenceMode,
) -> Result<AuthoredRouteInspection, dusklight_route_planner::PlannerContractError> {
    route_book.validate_against_composed(catalog)?;
    let method = route_book
        .methods
        .iter()
        .find(|method| method.id == AUTHORED_METHOD_ID)
        .ok_or_else(|| {
            dusklight_route_planner::PlannerContractError::new(
                "route_book.methods",
                "does not contain the browser-authored route method",
            )
        })?;
    let initial = state.clone();
    let mut steps = Vec::with_capacity(method.step_ids.len());
    for (index, step_id) in method.step_ids.iter().enumerate() {
        let step = route_book
            .steps
            .iter()
            .find(|step| &step.id == step_id)
            .ok_or_else(|| {
                dusklight_route_planner::PlannerContractError::new(
                    "route_book.methods.step_ids",
                    "references a missing authored step",
                )
            })?;
        let RouteActionRef::Transition { transition_id } = &step.action else {
            return Err(dusklight_route_planner::PlannerContractError::new(
                "route_book.steps.action",
                "authored route inspection currently requires transition steps",
            ));
        };
        let before = state.clone();
        let evaluation = assess_and_apply_transition(
            &mut state,
            catalog,
            equivalence_sets,
            transition_id,
            evidence_mode,
            &format!("route.inspect-{index:04}"),
        )?;
        if evaluation.assessment.classification != TransitionClassification::Executable {
            return Ok(AuthoredRouteInspection {
                steps,
                rejection: Some(AuthoredRouteRejectionInspection {
                    failed_step_id: step_id.clone(),
                    transition_id: transition_id.clone(),
                    assessment: evaluation.assessment,
                    diagnostics: evaluation.diagnostics,
                    prefix_state_change: inspect_route_state_change(
                        &initial,
                        &before,
                        catalog,
                        equivalence_sets,
                        evidence_mode,
                        &format!("route.inspect-rejection-{index:04}"),
                    )?,
                }),
            });
        }
        steps.push(AuthoredRouteStepInspection {
            step_id: step_id.clone(),
            transition_id: transition_id.clone(),
            assessment: evaluation.assessment,
            state_change: inspect_route_state_change(
                &before,
                &state,
                catalog,
                equivalence_sets,
                evidence_mode,
                &format!("route.inspect-step-{index:04}"),
            )?,
        });
    }
    Ok(AuthoredRouteInspection {
        steps,
        rejection: None,
    })
}

pub(super) fn inspect_route_state_change(
    before: &PlannerExecutionState,
    after: &PlannerExecutionState,
    catalog: &ComposedPlannerCatalog,
    equivalence_sets: &[EquivalenceSet],
    evidence_mode: crate::RuntimeEvidenceMode,
    boundary_id: &str,
) -> Result<AuthoredRouteStateChange, dusklight_route_planner::PlannerContractError> {
    Ok(AuthoredRouteStateChange {
        before: inspect_state(before, &catalog.facts, equivalence_sets, evidence_mode)?,
        after: inspect_state(after, &catalog.facts, equivalence_sets, evidence_mode)?,
        diff: inspect_state_diff(
            before,
            after,
            BoundaryKind::Custom {
                id: boundary_id.into(),
            },
            &catalog.facts,
            equivalence_sets,
            evidence_mode,
        )?,
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn suggest_transition_chain(
    state: PlannerExecutionState,
    catalog: &ComposedPlannerCatalog,
    equivalence_sets: &[EquivalenceSet],
    route_book: Option<RouteBook>,
    transition_id: &str,
    evidence_mode: crate::RuntimeEvidenceMode,
    max_depth: usize,
    max_states: usize,
) -> Result<PlannerServicePayload, dusklight_route_planner::PlannerContractError> {
    if max_depth == 0 || max_depth > 32 {
        return Err(dusklight_route_planner::PlannerContractError::new(
            "max_depth",
            "must be between 1 and 32",
        ));
    }
    if max_states == 0 || max_states > 100_000 {
        return Err(dusklight_route_planner::PlannerContractError::new(
            "max_states",
            "must be between 1 and 100000",
        ));
    }
    let frontier =
        inspect_route_frontier(state, catalog, equivalence_sets, route_book, evidence_mode)?;
    let PlannerServicePayload::RouteFrontier { frontier_state, .. } = frontier else {
        unreachable!("route-frontier inspection returns its typed payload")
    };
    let frontier_state = frontier_state.into_state()?;
    let mut initial_candidate = frontier_state.clone();
    let initial = assess_and_apply_transition(
        &mut initial_candidate,
        catalog,
        equivalence_sets,
        transition_id,
        evidence_mode,
        "route.suggest-initial",
    )?;

    let mut queue = VecDeque::from([(frontier_state.clone(), Vec::<String>::new())]);
    let mut visited = BTreeSet::from([frontier_state.digest()?]);
    let mut explored_states = 0usize;
    let mut hit_search_limit = false;
    while let Some((state, prefix)) = queue.pop_front() {
        if explored_states == max_states {
            hit_search_limit = true;
            break;
        }
        explored_states += 1;
        if prefix.len() < max_depth {
            let mut after = state.clone();
            let evaluation = assess_and_apply_transition(
                &mut after,
                catalog,
                equivalence_sets,
                transition_id,
                evidence_mode,
                &format!("route.suggest-target-{explored_states:06}"),
            )?;
            if evaluation.assessment.classification == TransitionClassification::Executable {
                let mut transition_ids = prefix;
                transition_ids.push(transition_id.into());
                return Ok(PlannerServicePayload::TransitionChainSuggestion {
                    target_transition_id: transition_id.into(),
                    transition_ids,
                    explored_states,
                    hit_search_limit: false,
                    assessment: Box::new(evaluation.assessment),
                    diagnostics: Box::new(evaluation.diagnostics),
                    after: Some(Box::new(after.to_document()?)),
                });
            }
        }
        if prefix.len() + 1 >= max_depth {
            continue;
        }
        for transition in &catalog.mechanics.transitions {
            if transition.id == transition_id {
                continue;
            }
            let mut next = state.clone();
            let evaluation = assess_and_apply_transition(
                &mut next,
                catalog,
                equivalence_sets,
                &transition.id,
                evidence_mode,
                &format!(
                    "route.suggest-producer-{explored_states:06}.{}",
                    transition.id
                ),
            )?;
            if evaluation.assessment.classification != TransitionClassification::Executable {
                continue;
            }
            let identity = next.digest()?;
            if visited.contains(&identity) {
                continue;
            }
            if visited.len() == max_states {
                hit_search_limit = true;
                break;
            }
            visited.insert(identity);
            let mut chain = prefix.clone();
            chain.push(transition.id.clone());
            queue.push_back((next, chain));
        }
    }
    if !queue.is_empty() {
        hit_search_limit = true;
    }
    Ok(PlannerServicePayload::TransitionChainSuggestion {
        target_transition_id: transition_id.into(),
        transition_ids: Vec::new(),
        explored_states,
        hit_search_limit,
        assessment: Box::new(initial.assessment),
        diagnostics: Box::new(initial.diagnostics),
        after: None,
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn append_transition_to_route_book(
    mut state: PlannerExecutionState,
    catalog: &ComposedPlannerCatalog,
    equivalence_sets: &[EquivalenceSet],
    route_book: Option<RouteBook>,
    route_book_id: String,
    route_book_label: String,
    transition_id: &str,
    evidence_mode: crate::RuntimeEvidenceMode,
) -> Result<PlannerServicePayload, dusklight_route_planner::PlannerContractError> {
    let previous_route_book_sha256 = route_book.as_ref().map(RouteBook::digest).transpose()?;
    if let Some(book) = &route_book {
        book.validate_against_composed(catalog)?;
        let method = book
            .methods
            .iter()
            .find(|method| method.id == AUTHORED_METHOD_ID);
        if method.is_none() && (!book.steps.is_empty() || !book.methods.is_empty()) {
            return Err(dusklight_route_planner::PlannerContractError::new(
                "route_book.methods",
                "does not contain the browser-authored route method and is not empty",
            ));
        }
        if let Some(method) = method {
            for (index, step_id) in method.step_ids.iter().enumerate() {
                let step = book
                    .steps
                    .iter()
                    .find(|step| &step.id == step_id)
                    .ok_or_else(|| {
                        dusklight_route_planner::PlannerContractError::new(
                            "route_book.methods.step_ids",
                            "references a missing authored step",
                        )
                    })?;
                let RouteActionRef::Transition {
                    transition_id: replay_id,
                } = &step.action
                else {
                    return Err(dusklight_route_planner::PlannerContractError::new(
                        "route_book.steps.action",
                        "authored route propagation currently requires transition steps",
                    ));
                };
                let evaluation = assess_and_apply_transition(
                    &mut state,
                    catalog,
                    equivalence_sets,
                    replay_id,
                    evidence_mode,
                    &format!("route.replay-{index:04}"),
                )?;
                if evaluation.assessment.classification != TransitionClassification::Executable {
                    return Err(dusklight_route_planner::PlannerContractError::new(
                        "route_book.steps",
                        format!(
                            "existing step {step_id} no longer composes: {:?}",
                            evaluation.assessment.classification
                        ),
                    ));
                }
            }
        }
    }

    let evaluation = assess_and_apply_transition(
        &mut state,
        catalog,
        equivalence_sets,
        transition_id,
        evidence_mode,
        "route.append",
    )?;
    if evaluation.assessment.classification != TransitionClassification::Executable {
        return Ok(PlannerServicePayload::RejectedTransitionJoin {
            assessment: Box::new(evaluation.assessment),
            diagnostics: Box::new(evaluation.diagnostics),
            closest_before: Box::new(state.to_document()?),
        });
    }
    let transition = catalog
        .mechanics
        .transitions
        .iter()
        .find(|transition| transition.id == transition_id)
        .expect("assessment resolved the transition");
    let step_id = next_authored_step_id(route_book.as_ref());
    let step = ReferenceStep {
        id: step_id.clone(),
        label: transition.label.clone(),
        scope: transition.scope.clone(),
        action: RouteActionRef::Transition {
            transition_id: transition_id.into(),
        },
        precondition: None,
        postcondition: None,
        region_id: Some(AUTHORED_REGION_ID.into()),
        annotation_ids: Vec::new(),
    };
    let book = if let Some(book) = route_book {
        let method = book
            .methods
            .iter()
            .find(|method| method.id == AUTHORED_METHOD_ID)
            .cloned();
        let scope = transition.scope.clone();
        let mut method = method.unwrap_or(PlanMethod {
            id: AUTHORED_METHOD_ID.into(),
            label: "Authored route".into(),
            scope: scope.clone(),
            region_id: AUTHORED_REGION_ID.into(),
            step_ids: Vec::new(),
        });
        method.step_ids.push(step_id.clone());
        let mut edits = vec![
            RouteBookEdit::UpsertStep { step },
            RouteBookEdit::UpsertMethod { method },
        ];
        if !book
            .regions
            .iter()
            .any(|region| region.id == AUTHORED_REGION_ID)
        {
            edits.push(RouteBookEdit::UpsertRegion {
                region: PlanRegion {
                    id: AUTHORED_REGION_ID.into(),
                    label: "Authored route".into(),
                    scope,
                    parent_region_id: None,
                    entry_predicate: None,
                    outcome_predicate: PredicateExpression::True,
                    method_ids: vec![AUTHORED_METHOD_ID.into()],
                    selected_method_id: Some(AUTHORED_METHOD_ID.into()),
                    collapse_policy: CollapsePolicy::Never,
                },
            });
        }
        RouteBookEditBatch {
            schema: ROUTE_BOOK_EDIT_BATCH_SCHEMA.into(),
            expected_route_book_sha256: book.digest()?,
            edits,
        }
        .apply_composed(&book, catalog)?
    } else {
        let refinement_stack_sha256 = Some(catalog.refinement_stack.digest()?);
        let scope = transition.scope.clone();
        let goal_id = catalog
            .mechanics
            .goals
            .first()
            .map(|goal| goal.id.clone())
            .ok_or_else(|| {
                dusklight_route_planner::PlannerContractError::new(
                    "catalog.mechanics.goals",
                    "must contain a goal before creating an authored route",
                )
            })?;
        let book = RouteBook {
            schema: ROUTE_BOOK_SCHEMA.into(),
            manifest: RouteBookManifest {
                id: route_book_id,
                version: "1.0.0".into(),
                label: route_book_label,
                author: "Route Planner".into(),
                source: "Browser-authored exact transition sequence".into(),
                scope: scope.clone(),
                refinement_stack_sha256,
            },
            goal_ids: vec![goal_id],
            constraints: Vec::new(),
            directives: Vec::new(),
            steps: vec![step],
            methods: vec![PlanMethod {
                id: AUTHORED_METHOD_ID.into(),
                label: "Authored route".into(),
                scope: scope.clone(),
                region_id: AUTHORED_REGION_ID.into(),
                step_ids: vec![step_id.clone()],
            }],
            regions: vec![PlanRegion {
                id: AUTHORED_REGION_ID.into(),
                label: "Authored route".into(),
                scope,
                parent_region_id: None,
                entry_predicate: None,
                outcome_predicate: PredicateExpression::True,
                method_ids: vec![AUTHORED_METHOD_ID.into()],
                selected_method_id: Some(AUTHORED_METHOD_ID.into()),
                collapse_policy: CollapsePolicy::Never,
            }],
            annotations: Vec::new(),
        };
        book.validate_against_composed(catalog)?;
        book
    };
    let route_book_sha256 = book.digest()?;
    Ok(PlannerServicePayload::AppendedTransition {
        book: Box::new(book),
        previous_route_book_sha256,
        route_book_sha256,
        step_id,
        assessment: Box::new(evaluation.assessment),
        after: Box::new(state.to_document()?),
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn insert_transition_after_route_step(
    mut state: PlannerExecutionState,
    catalog: &ComposedPlannerCatalog,
    equivalence_sets: &[EquivalenceSet],
    route_book: RouteBook,
    after_step_id: &str,
    transition_id: &str,
    evidence_mode: crate::RuntimeEvidenceMode,
) -> Result<PlannerServicePayload, dusklight_route_planner::PlannerContractError> {
    route_book.validate_against_composed(catalog)?;
    let previous_route_book_sha256 = route_book.digest()?;
    let method = route_book
        .methods
        .iter()
        .find(|method| method.id == AUTHORED_METHOD_ID)
        .ok_or_else(|| {
            dusklight_route_planner::PlannerContractError::new(
                "route_book.methods",
                "does not contain the browser-authored route method",
            )
        })?;
    let insertion_index = method
        .step_ids
        .iter()
        .position(|candidate| candidate == after_step_id)
        .ok_or_else(|| {
            dusklight_route_planner::PlannerContractError::new(
                "after_step_id",
                "does not name a step in the browser-authored route method",
            )
        })?;
    let transition = catalog
        .mechanics
        .transitions
        .iter()
        .find(|transition| transition.id == transition_id)
        .ok_or_else(|| {
            dusklight_route_planner::PlannerContractError::new(
                "transition_id",
                "does not name a transition in the composed catalog",
            )
        })?;
    let step_id = next_authored_step_id(Some(&route_book));
    let mut insertion_assessment = None;
    for (index, replay_step_id) in method.step_ids.iter().enumerate() {
        let step = route_book
            .steps
            .iter()
            .find(|step| &step.id == replay_step_id)
            .expect("validated route method references existing steps");
        let RouteActionRef::Transition {
            transition_id: replay_transition_id,
        } = &step.action
        else {
            return Err(dusklight_route_planner::PlannerContractError::new(
                "route_book.steps.action",
                "authored route propagation currently requires transition steps",
            ));
        };
        let evaluation = assess_and_apply_transition(
            &mut state,
            catalog,
            equivalence_sets,
            replay_transition_id,
            evidence_mode,
            &format!("route.insert-replay-{index:04}"),
        )?;
        if evaluation.assessment.classification != TransitionClassification::Executable {
            return Ok(PlannerServicePayload::RejectedRouteEdit {
                step_id: step_id.clone(),
                failed_step_id: replay_step_id.clone(),
                assessment: Box::new(evaluation.assessment),
                diagnostics: Box::new(evaluation.diagnostics),
                closest_before: Box::new(state.to_document()?),
            });
        }
        if index == insertion_index {
            let inserted = assess_and_apply_transition(
                &mut state,
                catalog,
                equivalence_sets,
                transition_id,
                evidence_mode,
                "route.insert",
            )?;
            if inserted.assessment.classification != TransitionClassification::Executable {
                return Ok(PlannerServicePayload::RejectedRouteEdit {
                    step_id: step_id.clone(),
                    failed_step_id: step_id,
                    assessment: Box::new(inserted.assessment),
                    diagnostics: Box::new(inserted.diagnostics),
                    closest_before: Box::new(state.to_document()?),
                });
            }
            insertion_assessment = Some(inserted.assessment);
        }
    }
    let assessment = insertion_assessment.expect("insertion anchor is in the authored method");
    let step = ReferenceStep {
        id: step_id.clone(),
        label: transition.label.clone(),
        scope: transition.scope.clone(),
        action: RouteActionRef::Transition {
            transition_id: transition_id.into(),
        },
        precondition: None,
        postcondition: None,
        region_id: Some(AUTHORED_REGION_ID.into()),
        annotation_ids: Vec::new(),
    };
    let mut edited_method = method.clone();
    edited_method
        .step_ids
        .insert(insertion_index + 1, step_id.clone());
    let book = RouteBookEditBatch {
        schema: ROUTE_BOOK_EDIT_BATCH_SCHEMA.into(),
        expected_route_book_sha256: previous_route_book_sha256,
        edits: vec![
            RouteBookEdit::UpsertStep { step },
            RouteBookEdit::UpsertMethod {
                method: edited_method,
            },
        ],
    }
    .apply_composed(&route_book, catalog)?;
    let route_book_sha256 = book.digest()?;
    Ok(PlannerServicePayload::InsertedTransition {
        book: Box::new(book),
        previous_route_book_sha256,
        route_book_sha256,
        step_id,
        after_step_id: after_step_id.into(),
        transition_id: transition_id.into(),
        assessment: Box::new(assessment),
        after: Box::new(state.to_document()?),
    })
}

pub(super) fn remove_authored_step_from_route_book(
    mut state: PlannerExecutionState,
    catalog: &ComposedPlannerCatalog,
    equivalence_sets: &[EquivalenceSet],
    route_book: RouteBook,
    step_id: &str,
    evidence_mode: crate::RuntimeEvidenceMode,
) -> Result<PlannerServicePayload, dusklight_route_planner::PlannerContractError> {
    route_book.validate_against_composed(catalog)?;
    let previous_route_book_sha256 = route_book.digest()?;
    let method = route_book
        .methods
        .iter()
        .find(|method| method.id == AUTHORED_METHOD_ID)
        .ok_or_else(|| {
            dusklight_route_planner::PlannerContractError::new(
                "route_book.methods",
                "does not contain the browser-authored route method",
            )
        })?;
    if !method.step_ids.iter().any(|candidate| candidate == step_id) {
        return Err(dusklight_route_planner::PlannerContractError::new(
            "step_id",
            "does not name a step in the browser-authored route method",
        ));
    }

    for (index, surviving_step_id) in method
        .step_ids
        .iter()
        .filter(|candidate| candidate.as_str() != step_id)
        .enumerate()
    {
        let step = route_book
            .steps
            .iter()
            .find(|step| &step.id == surviving_step_id)
            .expect("validated route method references existing steps");
        let RouteActionRef::Transition { transition_id } = &step.action else {
            return Err(dusklight_route_planner::PlannerContractError::new(
                "route_book.steps.action",
                "authored route propagation currently requires transition steps",
            ));
        };
        let evaluation = assess_and_apply_transition(
            &mut state,
            catalog,
            equivalence_sets,
            transition_id,
            evidence_mode,
            &format!("route.remove-replay-{index:04}"),
        )?;
        if evaluation.assessment.classification != TransitionClassification::Executable {
            return Ok(PlannerServicePayload::RejectedRouteEdit {
                step_id: step_id.into(),
                failed_step_id: surviving_step_id.clone(),
                assessment: Box::new(evaluation.assessment),
                diagnostics: Box::new(evaluation.diagnostics),
                closest_before: Box::new(state.to_document()?),
            });
        }
    }
    let after = Box::new(state.to_document()?);
    if method.step_ids.len() == 1 {
        return Ok(PlannerServicePayload::RemovedAuthoredStep {
            book: None,
            previous_route_book_sha256,
            route_book_sha256: None,
            step_id: step_id.into(),
            after,
        });
    }

    let mut edited_method = method.clone();
    edited_method
        .step_ids
        .retain(|candidate| candidate != step_id);
    let book = RouteBookEditBatch {
        schema: ROUTE_BOOK_EDIT_BATCH_SCHEMA.into(),
        expected_route_book_sha256: previous_route_book_sha256,
        edits: vec![
            RouteBookEdit::UpsertMethod {
                method: edited_method,
            },
            RouteBookEdit::RemoveStep {
                step_id: step_id.into(),
            },
        ],
    }
    .apply_composed(&route_book, catalog)?;
    let route_book_sha256 = book.digest()?;
    Ok(PlannerServicePayload::RemovedAuthoredStep {
        book: Some(Box::new(book)),
        previous_route_book_sha256,
        route_book_sha256: Some(route_book_sha256),
        step_id: step_id.into(),
        after,
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn replace_authored_step_in_route_book(
    mut state: PlannerExecutionState,
    catalog: &ComposedPlannerCatalog,
    equivalence_sets: &[EquivalenceSet],
    route_book: RouteBook,
    step_id: &str,
    transition_id: &str,
    evidence_mode: crate::RuntimeEvidenceMode,
) -> Result<PlannerServicePayload, dusklight_route_planner::PlannerContractError> {
    route_book.validate_against_composed(catalog)?;
    let previous_route_book_sha256 = route_book.digest()?;
    let method = route_book
        .methods
        .iter()
        .find(|method| method.id == AUTHORED_METHOD_ID)
        .ok_or_else(|| {
            dusklight_route_planner::PlannerContractError::new(
                "route_book.methods",
                "does not contain the browser-authored route method",
            )
        })?;
    if !method.step_ids.iter().any(|candidate| candidate == step_id) {
        return Err(dusklight_route_planner::PlannerContractError::new(
            "step_id",
            "does not name a step in the browser-authored route method",
        ));
    }
    let transition = catalog
        .mechanics
        .transitions
        .iter()
        .find(|transition| transition.id == transition_id)
        .ok_or_else(|| {
            dusklight_route_planner::PlannerContractError::new(
                "transition_id",
                "does not name a transition in the composed catalog",
            )
        })?;
    let mut replacement = route_book
        .steps
        .iter()
        .find(|step| step.id == step_id)
        .expect("validated route method references existing steps")
        .clone();
    replacement.label = transition.label.clone();
    replacement.scope = transition.scope.clone();
    replacement.action = RouteActionRef::Transition {
        transition_id: transition_id.into(),
    };

    let mut replacement_assessment = None;
    for (index, replay_step_id) in method.step_ids.iter().enumerate() {
        let step = route_book
            .steps
            .iter()
            .find(|step| &step.id == replay_step_id)
            .expect("validated route method references existing steps");
        let replay_transition_id = if replay_step_id == step_id {
            transition_id
        } else {
            let RouteActionRef::Transition { transition_id } = &step.action else {
                return Err(dusklight_route_planner::PlannerContractError::new(
                    "route_book.steps.action",
                    "authored route propagation currently requires transition steps",
                ));
            };
            transition_id
        };
        let evaluation = assess_and_apply_transition(
            &mut state,
            catalog,
            equivalence_sets,
            replay_transition_id,
            evidence_mode,
            &format!("route.replace-replay-{index:04}"),
        )?;
        if evaluation.assessment.classification != TransitionClassification::Executable {
            return Ok(PlannerServicePayload::RejectedRouteEdit {
                step_id: step_id.into(),
                failed_step_id: replay_step_id.clone(),
                assessment: Box::new(evaluation.assessment),
                diagnostics: Box::new(evaluation.diagnostics),
                closest_before: Box::new(state.to_document()?),
            });
        }
        if replay_step_id == step_id {
            replacement_assessment = Some(evaluation.assessment);
        }
    }
    let assessment = replacement_assessment.expect("authored method contains replacement step");
    let after = Box::new(state.to_document()?);
    let book = RouteBookEditBatch {
        schema: ROUTE_BOOK_EDIT_BATCH_SCHEMA.into(),
        expected_route_book_sha256: previous_route_book_sha256,
        edits: vec![RouteBookEdit::UpsertStep { step: replacement }],
    }
    .apply_composed(&route_book, catalog)?;
    let route_book_sha256 = book.digest()?;
    Ok(PlannerServicePayload::ReplacedAuthoredStep {
        book: Box::new(book),
        previous_route_book_sha256,
        route_book_sha256,
        step_id: step_id.into(),
        transition_id: transition_id.into(),
        assessment: Box::new(assessment),
        after,
    })
}

pub(super) fn next_authored_step_id(book: Option<&RouteBook>) -> String {
    let mut index = book.map_or(0, |book| book.steps.len());
    loop {
        let candidate = format!("step.route-{index:04}");
        if book.is_none_or(|book| book.steps.iter().all(|step| step.id != candidate)) {
            return candidate;
        }
        index += 1;
    }
}
