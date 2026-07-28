//! Compile route-book constraints and preferences into search policy.

use super::*;

#[derive(Clone)]
pub(super) struct RouteActionSequence {
    pub(super) steps: Vec<RouteSequenceStep>,
}

#[derive(Clone)]
pub(super) struct RouteSequenceStep {
    pub(super) action: RouteActionRef,
    pub(super) precondition: Option<PredicateExpression>,
    pub(super) postcondition: Option<PredicateExpression>,
}

pub(super) struct ActionPreference {
    pub(super) directive_id: String,
    pub(super) action: RouteActionRef,
    pub(super) weight: u32,
}

pub(super) struct MethodPreference {
    pub(super) directive_id: String,
    pub(super) sequence: RouteActionSequence,
    pub(super) weight: u32,
}

pub(super) fn compile_route_policy(
    book: &RouteBook,
    evaluator: &PredicateEvaluator<'_>,
    base_evidence_policy: EvidencePolicy,
) -> Result<RouteSearchPolicy, PlannerContractError> {
    if !evaluator.scope_applies(&book.manifest.scope) {
        return Err(PlannerContractError::new(
            "route_book.manifest.scope",
            "does not apply to the starting execution context",
        ));
    }
    let mut policy = RouteSearchPolicy {
        required_actions: BTreeSet::new(),
        banned_actions: BTreeSet::new(),
        required_predicates: Vec::new(),
        forbidden_predicates: Vec::new(),
        maintained_predicates: Vec::new(),
        required_sequences: Vec::new(),
        banned_sequences: Vec::new(),
        action_preferences: Vec::new(),
        method_preferences: Vec::new(),
        cost_limits: BTreeMap::new(),
        minimum_evidence: None,
        evidence_policy: EvidencePolicy::ESTABLISHED_ONLY,
    };
    let mut required_methods = BTreeMap::<String, RouteActionSequence>::new();
    let mut banned_methods = BTreeMap::<String, RouteActionSequence>::new();

    for constraint in book
        .constraints
        .iter()
        .filter(|constraint| evaluator.scope_applies(&constraint.scope))
    {
        match &constraint.constraint {
            PathConstraint::RequirePredicate { predicate } => {
                policy.required_predicates.push(predicate.clone());
            }
            PathConstraint::ForbidPredicate { predicate } => {
                policy.forbidden_predicates.push(predicate.clone());
            }
            PathConstraint::MaintainPredicate { predicate } => {
                policy.maintained_predicates.push(predicate.clone());
            }
            PathConstraint::RequireTransition { transition_id } => {
                policy.required_actions.insert(RouteActionRef::Transition {
                    transition_id: transition_id.clone(),
                });
            }
            PathConstraint::ForbidTransition { transition_id } => {
                policy.banned_actions.insert(RouteActionRef::Transition {
                    transition_id: transition_id.clone(),
                });
            }
            PathConstraint::RequireTechnique { technique_id } => {
                policy.required_actions.insert(RouteActionRef::Technique {
                    technique_id: technique_id.clone(),
                });
            }
            PathConstraint::ForbidTechnique { technique_id } => {
                policy.banned_actions.insert(RouteActionRef::Technique {
                    technique_id: technique_id.clone(),
                });
            }
            PathConstraint::CostAtMost { axis, maximum } => {
                policy
                    .cost_limits
                    .entry(axis.clone())
                    .and_modify(|current| *current = (*current).min(*maximum))
                    .or_insert(*maximum);
            }
            PathConstraint::EvidenceAtLeast { minimum } => {
                let minimum = parse_evidence_minimum(minimum)?;
                if policy
                    .minimum_evidence
                    .is_none_or(|current| evidence_quality(minimum) > evidence_quality(current))
                {
                    policy.minimum_evidence = Some(minimum);
                }
            }
        }
    }

    for directive in book
        .directives
        .iter()
        .filter(|directive| evaluator.scope_applies(&directive.scope))
    {
        match &directive.directive {
            RouteDirectiveKind::PinAction { action } => {
                require_searchable_action(action, &directive.id)?;
                policy.required_actions.insert(action.clone());
            }
            RouteDirectiveKind::BanAction { action } => {
                require_searchable_action(action, &directive.id)?;
                policy.banned_actions.insert(action.clone());
            }
            RouteDirectiveKind::PinMethod { method_id } => {
                let sequence = compile_method_sequence(book, method_id, evaluator, true)?
                    .ok_or_else(|| {
                        PlannerContractError::new(
                            "route_book.directives.method_id",
                            "required method unexpectedly had no active sequence",
                        )
                    })?;
                required_methods.insert(method_id.clone(), sequence);
            }
            RouteDirectiveKind::BanMethod { method_id } => {
                if let Some(sequence) = compile_method_sequence(book, method_id, evaluator, false)?
                {
                    banned_methods.insert(method_id.clone(), sequence);
                }
            }
            RouteDirectiveKind::PreferAction { action, weight } => {
                require_searchable_action(action, &directive.id)?;
                policy.action_preferences.push(ActionPreference {
                    directive_id: directive.id.clone(),
                    action: action.clone(),
                    weight: *weight,
                });
            }
            RouteDirectiveKind::PreferMethod { method_id, weight } => {
                if let Some(sequence) = compile_method_sequence(book, method_id, evaluator, false)?
                {
                    policy.method_preferences.push(MethodPreference {
                        directive_id: directive.id.clone(),
                        sequence,
                        weight: *weight,
                    });
                }
            }
        }
    }

    for region in book
        .regions
        .iter()
        .filter(|region| evaluator.scope_applies(&region.scope))
    {
        if let Some(method_id) = &region.selected_method_id {
            let sequence =
                compile_method_sequence(book, method_id, evaluator, true)?.ok_or_else(|| {
                    PlannerContractError::new(
                        "route_book.regions.selected_method_id",
                        "selected method unexpectedly had no active sequence",
                    )
                })?;
            required_methods.insert(method_id.clone(), sequence);
        }
    }
    if let Some(action) = policy
        .required_actions
        .intersection(&policy.banned_actions)
        .next()
    {
        return Err(PlannerContractError::new(
            "route_book",
            format!("action {action:?} is both required and banned"),
        ));
    }
    if let Some(method_id) = required_methods
        .keys()
        .find(|method_id| banned_methods.contains_key(*method_id))
    {
        return Err(PlannerContractError::new(
            "route_book",
            format!("method {method_id} is both required and banned"),
        ));
    }
    if let Some((method_id, action)) = required_methods.iter().find_map(|(method_id, sequence)| {
        sequence
            .steps
            .iter()
            .find(|step| policy.banned_actions.contains(&step.action))
            .map(|step| (method_id, &step.action))
    }) {
        return Err(PlannerContractError::new(
            "route_book",
            format!("required method {method_id} contains banned action {action:?}"),
        ));
    }
    policy.required_sequences = required_methods.into_values().collect();
    policy.banned_sequences = banned_methods.into_values().collect();
    policy.evidence_policy =
        evidence_policy_for_minimum(base_evidence_policy, policy.minimum_evidence);
    Ok(policy)
}

