//! Explain blocked transitions and compute bounded producer cut sets.

use super::*;

pub(super) fn blocked_witness(
    facts: &FactCatalog,
    mechanics: &MechanicsCatalog,
    transition: &CandidateTransition,
    source: &PlannerExecutionState,
    resolution: &FeasibilityResolution,
    assessment: &TransitionAssessment,
    readers: (&[ReaderResult], &[String]),
) -> Result<BlockedTransitionWitness, PlannerContractError> {
    let (reader_results, unknown_reader_ids) = readers;
    let evidence_dependencies = transition_evidence_dependencies(
        facts,
        mechanics,
        transition,
        resolution,
        assessment,
        reader_results,
        unknown_reader_ids,
    );
    let weakest_evidence = weakest_evidence(&evidence_dependencies);
    Ok(BlockedTransitionWitness {
        transition_id: transition.id.clone(),
        source_state_sha256: source.semantic_digest()?,
        classification: assessment.classification,
        hard_guard: assessment.hard_guard,
        selected_resolver_ids: resolution.applied_resolver_ids.clone(),
        selected_technique_ids: resolution.applicable_technique_ids.clone(),
        active_obstruction_ids: resolution.active_obstruction_ids.clone(),
        unknown_obstruction_ids: resolution.unknown_obstruction_ids.clone(),
        discharged_obligation_ids: resolution
            .discharged_obligation_ids
            .iter()
            .cloned()
            .collect(),
        outstanding_obligation_ids: assessment.outstanding_obligation_ids.clone(),
        unknown_obligation_ids: assessment.unknown_obligation_ids.clone(),
        supporting_microtrace_ids: resolution
            .supporting_microtrace_ids
            .iter()
            .cloned()
            .collect(),
        unknown_requirement_ids: assessment.unknown_requirement_ids.clone(),
        reader_results: reader_results.to_vec(),
        unknown_reader_ids: unknown_reader_ids.to_vec(),
        evidence_dependencies,
        weakest_evidence,
        hard_guard_expression: transition.activation.hard_guards.clone(),
        effect_operations: transition.activation.effects.clone(),
        obligation_derivations: mechanics
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
        unknown_requirements: transition.activation.unknown_requirements.clone(),
    })
}

pub(super) fn assess_transition_readers(
    evaluator: &PredicateEvaluator<'_>,
    mechanics: &MechanicsCatalog,
    transition_id: &str,
) -> (Vec<ReaderResult>, Vec<String>) {
    let mut results = Vec::new();
    let mut unknown = Vec::new();
    for reader in mechanics
        .readers
        .iter()
        .filter(|reader| reader.consuming_transition_id == transition_id)
    {
        let assessment = evaluator.assess_reader(reader);
        if !assessment.scope_applies {
            continue;
        }
        if !assessment.evidence_permitted {
            unknown.push(reader.id.clone());
            continue;
        }
        let Some(source_value) = assessment.source_value else {
            unknown.push(reader.id.clone());
            continue;
        };
        results.push(ReaderResult {
            reader_id: reader.id.clone(),
            source_value,
            interpretation: assessment.interpretation,
        });
    }
    (results, unknown)
}

pub(super) fn technique_evidence_dependencies(
    facts: &FactCatalog,
    mechanics: &MechanicsCatalog,
    technique: &crate::transition::Technique,
) -> Vec<EvidenceDependency> {
    let mut dependencies = BTreeMap::new();
    insert_evidence(
        &mut dependencies,
        EvidenceDependencyKind::Technique,
        &technique.id,
        &technique.evidence,
    );
    collect_predicate_evidence(
        &technique.prerequisites,
        facts,
        &mut dependencies,
        &mut BTreeSet::new(),
    );
    for obligation_id in technique
        .discharged_obligation_ids
        .iter()
        .chain(&technique.introduced_obligation_ids)
    {
        if let Some(obligation) = mechanics
            .obligations
            .iter()
            .find(|obligation| obligation.id == *obligation_id)
        {
            collect_obligation_evidence(obligation, facts, &mut dependencies);
        }
    }
    dependencies
        .into_iter()
        .map(
            |((dependency_kind, record_id), evidence)| EvidenceDependency {
                dependency_kind,
                record_id,
                evidence,
            },
        )
        .collect()
}

