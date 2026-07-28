//! Admit successor states and derive their route/action evidence.

use super::*;

impl<'a> ForwardSolver<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn enqueue_if_new(
        &self,
        queue: &mut BinaryHeap<QueueEntry>,
        visited: &BTreeSet<SearchIdentity>,
        node: &SearchNode,
        next: PlannerExecutionState,
        boundaries: &[AppliedActionBoundary],
        route_policy: Option<&RouteSearchPolicy>,
        insertion_order: u64,
        authorization: Option<&mut AuthorizationRecorder>,
        mut step: SearchStep,
    ) -> Result<bool, PlannerContractError> {
        let result = next.semantic_digest()?;
        let mut satisfied_required_actions = node.satisfied_required_actions.clone();
        let mut required_sequence_progress = node.required_sequence_progress.clone();
        let mut banned_sequence_progress = node.banned_sequence_progress.clone();
        let mut preferred_sequence_progress = node.preferred_sequence_progress.clone();
        let mut satisfied_preference_ids = node.satisfied_preference_ids.clone();
        let mut preference_score = node.preference_score;
        let mut route_condition_unknown = node.route_condition_unknown;
        let mut saw_unknown_condition = false;
        let mut route_costs = node.route_costs.clone();
        for boundary in boundaries {
            let RouteActionRef::Technique { technique_id } = &boundary.action else {
                continue;
            };
            let technique = self
                .mechanics
                .techniques
                .iter()
                .find(|technique| technique.id == *technique_id)
                .ok_or_else(|| {
                    PlannerContractError::new(
                        "solver.cost",
                        format!("references unknown technique {technique_id}"),
                    )
                })?;
            for (axis, increment) in &technique.cost.axes {
                if *increment == 0 {
                    continue;
                }
                let total = route_costs.entry(axis.clone()).or_default();
                *total = total.checked_add(*increment).ok_or_else(|| {
                    PlannerContractError::new("solver.cost", format!("axis {axis} overflowed u64"))
                })?;
            }
        }
        if let Some(policy) = route_policy {
            if policy
                .cost_limits
                .iter()
                .any(|(axis, maximum)| route_costs.get(axis).copied().unwrap_or(0) > *maximum)
            {
                return Ok(false);
            }
            for boundary in boundaries {
                if policy.required_actions.contains(&boundary.action) {
                    satisfied_required_actions.insert(boundary.action.clone());
                }
                for preference in &policy.action_preferences {
                    if preference.action == boundary.action
                        && satisfied_preference_ids.insert(preference.directive_id.clone())
                    {
                        preference_score =
                            preference_score.saturating_add(u64::from(preference.weight));
                    }
                }
                for (preference, progress) in policy
                    .method_preferences
                    .iter()
                    .zip(preferred_sequence_progress.iter_mut())
                {
                    if let Some(expected) = preference.sequence.steps.get(*progress)
                        && expected.action == boundary.action
                        && self.evaluate_step_boundary(
                            expected,
                            boundary,
                            policy.evidence_policy,
                        )? == EvaluatedTruth::True
                    {
                        *progress += 1;
                        if *progress == preference.sequence.steps.len()
                            && satisfied_preference_ids.insert(preference.directive_id.clone())
                        {
                            preference_score =
                                preference_score.saturating_add(u64::from(preference.weight));
                        }
                    }
                }
            }
            saw_unknown_condition |= self.advance_sequence_progress(
                &policy.required_sequences,
                &mut required_sequence_progress,
                boundaries,
                policy.evidence_policy,
            )?;
            let banned_unknown = self.advance_sequence_progress(
                &policy.banned_sequences,
                &mut banned_sequence_progress,
                boundaries,
                policy.evidence_policy,
            )?;
            saw_unknown_condition |= banned_unknown;
            route_condition_unknown |= banned_unknown;
            if policy
                .banned_sequences
                .iter()
                .zip(&banned_sequence_progress)
                .any(|(sequence, progress)| *progress == sequence.steps.len())
            {
                return Ok(saw_unknown_condition);
            }
        }
        let continuation = ContinuationIdentity {
            state_sha256: result,
            satisfied_required_actions: satisfied_required_actions.iter().cloned().collect(),
            required_sequence_progress: required_sequence_progress.clone(),
            banned_sequence_progress: banned_sequence_progress.clone(),
            preferred_sequence_progress: preferred_sequence_progress.clone(),
            satisfied_preference_ids: satisfied_preference_ids.iter().cloned().collect(),
            route_condition_unknown,
        };
        let search_identity = SearchIdentity {
            continuation,
            route_costs: route_costs.clone(),
        };
        let derivation_evidence_policy = route_policy
            .map_or(self.options.evidence_policy, |policy| {
                policy.evidence_policy
            });
        step.action_derivations = boundaries
            .iter()
            .map(|boundary| self.action_derivation(boundary, derivation_evidence_policy))
            .collect::<Result<Vec<_>, _>>()?;
        let obligation_ids = step
            .discharged_obligation_ids
            .iter()
            .chain(&step.outstanding_obligation_ids)
            .chain(&step.unknown_obligation_ids)
            .chain(&step.introduced_obligation_ids)
            .collect::<BTreeSet<_>>();
        step.obligation_derivations = self
            .mechanics
            .obligations
            .iter()
            .filter(|obligation| obligation_ids.contains(&obligation.id))
            .cloned()
            .collect();
        step.result_state_sha256 = result;
        if let Some(recorder) = authorization {
            recorder.observe_state(
                result,
                next.digest()?,
                next.snapshot.digest()?,
                node.depth + 1,
                false,
            );
            recorder.record_edge(&step);
        }
        if visited.contains(&search_identity) {
            return Ok(saw_unknown_condition);
        }
        let mut steps = node.steps.clone();
        steps.push(step);
        queue.push(QueueEntry {
            node: SearchNode {
                state: next,
                steps,
                depth: node.depth + 1,
                satisfied_required_actions,
                required_sequence_progress,
                banned_sequence_progress,
                preferred_sequence_progress,
                satisfied_preference_ids,
                preference_score,
                route_condition_unknown,
                route_costs,
            },
            insertion_order,
        });
        Ok(saw_unknown_condition)
    }

    pub(super) fn action_derivation(
        &self,
        boundary: &AppliedActionBoundary,
        evidence_policy: EvidencePolicy,
    ) -> Result<ActionDerivation, PlannerContractError> {
        let (precondition, operations) = match &boundary.action {
            RouteActionRef::Transition { transition_id } => {
                let transition = self
                    .mechanics
                    .transitions
                    .iter()
                    .find(|transition| transition.id == *transition_id)
                    .ok_or_else(|| {
                        PlannerContractError::new(
                            "solver.action_derivation",
                            format!("references unknown transition {transition_id}"),
                        )
                    })?;
                (
                    transition.activation.hard_guards.clone(),
                    transition.activation.effects.clone(),
                )
            }
            RouteActionRef::Technique { technique_id } => {
                let technique = self
                    .mechanics
                    .techniques
                    .iter()
                    .find(|technique| technique.id == *technique_id)
                    .ok_or_else(|| {
                        PlannerContractError::new(
                            "solver.action_derivation",
                            format!("references unknown technique {technique_id}"),
                        )
                    })?;
                (
                    technique.prerequisites.clone(),
                    technique.operations.clone(),
                )
            }
            RouteActionRef::Resolver { resolver_id } => {
                let resolver = self
                    .mechanics
                    .resolvers
                    .iter()
                    .find(|resolver| resolver.id == *resolver_id)
                    .ok_or_else(|| {
                        PlannerContractError::new(
                            "solver.action_derivation",
                            format!("references unknown resolver {resolver_id}"),
                        )
                    })?;
                (
                    resolver.applicable_when.clone(),
                    resolver.operations.clone(),
                )
            }
            RouteActionRef::Writer { writer_id } => {
                let writer = self
                    .mechanics
                    .writers
                    .iter()
                    .find(|writer| writer.id == *writer_id)
                    .ok_or_else(|| {
                        PlannerContractError::new(
                            "solver.action_derivation",
                            format!("references unknown writer {writer_id}"),
                        )
                    })?;
                (writer.activation.clone(), vec![writer.operation.clone()])
            }
            RouteActionRef::Microtrace { microtrace_id } => {
                let microtrace = self
                    .mechanics
                    .microtraces
                    .iter()
                    .find(|microtrace| microtrace.id == *microtrace_id)
                    .ok_or_else(|| {
                        PlannerContractError::new(
                            "solver.action_derivation",
                            format!("references unknown microtrace {microtrace_id}"),
                        )
                    })?;
                (
                    microtrace.precondition.clone(),
                    microtrace.operations.clone(),
                )
            }
        };
        let evaluator = PredicateEvaluator::new(
            &boundary.before.snapshot,
            self.facts,
            self.equivalence_sets,
            &boundary.before.gate_states,
            evidence_policy,
        )?;
        Ok(ActionDerivation {
            action: boundary.action.clone(),
            precondition_result: evaluator.evaluate(&precondition),
            precondition,
            operations,
            source_state_sha256: boundary.before.semantic_digest()?,
            result_state_sha256: boundary.after.semantic_digest()?,
        })
    }

    pub(super) fn advance_sequence_progress(
        &self,
        sequences: &[RouteActionSequence],
        progress: &mut [usize],
        boundaries: &[AppliedActionBoundary],
        evidence_policy: EvidencePolicy,
    ) -> Result<bool, PlannerContractError> {
        let mut saw_unknown = false;
        for boundary in boundaries {
            for (sequence, progress) in sequences.iter().zip(progress.iter_mut()) {
                let Some(expected) = sequence.steps.get(*progress) else {
                    continue;
                };
                if expected.action != boundary.action {
                    continue;
                }
                match self.evaluate_step_boundary(expected, boundary, evidence_policy)? {
                    EvaluatedTruth::True => *progress += 1,
                    EvaluatedTruth::Unknown => saw_unknown = true,
                    EvaluatedTruth::False => {}
                }
            }
        }
        Ok(saw_unknown)
    }

    pub(super) fn evaluate_step_boundary(
        &self,
        step: &RouteSequenceStep,
        boundary: &AppliedActionBoundary,
        evidence_policy: EvidencePolicy,
    ) -> Result<EvaluatedTruth, PlannerContractError> {
        let before = PredicateEvaluator::new(
            &boundary.before.snapshot,
            self.facts,
            self.equivalence_sets,
            &boundary.before.gate_states,
            evidence_policy,
        )?;
        let after = PredicateEvaluator::new(
            &boundary.after.snapshot,
            self.facts,
            self.equivalence_sets,
            &boundary.after.gate_states,
            evidence_policy,
        )?;
        Ok(and_truth(
            step.precondition
                .as_ref()
                .map_or(EvaluatedTruth::True, |predicate| before.evaluate(predicate)),
            step.postcondition
                .as_ref()
                .map_or(EvaluatedTruth::True, |predicate| after.evaluate(predicate)),
        ))
    }
}