pub(super) fn parse_evidence_minimum(value: &str) -> Result<TruthStatus, PlannerContractError> {
    match value {
        "established" => Ok(TruthStatus::Established),
        "contested" => Ok(TruthStatus::Contested),
        "hypothetical" => Ok(TruthStatus::Hypothetical),
        _ => Err(PlannerContractError::new(
            "route_book.constraints.minimum",
            "must be established, contested, or hypothetical",
        )),
    }
}

pub(super) fn evidence_quality(status: TruthStatus) -> u8 {
    match status {
        TruthStatus::Established => 3,
        TruthStatus::Contested => 2,
        TruthStatus::Hypothetical => 1,
        TruthStatus::Unknown => 0,
    }
}

pub(super) fn evidence_policy_for_minimum(
    base: EvidencePolicy,
    minimum: Option<TruthStatus>,
) -> EvidencePolicy {
    let required = match minimum {
        Some(TruthStatus::Established) => EvidencePolicy::ESTABLISHED_ONLY,
        Some(TruthStatus::Contested) => EvidencePolicy {
            allow_contested: true,
            allow_hypothetical: false,
        },
        Some(TruthStatus::Hypothetical) | None => EvidencePolicy::RESEARCH,
        Some(TruthStatus::Unknown) => EvidencePolicy::ESTABLISHED_ONLY,
    };
    EvidencePolicy {
        allow_contested: base.allow_contested && required.allow_contested,
        allow_hypothetical: base.allow_hypothetical && required.allow_hypothetical,
    }
}