pub(super) fn resolver_evidence_dependencies(
    facts: &FactCatalog,
    resolver: &crate::transition::ObstructionResolver,
) -> Vec<EvidenceDependency> {
    let mut dependencies = BTreeMap::new();
    insert_evidence(
        &mut dependencies,
        EvidenceDependencyKind::Resolver,
        &resolver.id,
        &resolver.evidence,
    );
    collect_predicate_evidence(
        &resolver.applicable_when,
        facts,
        &mut dependencies,
        &mut BTreeSet::new(),
    );
    dependencies
        .into_iter()
        .map(
            |((dependency_kind, record_id), evidence)| EvidenceDependency {
                dependency_kind,
                record_id,
                evidence,
            },
        )
        .collect()
}

pub(super) fn reconstruction_evidence_dependencies(
    facts: &FactCatalog,
    rule: &crate::transition::ActorReconstructionRule,
) -> Vec<EvidenceDependency> {
    let mut dependencies = BTreeMap::new();
    insert_evidence(
        &mut dependencies,
        EvidenceDependencyKind::Reconstruction,
        &rule.id,
        &rule.evidence,
    );
    collect_predicate_evidence(
        &rule.instantiate_when,
        facts,
        &mut dependencies,
        &mut BTreeSet::new(),
    );
    dependencies
        .into_iter()
        .map(
            |((dependency_kind, record_id), evidence)| EvidenceDependency {
                dependency_kind,
                record_id,
                evidence,
            },
        )
        .collect()
}

pub(super) fn writer_evidence_dependencies(
    facts: &FactCatalog,
    mechanics: &MechanicsCatalog,
    writer: &crate::transition::WriterRule,
    assessment: &WriterAssessment,
) -> Vec<EvidenceDependency> {
    let mut dependencies = BTreeMap::new();
    insert_evidence(
        &mut dependencies,
        EvidenceDependencyKind::Writer,
        &writer.id,
        &writer.evidence,
    );
    collect_predicate_evidence(
        &writer.activation,
        facts,
        &mut dependencies,
        &mut BTreeSet::new(),
    );
    for gate in mechanics.gates.iter().filter(|gate| {
        gate.blocked_writer_ids
            .iter()
            .any(|writer_id| writer_id == &writer.id)
    }) {
        insert_evidence(
            &mut dependencies,
            EvidenceDependencyKind::Gate,
            &gate.id,
            &gate.evidence,
        );
        collect_predicate_evidence(
            &gate.active_when,
            facts,
            &mut dependencies,
            &mut BTreeSet::new(),
        );
    }
    for gate_id in assessment
        .active_gate_ids
        .iter()
        .chain(&assessment.unknown_gate_ids)
    {
        debug_assert!(mechanics.gates.iter().any(|gate| gate.id == *gate_id));
    }
    dependencies
        .into_iter()
        .map(
            |((dependency_kind, record_id), evidence)| EvidenceDependency {
                dependency_kind,
                record_id,
                evidence,
            },
        )
        .collect()
}

