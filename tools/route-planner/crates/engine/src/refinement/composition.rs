use super::*;

pub(super) fn apply_replacements(
    pack: &RefinementPack,
    facts: &mut FactCatalog,
    mechanics: &mut MechanicsCatalog,
) -> Result<(), PlannerContractError> {
    for rule in &pack.rules {
        let RefinementOperation::ReplaceRecord {
            target_id,
            replacement_kind,
            replacement_rule_id,
        } = &rule.operation
        else {
            continue;
        };
        let removed = remove_record(facts, mechanics, target_id);
        if removed == 0 {
            return Err(PlannerContractError::new(
                "rules.target_id",
                format!("references absent record {target_id}"),
            ));
        }
        if removed > 1 {
            return Err(PlannerContractError::new(
                "rules.target_id",
                format!("record ID {target_id} is ambiguous across catalogs"),
            ));
        }
        if matches!(
            replacement_kind,
            ReplacementKind::Replace | ReplacementKind::Supersede
        ) {
            let replacement_id = replacement_rule_id.as_ref().ok_or_else(|| {
                PlannerContractError::new(
                    "rules.replacement_rule_id",
                    "is required for replace or supersede",
                )
            })?;
            let replacement = pack
                .rules
                .iter()
                .find(|candidate| candidate.id == *replacement_id)
                .ok_or_else(|| {
                    PlannerContractError::new(
                        "rules.replacement_rule_id",
                        "must reference a rule in the same pack",
                    )
                })?;
            if matches!(
                replacement.operation,
                RefinementOperation::ReplaceRecord { .. }
            ) {
                return Err(PlannerContractError::new(
                    "rules.replacement_rule_id",
                    "cannot reference another replacement operation",
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn apply_addition(
    pack: &RefinementPack,
    rule: &RefinementRule,
    facts: &mut FactCatalog,
    mechanics: &mut MechanicsCatalog,
) -> Result<(), PlannerContractError> {
    match &rule.operation {
        RefinementOperation::AddTransition { transition } => {
            mechanics.transitions.push(transition.clone())
        }
        RefinementOperation::AddObligation { obligation } => {
            mechanics.obligations.push(obligation.clone())
        }
        RefinementOperation::AddObstruction { obstruction } => {
            mechanics.obstructions.push(obstruction.clone())
        }
        RefinementOperation::BindObstruction { .. } => {}
        RefinementOperation::AddTechnique { technique } => {
            mechanics.techniques.push(technique.clone())
        }
        RefinementOperation::AddResolver { resolver } => mechanics.resolvers.push(resolver.clone()),
        RefinementOperation::AddWriter { writer } => mechanics.writers.push(writer.clone()),
        RefinementOperation::AddGate { gate } => mechanics.gates.push(gate.clone()),
        RefinementOperation::AddReader { reader } => mechanics.readers.push(reader.clone()),
        RefinementOperation::AddReconstructionRule {
            reconstruction_rule,
        } => mechanics
            .reconstruction_rules
            .push(reconstruction_rule.clone()),
        RefinementOperation::AddMicrotrace { microtrace } => {
            mechanics.microtraces.push(microtrace.clone())
        }
        RefinementOperation::AddGoal { goal } => mechanics.goals.push(goal.clone()),
        RefinementOperation::AddAlias { alias } => facts.aliases.push(alias.clone()),
        RefinementOperation::AddDerivedFact { fact } => facts.derived_facts.push(fact.clone()),
        RefinementOperation::ComponentTransform {
            prerequisite,
            operations,
        } => mechanics.techniques.push(Technique {
            id: rule.id.clone(),
            label: rule.label.clone(),
            scope: pack.manifest.scope.clone(),
            prerequisites: prerequisite.clone(),
            operations: operations.clone(),
            discharged_obligation_ids: Vec::new(),
            introduced_obligation_ids: Vec::new(),
            cost: RouteCost {
                axes: BTreeMap::new(),
            },
            evidence: rule.evidence.clone(),
        }),
        RefinementOperation::SuppressWriter { writer_id, when } => {
            mechanics.gates.push(GateRule {
                id: rule.id.clone(),
                scope: pack.manifest.scope.clone(),
                active_when: when.clone(),
                blocked_writer_ids: vec![writer_id.clone()],
                lifetime: SemanticLifetime::Unknown,
                evidence: rule.evidence.clone(),
            });
        }
        RefinementOperation::AssumeObstructionAbsent {
            obstruction_id,
            when,
        } => mechanics.resolvers.push(ObstructionResolver {
            id: rule.id.clone(),
            label: rule.label.clone(),
            scope: pack.manifest.scope.clone(),
            obstruction_id: obstruction_id.clone(),
            resolution_kind: ResolutionKind::AssumeAbsent,
            applicable_when: when.clone(),
            operations: Vec::new(),
            evidence: rule.evidence.clone(),
        }),
        RefinementOperation::ReplaceRecord { .. } => {}
    }
    Ok(())
}

pub(super) fn validate_authored_obstruction(
    obstruction: &AuthoredObstruction,
) -> Result<(), PlannerContractError> {
    validate_stable_id("rules.obstruction.id", &obstruction.id)?;
    validate_label("rules.obstruction.label", &obstruction.label)?;
    obstruction.scope.validate("rules.obstruction.scope")?;
    obstruction.active_when.validate()?;
    validate_ids(
        "rules.obstruction.obligation_ids",
        &obstruction.obligation_ids,
        false,
    )?;
    obstruction
        .evidence
        .validate("rules.obstruction.evidence")?;
    validate_obstruction_action_selector(&obstruction.action_selector)
}

pub(super) fn validate_obstruction_action_selector(
    selector: &ObstructionActionSelector,
) -> Result<(), PlannerContractError> {
    match selector {
        ObstructionActionSelector::ActionId { action_id } => {
            validate_stable_id("rules.obstruction.action_selector.action_id", action_id)
        }
        ObstructionActionSelector::Transition {
            transition_kind,
            approach_id,
            source,
            destination,
        } => {
            if transition_kind.is_none()
                && approach_id.is_none()
                && source.is_none()
                && destination.is_none()
            {
                return Err(PlannerContractError::new(
                    "rules.obstruction.action_selector",
                    "must contain at least one structural transition criterion",
                ));
            }
            if let Some(approach_id) = approach_id {
                validate_stable_id("rules.obstruction.action_selector.approach_id", approach_id)?;
            }
            if let Some(source) = source {
                validate_location_selector("rules.obstruction.action_selector.source", source)?;
            }
            if let Some(destination) = destination {
                validate_location_selector(
                    "rules.obstruction.action_selector.destination",
                    destination,
                )?;
            }
            Ok(())
        }
    }
}

pub(super) fn validate_location_selector(
    field: &str,
    selector: &SceneLocationSelector,
) -> Result<(), PlannerContractError> {
    if selector.stage.is_none()
        && selector.room.is_none()
        && selector.layer.is_none()
        && selector.spawn.is_none()
    {
        return Err(PlannerContractError::new(
            field,
            "must constrain at least one location field",
        ));
    }
    if let Some(stage) = &selector.stage {
        validate_label(field, stage)?;
    }
    Ok(())
}

pub(super) fn compile_obstruction_bindings(
    stack: &RefinementStack,
    packs: &BTreeMap<&str, &RefinementPack>,
    mechanics: &mut MechanicsCatalog,
) -> Result<Vec<CompiledObstructionBinding>, PlannerContractError> {
    let mut compiled_by_template = BTreeMap::<String, Vec<(String, String)>>::new();
    let mut binding_records = Vec::new();
    for entry in &stack.entries {
        let pack = packs[entry.pack_id.as_str()];
        for rule in &pack.rules {
            let RefinementOperation::BindObstruction { obstruction } = &rule.operation else {
                continue;
            };
            if compiled_by_template.contains_key(&obstruction.id) {
                return Err(PlannerContractError::new(
                    "rules.obstruction.id",
                    format!("duplicate authored obstruction template {}", obstruction.id),
                ));
            }
            let matches = mechanics
                .transitions
                .iter()
                .filter(|transition| transition_matches(&obstruction.action_selector, transition))
                .collect::<Vec<_>>();
            if matches.is_empty() {
                return Err(PlannerContractError::new(
                    "rules.obstruction.action_selector",
                    format!("matched no candidate actions for {}", obstruction.id),
                ));
            }
            if obstruction.match_cardinality == MatchCardinality::ExactlyOne && matches.len() != 1 {
                return Err(PlannerContractError::new(
                    "rules.obstruction.action_selector",
                    format!(
                        "expected exactly one candidate action for {}, matched {}",
                        obstruction.id,
                        matches.len()
                    ),
                ));
            }

            let plural = matches.len() > 1;
            let mut compiled = Vec::with_capacity(matches.len());
            for transition in matches {
                let id = if plural {
                    generated_binding_id("obstruction", &obstruction.id, &transition.id)
                } else {
                    obstruction.id.clone()
                };
                mechanics.obstructions.push(Obstruction {
                    id: id.clone(),
                    label: obstruction.label.clone(),
                    scope: obstruction.scope.clone(),
                    blocked_action_id: transition.id.clone(),
                    approach_id: transition.approach_id.clone(),
                    active_when: obstruction.active_when.clone(),
                    obligation_ids: obstruction.obligation_ids.clone(),
                    evidence: obstruction.evidence.clone(),
                });
                binding_records.push(CompiledObstructionBinding {
                    authored_obstruction_id: obstruction.id.clone(),
                    compiled_obstruction_id: id.clone(),
                    action_id: transition.id.clone(),
                    action_selector: obstruction.action_selector.clone(),
                    match_cardinality: obstruction.match_cardinality,
                    source_pack_id: pack.manifest.id.clone(),
                    source_rule_id: rule.id.clone(),
                });
                compiled.push((id, transition.id.clone()));
            }
            compiled_by_template.insert(obstruction.id.clone(), compiled);
        }
    }

    if compiled_by_template.is_empty() {
        return Ok(Vec::new());
    }
    let resolvers = std::mem::take(&mut mechanics.resolvers);
    for resolver in resolvers {
        let Some(bindings) = compiled_by_template.get(&resolver.obstruction_id) else {
            mechanics.resolvers.push(resolver);
            continue;
        };
        for (index, (obstruction_id, action_id)) in bindings.iter().enumerate() {
            let mut bound = resolver.clone();
            if bindings.len() > 1 {
                bound.id = generated_binding_id("resolver", &resolver.id, action_id);
            } else if index != 0 {
                unreachable!("a singular binding contains only one resolver target");
            }
            bound.obstruction_id = obstruction_id.clone();
            mechanics.resolvers.push(bound);
        }
    }
    binding_records.sort();
    Ok(binding_records)
}

pub(super) fn validate_compiled_obstruction_bindings(
    catalog: &ComposedPlannerCatalog,
) -> Result<(), PlannerContractError> {
    let transition_by_id = catalog
        .mechanics
        .transitions
        .iter()
        .map(|transition| (transition.id.as_str(), transition))
        .collect::<BTreeMap<_, _>>();
    let obstruction_by_id = catalog
        .mechanics
        .obstructions
        .iter()
        .map(|obstruction| (obstruction.id.as_str(), obstruction))
        .collect::<BTreeMap<_, _>>();
    let pack_ids = catalog
        .refinement_stack
        .entries
        .iter()
        .map(|entry| entry.pack_id.as_str())
        .collect::<BTreeSet<_>>();
    let mut previous = None;
    let mut pairs = BTreeSet::new();
    let mut groups = BTreeMap::<&str, Vec<&CompiledObstructionBinding>>::new();
    for binding in &catalog.obstruction_bindings {
        validate_obstruction_action_selector(&binding.action_selector)?;
        for (field, id) in [
            (
                "obstruction_bindings.authored_obstruction_id",
                &binding.authored_obstruction_id,
            ),
            (
                "obstruction_bindings.compiled_obstruction_id",
                &binding.compiled_obstruction_id,
            ),
            ("obstruction_bindings.action_id", &binding.action_id),
            (
                "obstruction_bindings.source_pack_id",
                &binding.source_pack_id,
            ),
            (
                "obstruction_bindings.source_rule_id",
                &binding.source_rule_id,
            ),
        ] {
            validate_stable_id(field, id)?;
        }
        if previous.is_some_and(|prior: &CompiledObstructionBinding| prior >= binding) {
            return Err(PlannerContractError::new(
                "obstruction_bindings",
                "must be unique and sorted",
            ));
        }
        if !pairs.insert((
            binding.authored_obstruction_id.as_str(),
            binding.action_id.as_str(),
        )) {
            return Err(PlannerContractError::new(
                "obstruction_bindings",
                "contains a duplicate authored-obstruction/action pair",
            ));
        }
        let transition = transition_by_id
            .get(binding.action_id.as_str())
            .ok_or_else(|| {
                PlannerContractError::new(
                    "obstruction_bindings.action_id",
                    "references an unknown transition",
                )
            })?;
        let obstruction = obstruction_by_id
            .get(binding.compiled_obstruction_id.as_str())
            .ok_or_else(|| {
                PlannerContractError::new(
                    "obstruction_bindings.compiled_obstruction_id",
                    "references an unknown obstruction",
                )
            })?;
        if obstruction.blocked_action_id != binding.action_id
            || obstruction.approach_id != transition.approach_id
            || !transition_matches(&binding.action_selector, transition)
        {
            return Err(PlannerContractError::new(
                "obstruction_bindings",
                "does not agree with its compiled obstruction and transition",
            ));
        }
        if !pack_ids.contains(binding.source_pack_id.as_str()) {
            return Err(PlannerContractError::new(
                "obstruction_bindings.source_pack_id",
                "references a pack absent from the refinement stack",
            ));
        }
        groups
            .entry(binding.authored_obstruction_id.as_str())
            .or_default()
            .push(binding);
        previous = Some(binding);
    }
    for bindings in groups.values() {
        let first = bindings[0];
        if bindings.iter().any(|binding| {
            binding.action_selector != first.action_selector
                || binding.match_cardinality != first.match_cardinality
                || binding.source_pack_id != first.source_pack_id
                || binding.source_rule_id != first.source_rule_id
        }) {
            return Err(PlannerContractError::new(
                "obstruction_bindings",
                "one authored obstruction has inconsistent selector provenance",
            ));
        }
        if first.match_cardinality == MatchCardinality::ExactlyOne && bindings.len() != 1 {
            return Err(PlannerContractError::new(
                "obstruction_bindings",
                "an exactly-one selector must have exactly one compiled binding",
            ));
        }
    }
    Ok(())
}

pub(super) fn transition_matches(
    selector: &ObstructionActionSelector,
    transition: &CandidateTransition,
) -> bool {
    match selector {
        ObstructionActionSelector::ActionId { action_id } => transition.id == *action_id,
        ObstructionActionSelector::Transition {
            transition_kind,
            approach_id,
            source,
            destination,
        } => {
            transition_kind.is_none_or(|kind| transition.transition_kind == kind)
                && approach_id
                    .as_ref()
                    .is_none_or(|approach| transition.approach_id == *approach)
                && source.as_ref().is_none_or(|source| {
                    source_matches_guard(source, &transition.activation.hard_guards)
                })
                && destination.as_ref().is_none_or(|destination| {
                    transition
                        .activation
                        .effects
                        .iter()
                        .rev()
                        .find_map(|operation| match operation {
                            StateOperation::SetLocation { location } => Some(location),
                            _ => None,
                        })
                        .is_some_and(|location| location_matches(destination, location))
                })
        }
    }
}

pub(super) fn source_matches_guard(
    selector: &SceneLocationSelector,
    guard: &PredicateExpression,
) -> bool {
    selector.stage.as_ref().is_none_or(|stage| {
        guard_contains_location_equality(
            guard,
            &ValueReference::LocationStage,
            &StateValue::Text(stage.clone()),
        )
    }) && selector.room.is_none_or(|room| {
        guard_contains_location_equality(
            guard,
            &ValueReference::LocationRoom,
            &StateValue::Signed(room.into()),
        )
    }) && selector.layer.is_none_or(|layer| {
        guard_contains_location_equality(
            guard,
            &ValueReference::LocationLayer,
            &StateValue::Signed(layer.into()),
        )
    }) && selector.spawn.is_none_or(|spawn| {
        guard_contains_location_equality(
            guard,
            &ValueReference::LocationSpawn,
            &StateValue::Signed(spawn.into()),
        )
    })
}

pub(super) fn guard_contains_location_equality(
    expression: &PredicateExpression,
    reference: &ValueReference,
    value: &StateValue,
) -> bool {
    match expression {
        PredicateExpression::Compare {
            left,
            operator: ComparisonOperator::Equal,
            right,
        } => {
            (left == reference
                && right
                    == &ValueReference::Literal {
                        value: value.clone(),
                    })
                || (right == reference
                    && left
                        == &ValueReference::Literal {
                            value: value.clone(),
                        })
        }
        PredicateExpression::All { terms } => terms
            .iter()
            .any(|term| guard_contains_location_equality(term, reference, value)),
        PredicateExpression::True
        | PredicateExpression::False
        | PredicateExpression::Fact { .. }
        | PredicateExpression::Any { .. }
        | PredicateExpression::Not { .. }
        | PredicateExpression::Compare { .. } => false,
    }
}

pub(super) fn location_matches(selector: &SceneLocationSelector, location: &SceneLocation) -> bool {
    selector
        .stage
        .as_ref()
        .is_none_or(|stage| location.stage == *stage)
        && selector.room.is_none_or(|room| location.room == room)
        && selector.layer.is_none_or(|layer| location.layer == layer)
        && selector.spawn.is_none_or(|spawn| location.spawn == spawn)
}

pub(super) fn generated_binding_id(kind: &str, template_id: &str, action_id: &str) -> String {
    let digest =
        Digest(Sha256::digest(format!("{kind}\0{template_id}\0{action_id}").as_bytes()).into());
    format!("binding.{kind}.{digest}")
}

pub(super) fn remove_record(
    facts: &mut FactCatalog,
    mechanics: &mut MechanicsCatalog,
    id: &str,
) -> usize {
    let mut removed = 0;
    removed += remove_where(&mut facts.aliases, id, |record| &record.id);
    removed += remove_where(&mut facts.derived_facts, id, |record| &record.id);
    removed += remove_where(&mut mechanics.transitions, id, |record| &record.id);
    removed += remove_where(&mut mechanics.obligations, id, |record| &record.id);
    removed += remove_where(&mut mechanics.writers, id, |record| &record.id);
    removed += remove_where(&mut mechanics.gates, id, |record| &record.id);
    removed += remove_where(&mut mechanics.readers, id, |record| &record.id);
    removed += remove_where(&mut mechanics.reconstruction_rules, id, |record| &record.id);
    removed += remove_where(&mut mechanics.obstructions, id, |record| &record.id);
    removed += remove_where(&mut mechanics.resolvers, id, |record| &record.id);
    removed += remove_where(&mut mechanics.techniques, id, |record| &record.id);
    removed += remove_where(&mut mechanics.microtraces, id, |record| &record.id);
    removed += remove_where(&mut mechanics.goals, id, |record| &record.id);
    removed
}

pub(super) fn remove_where<T, F>(records: &mut Vec<T>, id: &str, get_id: F) -> usize
where
    F: Fn(&T) -> &String,
{
    let before = records.len();
    records.retain(|record| get_id(record) != id);
    before - records.len()
}

pub(super) fn sort_catalogs(facts: &mut FactCatalog, mechanics: &mut MechanicsCatalog) {
    facts.aliases.sort_by(|left, right| left.id.cmp(&right.id));
    facts
        .derived_facts
        .sort_by(|left, right| left.id.cmp(&right.id));
    mechanics
        .transitions
        .sort_by(|left, right| left.id.cmp(&right.id));
    mechanics
        .obligations
        .sort_by(|left, right| left.id.cmp(&right.id));
    mechanics
        .writers
        .sort_by(|left, right| left.id.cmp(&right.id));
    mechanics
        .gates
        .sort_by(|left, right| left.id.cmp(&right.id));
    mechanics
        .readers
        .sort_by(|left, right| left.id.cmp(&right.id));
    mechanics
        .reconstruction_rules
        .sort_by(|left, right| left.id.cmp(&right.id));
    mechanics
        .obstructions
        .sort_by(|left, right| left.id.cmp(&right.id));
    mechanics
        .resolvers
        .sort_by(|left, right| left.id.cmp(&right.id));
    mechanics
        .techniques
        .sort_by(|left, right| left.id.cmp(&right.id));
    mechanics
        .microtraces
        .sort_by(|left, right| left.id.cmp(&right.id));
    mechanics
        .goals
        .sort_by(|left, right| left.id.cmp(&right.id));
}

pub(super) fn validate_operations(
    operations: &[StateOperation],
) -> Result<(), PlannerContractError> {
    if operations.len() > 4_096 {
        return Err(PlannerContractError::new(
            "rules.operations",
            "must contain at most 4096 operations",
        ));
    }
    for operation in operations {
        operation.validate()?;
    }
    Ok(())
}

pub(super) fn validate_dependencies(
    dependencies: &[PackDependency],
) -> Result<(), PlannerContractError> {
    if dependencies.len() > 256 {
        return Err(PlannerContractError::new(
            "manifest.dependencies",
            "must contain at most 256 records",
        ));
    }
    let mut previous = None;
    for dependency in dependencies {
        validate_stable_id("manifest.dependencies.pack_id", &dependency.pack_id)?;
        if dependency.pack_sha256 == Digest::ZERO {
            return Err(PlannerContractError::new(
                "manifest.dependencies.pack_sha256",
                "must be nonzero",
            ));
        }
        if previous.is_some_and(|prior: &str| prior >= dependency.pack_id.as_str()) {
            return Err(PlannerContractError::new(
                "manifest.dependencies",
                "must be unique and sorted by pack ID",
            ));
        }
        previous = Some(dependency.pack_id.as_str());
    }
    Ok(())
}

pub(super) fn validate_ids(
    field: &str,
    ids: &[String],
    allow_empty: bool,
) -> Result<(), PlannerContractError> {
    if (!allow_empty && ids.is_empty()) || ids.len() > 256 {
        return Err(PlannerContractError::new(
            field,
            "contains an invalid number of IDs",
        ));
    }
    let mut previous = None;
    for id in ids {
        validate_stable_id(field, id)?;
        if previous.is_some_and(|prior: &str| prior >= id.as_str()) {
            return Err(PlannerContractError::new(
                field,
                "must be unique and sorted",
            ));
        }
        previous = Some(id.as_str());
    }
    Ok(())
}

pub(super) fn validate_version(version: &str) -> Result<(), PlannerContractError> {
    let parts = version.split('.').collect::<Vec<_>>();
    if parts.len() != 3
        || parts
            .iter()
            .any(|part| part.is_empty() || part.parse::<u32>().is_err())
    {
        return Err(PlannerContractError::new(
            "manifest.version",
            "must be a numeric major.minor.patch version",
        ));
    }
    Ok(())
}

pub(super) fn reject_dependency_cycles(
    packs: &BTreeMap<&str, &RefinementPack>,
) -> Result<(), PlannerContractError> {
    fn visit<'a>(
        id: &'a str,
        packs: &BTreeMap<&'a str, &'a RefinementPack>,
        visiting: &mut BTreeSet<&'a str>,
        complete: &mut BTreeSet<&'a str>,
    ) -> Result<(), PlannerContractError> {
        if complete.contains(id) {
            return Ok(());
        }
        if !visiting.insert(id) {
            return Err(PlannerContractError::new(
                "manifest.dependencies",
                format!("dependency cycle at {id}"),
            ));
        }
        for dependency in &packs[id].manifest.dependencies {
            if let Some((canonical, _)) = packs.get_key_value(dependency.pack_id.as_str()) {
                visit(canonical, packs, visiting, complete)?;
            }
        }
        visiting.remove(id);
        complete.insert(id);
        Ok(())
    }

    let mut complete = BTreeSet::new();
    for id in packs.keys().copied() {
        visit(id, packs, &mut BTreeSet::new(), &mut complete)?;
    }
    Ok(())
}
