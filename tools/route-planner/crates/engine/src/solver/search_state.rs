//! Internal queue, coverage, continuation, and dominance state.

use super::*;

pub(super) struct SearchNode {
    pub(super) state: PlannerExecutionState,
    pub(super) steps: Vec<SearchStep>,
    pub(super) depth: usize,
    pub(super) satisfied_required_actions: BTreeSet<RouteActionRef>,
    pub(super) required_sequence_progress: Vec<usize>,
    pub(super) banned_sequence_progress: Vec<usize>,
    pub(super) preferred_sequence_progress: Vec<usize>,
    pub(super) satisfied_preference_ids: BTreeSet<String>,
    pub(super) preference_score: u64,
    pub(super) route_condition_unknown: bool,
    pub(super) route_costs: BTreeMap<String, u64>,
}

pub(super) struct QueueEntry {
    pub(super) node: SearchNode,
    pub(super) insertion_order: u64,
}

pub(super) struct AppliedActionBoundary {
    pub(super) action: RouteActionRef,
    pub(super) before: PlannerExecutionState,
    pub(super) after: PlannerExecutionState,
}

pub(super) struct GoalTruthCoverage {
    pub(super) expression: PredicateExpression,
    pub(super) saw_true: bool,
    pub(super) saw_false: bool,
    pub(super) saw_unknown: bool,
    pub(super) children: Vec<Self>,
}

pub(super) struct FailureDependencySets {
    pub(super) sets: Vec<BTreeSet<StateDependency>>,
    pub(super) complete: bool,
}

impl GoalTruthCoverage {
    pub(super) fn new(expression: &PredicateExpression) -> Self {
        let children = match expression {
            PredicateExpression::All { terms } | PredicateExpression::Any { terms } => {
                terms.iter().map(Self::new).collect()
            }
            PredicateExpression::Not { term } => vec![Self::new(term)],
            _ => Vec::new(),
        };
        Self {
            expression: expression.clone(),
            saw_true: false,
            saw_false: false,
            saw_unknown: false,
            children,
        }
    }

    pub(super) fn observe(&mut self, evaluator: &PredicateEvaluator<'_>) {
        match evaluator.evaluate(&self.expression) {
            EvaluatedTruth::True => self.saw_true = true,
            EvaluatedTruth::False => self.saw_false = true,
            EvaluatedTruth::Unknown => self.saw_unknown = true,
        }
        for child in &mut self.children {
            child.observe(evaluator);
        }
    }

    pub(super) fn saw(&self, desired: bool) -> bool {
        if desired {
            self.saw_true
        } else {
            self.saw_false
        }
    }

    pub(super) fn failure_sets(
        &self,
        desired: bool,
        facts: &FactCatalog,
        available: &BTreeSet<StateDependency>,
        maximum: usize,
    ) -> Result<FailureDependencySets, PlannerContractError> {
        if self.saw(desired) {
            return Ok(FailureDependencySets {
                sets: Vec::new(),
                complete: true,
            });
        }
        if self.saw_unknown {
            return Ok(FailureDependencySets {
                sets: Vec::new(),
                complete: false,
            });
        }
        let combine_union = |children: &[Self], desired| {
            let mut sets = Vec::new();
            let mut complete = true;
            for child in children {
                let result = child.failure_sets(desired, facts, available, maximum)?;
                complete &= result.complete;
                sets.extend(result.sets);
                if sets.len() > maximum {
                    return Ok(FailureDependencySets {
                        sets: Vec::new(),
                        complete: false,
                    });
                }
            }
            normalize_dependency_sets(&mut sets);
            if sets.is_empty() {
                complete = false;
            }
            Ok(FailureDependencySets { sets, complete })
        };
        let combine_product = |children: &[Self], desired| {
            let mut product = vec![BTreeSet::new()];
            let mut complete = true;
            for child in children {
                let result = child.failure_sets(desired, facts, available, maximum)?;
                complete &= result.complete;
                if result.sets.is_empty() {
                    return Ok(FailureDependencySets {
                        sets: Vec::new(),
                        complete: false,
                    });
                }
                let mut next = Vec::new();
                for left in &product {
                    for right in &result.sets {
                        if next.len() == maximum {
                            return Ok(FailureDependencySets {
                                sets: Vec::new(),
                                complete: false,
                            });
                        }
                        next.push(left.union(right).cloned().collect());
                    }
                }
                product = next;
            }
            normalize_dependency_sets(&mut product);
            Ok(FailureDependencySets {
                sets: product,
                complete,
            })
        };
        match &self.expression {
            PredicateExpression::All { .. } if desired => combine_union(&self.children, true),
            PredicateExpression::All { .. } => combine_product(&self.children, false),
            PredicateExpression::Any { .. } if desired => combine_product(&self.children, true),
            PredicateExpression::Any { .. } => combine_union(&self.children, false),
            PredicateExpression::Not { .. } => {
                self.children[0].failure_sets(!desired, facts, available, maximum)
            }
            PredicateExpression::Compare { .. } | PredicateExpression::Fact { .. } => {
                let dependencies = predicate_leaf_dependencies(facts, &self.expression)?
                    .into_iter()
                    .collect::<BTreeSet<_>>();
                if dependencies.is_empty() || !dependencies.is_subset(available) {
                    Ok(FailureDependencySets {
                        sets: Vec::new(),
                        complete: false,
                    })
                } else {
                    Ok(FailureDependencySets {
                        sets: vec![dependencies],
                        complete: true,
                    })
                }
            }
            PredicateExpression::True | PredicateExpression::False => Ok(FailureDependencySets {
                sets: Vec::new(),
                complete: false,
            }),
        }
    }
}