pub(super) fn transition_evidence_dependencies(
    facts: &FactCatalog,
    mechanics: &MechanicsCatalog,
    transition: &CandidateTransition,
    resolution: &FeasibilityResolution,
    assessment: &TransitionAssessment,
    reader_results: &[ReaderResult],
    unknown_reader_ids: &[String],
) -> Vec<EvidenceDependency> {
    let mut dependencies = BTreeMap::new();
    insert_evidence(
        &mut dependencies,
        EvidenceDependencyKind::Transition,
        &transition.id,
        &transition.evidence,
    );
    collect_predicate_evidence(
        &transition.activation.hard_guards,
        facts,
        &mut dependencies,
        &mut BTreeSet::new(),
    );
    for requirement_id in &assessment.unknown_requirement_ids {
        if let Some(requirement) = transition
            .activation
            .unknown_requirements
            .iter()
            .find(|requirement| requirement.id == *requirement_id)
        {
            insert_evidence(
                &mut dependencies,
                EvidenceDependencyKind::UnknownRequirement,
                &requirement.id,
                &requirement.evidence,
            );
        }
    }
    for obstruction_id in resolution
        .active_obstruction_ids
        .iter()
        .chain(&resolution.unknown_obstruction_ids)
    {
        if let Some(obstruction) = mechanics
            .obstructions
            .iter()
            .find(|obstruction| obstruction.id == *obstruction_id)
        {
            insert_evidence(
                &mut dependencies,
                EvidenceDependencyKind::Obstruction,
                &obstruction.id,
                &obstruction.evidence,
            );
            collect_predicate_evidence(
                &obstruction.active_when,
                facts,
                &mut dependencies,
                &mut BTreeSet::new(),
            );
        }
    }
    for resolver_id in &resolution.applied_resolver_ids {
        if let Some(resolver) = mechanics
            .resolvers
            .iter()
            .find(|resolver| resolver.id == *resolver_id)
        {
            insert_evidence(
                &mut dependencies,
                EvidenceDependencyKind::Resolver,
                &resolver.id,
                &resolver.evidence,
            );
            collect_predicate_evidence(
                &resolver.applicable_when,
                facts,
                &mut dependencies,
                &mut BTreeSet::new(),
            );
        }
    }
    for technique_id in &resolution.applicable_technique_ids {
        if let Some(technique) = mechanics
            .techniques
            .iter()
            .find(|technique| technique.id == *technique_id)
        {
            for dependency in technique_evidence_dependencies(facts, mechanics, technique) {
                dependencies.insert(
                    (dependency.dependency_kind, dependency.record_id),
                    dependency.evidence,
                );
            }
        }
    }
    let obligation_ids = resolution
        .discharged_obligation_ids
        .iter()
        .chain(&assessment.outstanding_obligation_ids)
        .chain(&assessment.unknown_obligation_ids)
        .collect::<BTreeSet<_>>();
    for obligation_id in obligation_ids {
        if let Some(obligation) = mechanics
            .obligations
            .iter()
            .find(|obligation| obligation.id == *obligation_id)
        {
            collect_obligation_evidence(obligation, facts, &mut dependencies);
        }
    }
    for microtrace_id in &resolution.supporting_microtrace_ids {
        if let Some(microtrace) = mechanics
            .microtraces
            .iter()
            .find(|microtrace| microtrace.id == *microtrace_id)
        {
            insert_evidence(
                &mut dependencies,
                EvidenceDependencyKind::Microtrace,
                &microtrace.id,
                &microtrace.evidence,
            );
            collect_predicate_evidence(
                &microtrace.precondition,
                facts,
                &mut dependencies,
                &mut BTreeSet::new(),
            );
            collect_predicate_evidence(
                &microtrace.postcondition,
                facts,
                &mut dependencies,
                &mut BTreeSet::new(),
            );
        }
    }
    let reader_ids = reader_results
        .iter()
        .map(|result| result.reader_id.as_str())
        .chain(unknown_reader_ids.iter().map(String::as_str))
        .collect::<BTreeSet<_>>();
    for reader_id in reader_ids {
        if let Some(reader) = mechanics
            .readers
            .iter()
            .find(|reader| reader.id == reader_id)
        {
            insert_evidence(
                &mut dependencies,
                EvidenceDependencyKind::Reader,
                &reader.id,
                &reader.evidence,
            );
            if let Some(fact_id) = &reader.interpretation_fact_id {
                collect_predicate_evidence(
                    &PredicateExpression::Fact {
                        fact_id: fact_id.clone(),
                    },
                    facts,
                    &mut dependencies,
                    &mut BTreeSet::new(),
                );
            }
        }
    }
    dependencies
        .into_iter()
        .map(
            |((dependency_kind, record_id), evidence)| EvidenceDependency {
                dependency_kind,
                record_id,
                evidence,
            },
        )
        .collect()
}