pub(super) fn compile_method_sequence(
    book: &RouteBook,
    method_id: &str,
    evaluator: &PredicateEvaluator<'_>,
    required: bool,
) -> Result<Option<RouteActionSequence>, PlannerContractError> {
    let method = book
        .methods
        .iter()
        .find(|method| method.id == method_id)
        .ok_or_else(|| {
            PlannerContractError::new(
                "route_book.directives.method_id",
                format!("references unknown method {method_id}"),
            )
        })?;
    if !evaluator.scope_applies(&method.scope) {
        if required {
            return Err(PlannerContractError::new(
                "route_book.methods.scope",
                format!("required method {method_id} does not apply to the starting context"),
            ));
        }
        return Ok(None);
    }
    let mut steps = Vec::with_capacity(method.step_ids.len());
    for step_id in &method.step_ids {
        let step = book
            .steps
            .iter()
            .find(|step| step.id == *step_id)
            .ok_or_else(|| {
                PlannerContractError::new(
                    "route_book.methods.step_ids",
                    format!("references unknown step {step_id}"),
                )
            })?;
        require_searchable_action(&step.action, method_id)?;
        steps.push(RouteSequenceStep {
            action: step.action.clone(),
            precondition: step.precondition.clone(),
            postcondition: step.postcondition.clone(),
        });
    }
    Ok(Some(RouteActionSequence { steps }))
}

pub(super) fn require_searchable_action(
    action: &RouteActionRef,
    directive_id: &str,
) -> Result<(), PlannerContractError> {
    if matches!(
        action,
        RouteActionRef::Transition { .. }
            | RouteActionRef::Technique { .. }
            | RouteActionRef::Resolver { .. }
            | RouteActionRef::Writer { .. }
    ) {
        Ok(())
    } else {
        Err(PlannerContractError::new(
            "route_book.directives",
            format!(
                "directive {directive_id} references an action kind the bounded forward solver cannot execute"
            ),
        ))
    }
}

pub(super) fn evaluate_all(
    evaluator: &PredicateEvaluator<'_>,
    predicates: &[PredicateExpression],
) -> EvaluatedTruth {
    let mut result = EvaluatedTruth::True;
    for predicate in predicates {
        match evaluator.evaluate(predicate) {
            EvaluatedTruth::False => return EvaluatedTruth::False,
            EvaluatedTruth::Unknown => result = EvaluatedTruth::Unknown,
            EvaluatedTruth::True => {}
        }
    }
    result
}

pub(super) struct RouteSearchPolicy {
    pub(super) required_actions: BTreeSet<RouteActionRef>,
    pub(super) banned_actions: BTreeSet<RouteActionRef>,
    pub(super) required_predicates: Vec<PredicateExpression>,
    pub(super) forbidden_predicates: Vec<PredicateExpression>,
    pub(super) maintained_predicates: Vec<PredicateExpression>,
    pub(super) required_sequences: Vec<RouteActionSequence>,
    pub(super) banned_sequences: Vec<RouteActionSequence>,
    pub(super) action_preferences: Vec<ActionPreference>,
    pub(super) method_preferences: Vec<MethodPreference>,
    pub(super) cost_limits: BTreeMap<String, u64>,
    pub(super) minimum_evidence: Option<TruthStatus>,
    pub(super) evidence_policy: EvidencePolicy,
}