pub(super) fn normalize_dependency_sets(sets: &mut Vec<BTreeSet<StateDependency>>) {
    sets.sort_by(|left, right| {
        left.len()
            .cmp(&right.len())
            .then_with(|| left.iter().cmp(right.iter()))
    });
    sets.dedup();
    let mut minimal = Vec::<BTreeSet<StateDependency>>::new();
    for candidate in sets.drain(..) {
        if !minimal
            .iter()
            .any(|existing| existing.is_subset(&candidate))
        {
            minimal.push(candidate);
        }
    }
    *sets = minimal;
}

impl PartialEq for QueueEntry {
    fn eq(&self, other: &Self) -> bool {
        self.node.depth == other.node.depth
            && self.node.preference_score == other.node.preference_score
            && self.insertion_order == other.insertion_order
    }
}

impl Eq for QueueEntry {}

impl PartialOrd for QueueEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for QueueEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .node
            .depth
            .cmp(&self.node.depth)
            .then_with(|| self.node.preference_score.cmp(&other.node.preference_score))
            .then_with(|| other.insertion_order.cmp(&self.insertion_order))
    }
}

#[derive(Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct SearchIdentity {
    pub(super) continuation: ContinuationIdentity,
    pub(super) route_costs: BTreeMap<String, u64>,
}

pub(super) fn continuation_identity(
    node: &SearchNode,
    state_sha256: Digest,
) -> ContinuationIdentity {
    ContinuationIdentity {
        state_sha256,
        satisfied_required_actions: node.satisfied_required_actions.iter().cloned().collect(),
        required_sequence_progress: node.required_sequence_progress.clone(),
        banned_sequence_progress: node.banned_sequence_progress.clone(),
        preferred_sequence_progress: node.preferred_sequence_progress.clone(),
        satisfied_preference_ids: node.satisfied_preference_ids.iter().cloned().collect(),
        route_condition_unknown: node.route_condition_unknown,
    }
}

pub(super) fn resource_label(
    depth: usize,
    route_costs: &BTreeMap<String, u64>,
) -> SearchResourceLabel {
    SearchResourceLabel {
        depth,
        route_costs: route_costs
            .iter()
            .filter(|(_, value)| **value != 0)
            .map(|(axis, value)| (axis.clone(), *value))
            .collect(),
    }
}

pub(super) fn strictly_dominates(left: &SearchResourceLabel, right: &SearchResourceLabel) -> bool {
    if left.depth > right.depth {
        return false;
    }
    let mut strict = left.depth < right.depth;
    for axis in left
        .route_costs
        .keys()
        .chain(right.route_costs.keys())
        .collect::<BTreeSet<_>>()
    {
        let left_cost = left.route_costs.get(axis).copied().unwrap_or(0);
        let right_cost = right.route_costs.get(axis).copied().unwrap_or(0);
        if left_cost > right_cost {
            return false;
        }
        strict |= left_cost < right_cost;
    }
    strict
}

pub(super) fn plan_strictly_dominates(left: &SearchPlan, right: &SearchPlan) -> bool {
    let left_resources = resource_label(left.steps.len(), &left.route_costs);
    let right_resources = resource_label(right.steps.len(), &right.route_costs);
    let resources_no_worse = left_resources.depth <= right_resources.depth
        && left_resources
            .route_costs
            .keys()
            .chain(right_resources.route_costs.keys())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .all(|axis| {
                left_resources.route_costs.get(axis).copied().unwrap_or(0)
                    <= right_resources.route_costs.get(axis).copied().unwrap_or(0)
            });
    resources_no_worse
        && left.preference_score >= right.preference_score
        && (strictly_dominates(&left_resources, &right_resources)
            || left.preference_score > right.preference_score)
}

pub(super) fn search_plan_signature(plan: &SearchPlan) -> Vec<(SearchActionKind, String)> {
    plan.steps
        .iter()
        .map(|step| (step.action_kind, step.action_id.clone()))
        .collect()
}

pub(super) fn retain_nondominated_plan(plans: &mut Vec<SearchPlan>, candidate: SearchPlan) {
    if plans
        .iter()
        .any(|plan| plan == &candidate || plan_strictly_dominates(plan, &candidate))
    {
        return;
    }
    plans.retain(|plan| !plan_strictly_dominates(&candidate, plan));
    plans.push(candidate);
}

pub(super) fn order_search_plans(plans: &mut [SearchPlan]) {
    plans.sort_by(|left, right| {
        left.steps
            .len()
            .cmp(&right.steps.len())
            .then_with(|| right.preference_score.cmp(&left.preference_score))
            .then_with(|| left.route_costs.cmp(&right.route_costs))
            .then_with(|| search_plan_signature(left).cmp(&search_plan_signature(right)))
            .then_with(|| left.result_state_sha256.cmp(&right.result_state_sha256))
    });
}