pub(super) fn collect_obligation_evidence(
    obligation: &crate::transition::FeasibilityObligation,
    facts: &FactCatalog,
    dependencies: &mut BTreeMap<(EvidenceDependencyKind, String), RuleEvidence>,
) {
    insert_evidence(
        dependencies,
        EvidenceDependencyKind::Obligation,
        &obligation.id,
        &obligation.evidence,
    );
    if let crate::transition::ObligationDetail::CompoundInteraction { branches, .. } =
        &obligation.detail
    {
        for branch in branches {
            collect_predicate_evidence(&branch.when, facts, dependencies, &mut BTreeSet::new());
            collect_predicate_evidence(
                &branch.pose_predicate,
                facts,
                dependencies,
                &mut BTreeSet::new(),
            );
        }
        return;
    }
    let predicate = match &obligation.detail {
        crate::transition::ObligationDetail::Predicate { predicate } => Some(predicate),
        crate::transition::ObligationDetail::Interaction { pose_predicate, .. } => {
            Some(pose_predicate)
        }
        crate::transition::ObligationDetail::Temporal { precondition, .. } => Some(precondition),
        crate::transition::ObligationDetail::CompoundInteraction { .. } => unreachable!(),
        crate::transition::ObligationDetail::Geometry { .. }
        | crate::transition::ObligationDetail::PlaneSide { .. }
        | crate::transition::ObligationDetail::Facing { .. }
        | crate::transition::ObligationDetail::Unresolved { .. } => None,
    };
    if let Some(predicate) = predicate {
        collect_predicate_evidence(predicate, facts, dependencies, &mut BTreeSet::new());
    }
}

pub(super) fn collect_predicate_evidence(
    predicate: &PredicateExpression,
    facts: &FactCatalog,
    dependencies: &mut BTreeMap<(EvidenceDependencyKind, String), RuleEvidence>,
    visiting: &mut BTreeSet<String>,
) {
    match predicate {
        PredicateExpression::Fact { fact_id } => {
            if !visiting.insert(fact_id.clone()) {
                return;
            }
            if let Some(alias) = facts.aliases.iter().find(|alias| alias.id == *fact_id) {
                insert_evidence(
                    dependencies,
                    EvidenceDependencyKind::Fact,
                    &alias.id,
                    &alias.evidence,
                );
            } else if let Some(fact) = facts.derived_facts.iter().find(|fact| fact.id == *fact_id) {
                insert_evidence(
                    dependencies,
                    EvidenceDependencyKind::Fact,
                    &fact.id,
                    &fact.evidence,
                );
                collect_predicate_evidence(&fact.rule, facts, dependencies, visiting);
            }
            visiting.remove(fact_id);
        }
        PredicateExpression::All { terms } | PredicateExpression::Any { terms } => {
            for term in terms {
                collect_predicate_evidence(term, facts, dependencies, visiting);
            }
        }
        PredicateExpression::Not { term } => {
            collect_predicate_evidence(term, facts, dependencies, visiting)
        }
        PredicateExpression::True
        | PredicateExpression::False
        | PredicateExpression::Compare { .. } => {}
    }
}

pub(super) fn insert_evidence(
    dependencies: &mut BTreeMap<(EvidenceDependencyKind, String), RuleEvidence>,
    dependency_kind: EvidenceDependencyKind,
    record_id: &str,
    evidence: &RuleEvidence,
) {
    dependencies.insert((dependency_kind, record_id.into()), evidence.clone());
}

pub(super) fn weakest_evidence(dependencies: &[EvidenceDependency]) -> Option<TruthStatus> {
    dependencies
        .iter()
        .map(|dependency| dependency.evidence.truth)
        .max()
}

pub(super) fn record_blocked_transition_witness(
    witnesses: &mut BTreeMap<String, BlockedTransitionWitness>,
    candidate: BlockedTransitionWitness,
) {
    let replace = witnesses
        .get(&candidate.transition_id)
        .is_none_or(|current| blocker_rank(&candidate) < blocker_rank(current));
    if replace {
        witnesses.insert(candidate.transition_id.clone(), candidate);
    }
}

pub(super) fn failed_producer_cuts(
    relevance: &BackwardRelevance,
    mechanics: &MechanicsCatalog,
    initial_state_sha256: Digest,
    executed_actions: &BTreeSet<RouteActionRef>,
    transition_witnesses: &BTreeMap<String, BlockedTransitionWitness>,
    writer_witnesses: &BTreeMap<String, BlockedWriterWitness>,
    technique_witnesses: &BTreeMap<String, BlockedTechniqueWitness>,
    resolver_witnesses: &BTreeMap<String, BlockedResolverWitness>,
    reconstruction_witnesses: &BTreeMap<String, BlockedReconstructionWitness>,
) -> Result<Vec<FailedProducerCut>, PlannerContractError> {
    let mut cuts = Vec::new();
    for dependency in &relevance.dependencies {
        if matches!(
            dependency,
            StateDependency::Fact { .. } | StateDependency::AnyState
        ) {
            continue;
        }
        let produces = |operations: &[crate::transition::StateOperation]| {
            operations.iter().any(|operation| {
                operation_outputs(operation)
                    .iter()
                    .any(|output| dependencies_overlap(dependency, output))
            })
        };
        let mut blockers = Vec::new();
        let mut assumptions = Vec::new();
        let mut saw_producer = false;
        let mut complete = true;
        for transition in &mechanics.transitions {
            if !produces(&transition.activation.effects) {
                continue;
            }
            saw_producer = true;
            let action = RouteActionRef::Transition {
                transition_id: transition.id.clone(),
            };
            if executed_actions.contains(&action) {
                complete = false;
                continue;
            }
            if let Some(witness) = transition_witnesses.get(&transition.id) {
                blockers.push(FailedProducerBlocker::Transition {
                    transition_id: transition.id.clone(),
                    source_state_sha256: witness.source_state_sha256,
                    classification: witness.classification,
                });
            } else {
                complete = false;
            }
        }
        for writer in &mechanics.writers {
            if !produces(std::slice::from_ref(&writer.operation)) {
                continue;
            }
            saw_producer = true;
            let action = RouteActionRef::Writer {
                writer_id: writer.id.clone(),
            };
            if executed_actions.contains(&action) {
                complete = false;
                continue;
            }
            if let Some(witness) = writer_witnesses.get(&writer.id) {
                blockers.push(FailedProducerBlocker::Writer {
                    writer_id: writer.id.clone(),
                    source_state_sha256: witness.source_state_sha256,
                    classification: witness.classification,
                });
            } else {
                complete = false;
            }
        }
        for technique in &mechanics.techniques {
            if !produces(&technique.operations) {
                continue;
            }
            saw_producer = true;
            let action = RouteActionRef::Technique {
                technique_id: technique.id.clone(),
            };
            if executed_actions.contains(&action) {
                complete = false;
                continue;
            }
            if let Some(witness) = technique_witnesses.get(&technique.id) {
                blockers.push(FailedProducerBlocker::Technique {
                    technique_id: technique.id.clone(),
                    source_state_sha256: witness.source_state_sha256,
                    classification: witness.classification,
                });
            } else {
                complete = false;
            }
        }
        for resolver in &mechanics.resolvers {
            if !produces(&resolver.operations) {
                continue;
            }
            saw_producer = true;
            let action = RouteActionRef::Resolver {
                resolver_id: resolver.id.clone(),
            };
            if executed_actions.contains(&action) {
                complete = false;
                continue;
            }
            if let Some(witness) = resolver_witnesses.get(&resolver.id) {
                blockers.push(FailedProducerBlocker::Resolver {
                    resolver_id: resolver.id.clone(),
                    source_state_sha256: witness.source_state_sha256,
                    classification: witness.classification,
                    consumer_transition_id: None,
                    consumer_classification: None,
                });
                continue;
            }
            let blocked_consumer = mechanics
                .obstructions
                .iter()
                .find(|obstruction| obstruction.id == resolver.obstruction_id)
                .and_then(|obstruction| {
                    transition_witnesses
                        .get(&obstruction.blocked_action_id)
                        .map(|witness| (obstruction, witness))
                });
            if let Some((obstruction, witness)) = blocked_consumer {
                blockers.push(FailedProducerBlocker::Resolver {
                    resolver_id: resolver.id.clone(),
                    source_state_sha256: witness.source_state_sha256,
                    classification: RuleClassification::Active,
                    consumer_transition_id: Some(obstruction.blocked_action_id.clone()),
                    consumer_classification: Some(witness.classification),
                });
            } else {
                complete = false;
            }
        }
        for rule in &mechanics.reconstruction_rules {
            if !produces(&rule.initialization_operations) {
                continue;
            }
            saw_producer = true;
            if let Some(witness) = reconstruction_witnesses.get(&rule.id) {
                assumptions.push(FailedProducerAssumption::ReconstructionBoundary {
                    reconstruction_rule_id: rule.id.clone(),
                    source_state_sha256: witness.source_state_sha256,
                    classification: witness.classification,
                });
            } else {
                complete = false;
            }
        }
        if !saw_producer {
            assumptions.push(FailedProducerAssumption::NoCatalogProducer {
                source_state_sha256: initial_state_sha256,
            });
        }
        if !complete {
            continue;
        }
        blockers.sort_by_key(FailedProducerBlocker::action);
        assumptions.sort_by(|left, right| left.identity().cmp(right.identity()));
        let cut = FailedProducerCut {
            dependency: dependency.clone(),
            blocked_producers: blockers,
            missing_assumptions: assumptions,
        };
        cut.validate()?;
        cuts.push(cut);
    }
    Ok(cuts)
}

pub(super) fn failed_producer_cut_sets(
    coverage: &GoalTruthCoverage,
    facts: &FactCatalog,
    cuts: &[FailedProducerCut],
    maximum: usize,
) -> Result<(Vec<FailedProducerCutSet>, bool), PlannerContractError> {
    let by_dependency = cuts
        .iter()
        .map(|cut| (cut.dependency.clone(), cut))
        .collect::<BTreeMap<_, _>>();
    let available = by_dependency.keys().cloned().collect::<BTreeSet<_>>();
    let failure = coverage.failure_sets(true, facts, &available, maximum)?;
    let mut sets = Vec::with_capacity(failure.sets.len());
    for dependencies in failure.sets {
        let set = FailedProducerCutSet {
            cuts: dependencies
                .into_iter()
                .map(|dependency| by_dependency[&dependency].clone())
                .collect(),
        };
        set.validate()?;
        sets.push(set);
    }
    Ok((sets, failure.complete))
}

pub(super) fn blocker_rank(witness: &BlockedTransitionWitness) -> (usize, u8, Digest) {
    let unresolved = witness
        .active_obstruction_ids
        .len()
        .saturating_add(witness.unknown_obstruction_ids.len())
        .saturating_add(witness.outstanding_obligation_ids.len())
        .saturating_add(witness.unknown_obligation_ids.len())
        .saturating_add(witness.unknown_requirement_ids.len())
        .saturating_add(usize::from(witness.hard_guard != EvaluatedTruth::True));
    let classification = match witness.classification {
        TransitionClassification::Executable => 0,
        TransitionClassification::Obstructed => 1,
        TransitionClassification::GuardBlocked => 2,
        TransitionClassification::FeasibilityUnknown => 3,
        TransitionClassification::Inapplicable => 4,
    };
    (unresolved, classification, witness.source_state_sha256)
}

pub(super) fn rule_blocker_rank(
    classification: RuleClassification,
    source: Digest,
) -> (u8, Digest) {
    let classification = match classification {
        RuleClassification::Active => 0,
        RuleClassification::Inactive => 1,
        RuleClassification::ActivationUnknown => 2,
        RuleClassification::EvidenceUnknown => 3,
        RuleClassification::Inapplicable => 4,
    };
    (classification, source)
}

pub(super) fn and_truth(left: EvaluatedTruth, right: EvaluatedTruth) -> EvaluatedTruth {
    match (left, right) {
        (EvaluatedTruth::False, _) | (_, EvaluatedTruth::False) => EvaluatedTruth::False,
        (EvaluatedTruth::Unknown, _) | (_, EvaluatedTruth::Unknown) => EvaluatedTruth::Unknown,
        (EvaluatedTruth::True, EvaluatedTruth::True) => EvaluatedTruth::True,
    }
}

pub(super) fn bounded_subsets(ids: &[String], maximum: usize) -> Vec<BTreeSet<String>> {
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
