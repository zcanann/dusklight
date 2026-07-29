use super::*;

impl MechanicsCatalog {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != MECHANICS_CATALOG_SCHEMA {
            return Err(PlannerContractError::new("schema", "is unsupported"));
        }
        let total = self.transitions.len()
            + self.obligations.len()
            + self.writers.len()
            + self.gates.len()
            + self.readers.len()
            + self.reconstruction_rules.len()
            + self.obstructions.len()
            + self.resolvers.len()
            + self.techniques.len()
            + self.microtraces.len()
            + self.goals.len();
        if total > MAX_MECHANICS_RECORDS {
            return Err(PlannerContractError::new(
                "catalog",
                "contains too many mechanics records",
            ));
        }

        let obligation_ids = validate_sorted_records(
            "obligations",
            &self.obligations,
            |value| value.id.as_str(),
            validate_obligation,
        )?;
        let transition_ids = validate_sorted_records(
            "transitions",
            &self.transitions,
            |value| value.id.as_str(),
            validate_transition,
        )?;
        for transition in &self.transitions {
            require_known_ids(
                "transitions.activation.physical_obligation_ids",
                &transition.activation.physical_obligation_ids,
                &obligation_ids,
            )?;
            for obligation_id in &transition.activation.physical_obligation_ids {
                let obligation = self
                    .obligations
                    .iter()
                    .find(|obligation| obligation.id == *obligation_id)
                    .expect("known obligation IDs were checked above");
                validate_transition_obligation_binding(transition, obligation)?;
            }
        }

        let writer_ids = validate_sorted_records(
            "writers",
            &self.writers,
            |value| value.id.as_str(),
            validate_writer,
        )?;
        validate_sorted_records(
            "gates",
            &self.gates,
            |value| value.id.as_str(),
            |gate| validate_gate(gate, &writer_ids),
        )?;
        validate_sorted_records(
            "readers",
            &self.readers,
            |value| value.id.as_str(),
            |reader| validate_reader(reader, &transition_ids),
        )?;
        validate_sorted_records(
            "reconstruction_rules",
            &self.reconstruction_rules,
            |value| value.id.as_str(),
            validate_reconstruction_rule,
        )?;
        let obstruction_ids = validate_sorted_records(
            "obstructions",
            &self.obstructions,
            |value| value.id.as_str(),
            |obstruction| validate_obstruction(obstruction, &obligation_ids),
        )?;
        validate_sorted_records(
            "resolvers",
            &self.resolvers,
            |value| value.id.as_str(),
            |resolver| validate_resolver(resolver, &obstruction_ids),
        )?;
        validate_sorted_records(
            "techniques",
            &self.techniques,
            |value| value.id.as_str(),
            |technique| validate_technique(technique, &obligation_ids),
        )?;
        validate_sorted_records(
            "microtraces",
            &self.microtraces,
            |value| value.id.as_str(),
            validate_microtrace,
        )?;
        validate_sorted_records(
            "goals",
            &self.goals,
            |value| value.id.as_str(),
            validate_goal,
        )?;
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PlannerContractError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, PlannerContractError> {
        let catalog: Self = serde_json::from_slice(bytes)?;
        catalog.validate()?;
        if catalog.canonical_bytes()? != bytes {
            return Err(PlannerContractError::new(
                "mechanics_catalog",
                "is not canonical JSON",
            ));
        }
        Ok(catalog)
    }

    pub fn digest(&self) -> Result<Digest, PlannerContractError> {
        Ok(Digest(Sha256::digest(self.canonical_bytes()?).into()))
    }
}

pub(super) fn validate_transition(
    transition: &CandidateTransition,
) -> Result<(), PlannerContractError> {
    validate_stable_id("transitions.id", &transition.id)?;
    validate_label("transitions.label", &transition.label)?;
    transition.scope.validate("transitions.scope")?;
    validate_stable_id("transitions.approach_id", &transition.approach_id)?;
    transition.activation.hard_guards.validate()?;
    validate_id_list(
        "transitions.activation.physical_obligation_ids",
        &transition.activation.physical_obligation_ids,
        true,
    )?;
    validate_operations(&transition.activation.effects)?;
    for unknown in &transition.activation.unknown_requirements {
        validate_stable_id("transitions.unknown.id", &unknown.id)?;
        validate_label("transitions.unknown.description", &unknown.description)?;
        unknown.evidence.validate("transitions.unknown.evidence")?;
    }
    transition.evidence.validate("transitions.evidence")?;
    let changes_location = transition.activation.effects.iter().any(|operation| {
        matches!(
            operation,
            StateOperation::SetLocation { .. } | StateOperation::SetLocationFromFields { .. }
        )
    });
    let extracted_destination = transition.transition_kind == TransitionKind::EncodedMapExit
        && changes_location
        && transition
            .evidence
            .records
            .iter()
            .any(|record| record.kind == EvidenceKind::Extracted);
    if extracted_destination
        && transition.activation.physical_obligation_ids.is_empty()
        && transition.activation.unknown_requirements.is_empty()
    {
        return Err(PlannerContractError::new(
            "transitions.activation.extracted_destination",
            "must retain a physical obligation or an explicit unknown requirement",
        ));
    }
    Ok(())
}

pub(super) fn validate_obligation(
    obligation: &FeasibilityObligation,
) -> Result<(), PlannerContractError> {
    validate_stable_id("obligations.id", &obligation.id)?;
    validate_label("obligations.label", &obligation.label)?;
    obligation.scope.validate("obligations.scope")?;
    if obligation.stage == ObligationStage::Interrupt
        && !matches!(
            obligation.detail,
            ObligationDetail::Temporal { .. } | ObligationDetail::Unresolved { .. }
        )
    {
        return Err(PlannerContractError::new(
            "obligations.stage",
            "interrupt obligations must be temporal or explicitly unresolved",
        ));
    }
    match &obligation.detail {
        ObligationDetail::Predicate { predicate } => predicate.validate()?,
        ObligationDetail::Interaction {
            actor_instance_id,
            interaction_mode,
            required_volumes,
            excluded_volumes,
            pose_predicate,
            temporal_requirement,
        } => {
            validate_stable_id("obligation.actor_instance_id", actor_instance_id)?;
            validate_stable_id("obligation.interaction_mode", interaction_mode)?;
            validate_volumes(required_volumes)?;
            validate_volumes(excluded_volumes)?;
            pose_predicate.validate()?;
            if let Some(requirement) = temporal_requirement {
                requirement.validate()?;
            }
        }
        ObligationDetail::CompoundInteraction {
            actor_instance_id,
            interaction_mode,
            branches,
            temporal_requirement,
        } => {
            validate_stable_id("obligation.actor_instance_id", actor_instance_id)?;
            validate_stable_id("obligation.interaction_mode", interaction_mode)?;
            if branches.is_empty() || branches.len() > 16 {
                return Err(PlannerContractError::new(
                    "obligation.branches",
                    "must contain between 1 and 16 interaction branches",
                ));
            }
            for branch in branches {
                branch.when.validate()?;
                branch.pose_predicate.validate()?;
                if branch.volume_tests.is_empty() || branch.volume_tests.len() > 16 {
                    return Err(PlannerContractError::new(
                        "obligation.volume_tests",
                        "must contain between 1 and 16 tests",
                    ));
                }
                for test in &branch.volume_tests {
                    validate_stable_id("obligation.volume.object_id", &test.volume.object_id)?;
                    validate_stable_id("obligation.volume.volume_id", &test.volume.volume_id)?;
                }
            }
            if let Some(requirement) = temporal_requirement {
                requirement.validate()?;
            }
        }
        ObligationDetail::Geometry {
            approach_id,
            source_region_id,
            destination_region_id,
        } => {
            validate_stable_id("obligation.approach_id", approach_id)?;
            validate_stable_id("obligation.source_region_id", source_region_id)?;
            validate_stable_id("obligation.destination_region_id", destination_region_id)?;
        }
        ObligationDetail::PlaneSide { plane_id, .. } => {
            validate_stable_id("obligation.plane_id", plane_id)?;
        }
        ObligationDetail::Facing {
            yaw, maximum_delta, ..
        } => {
            validate_value_reference(yaw)?;
            if *maximum_delta > 0x8000 {
                return Err(PlannerContractError::new(
                    "obligation.maximum_delta",
                    "binary-angle distance cannot exceed one half-turn",
                ));
            }
        }
        ObligationDetail::Temporal {
            requirement,
            precondition,
        } => {
            requirement.validate()?;
            precondition.validate()?;
        }
        ObligationDetail::Unresolved { research_question } => {
            validate_label("obligation.research_question", research_question)?;
        }
    }
    obligation.evidence.validate("obligations.evidence")
}

pub(super) fn validate_transition_obligation_binding(
    transition: &CandidateTransition,
    obligation: &FeasibilityObligation,
) -> Result<(), PlannerContractError> {
    if obligation.stage == ObligationStage::Effect && transition.activation.effects.is_empty() {
        return Err(PlannerContractError::new(
            "transitions.activation.physical_obligation_ids",
            "an effect obligation requires a state-producing transition",
        ));
    }
    if let ObligationDetail::Geometry { approach_id, .. } = &obligation.detail {
        if transition.approach_id != *approach_id {
            return Err(PlannerContractError::new(
                "transitions.activation.physical_obligation_ids",
                "geometry obligations must name the transition's exact approach",
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_writer(writer: &WriterRule) -> Result<(), PlannerContractError> {
    validate_stable_id("writers.id", &writer.id)?;
    writer.scope.validate("writers.scope")?;
    writer.activation.validate()?;
    writer.operation.validate()?;
    writer.evidence.validate("writers.evidence")
}

pub(super) fn validate_gate(
    gate: &GateRule,
    writer_ids: &BTreeSet<&str>,
) -> Result<(), PlannerContractError> {
    validate_stable_id("gates.id", &gate.id)?;
    gate.scope.validate("gates.scope")?;
    gate.active_when.validate()?;
    validate_id_list("gates.blocked_writer_ids", &gate.blocked_writer_ids, false)?;
    require_known_ids(
        "gates.blocked_writer_ids",
        &gate.blocked_writer_ids,
        writer_ids,
    )?;
    gate.evidence.validate("gates.evidence")
}

pub(super) fn validate_reader(
    reader: &ReaderRule,
    transition_ids: &BTreeSet<&str>,
) -> Result<(), PlannerContractError> {
    validate_stable_id("readers.id", &reader.id)?;
    reader.scope.validate("readers.scope")?;
    validate_value_reference(&reader.source)?;
    validate_stable_id(
        "readers.consuming_transition_id",
        &reader.consuming_transition_id,
    )?;
    if !transition_ids.contains(reader.consuming_transition_id.as_str()) {
        return Err(PlannerContractError::new(
            "readers.consuming_transition_id",
            "references an unknown transition",
        ));
    }
    if let Some(fact_id) = &reader.interpretation_fact_id {
        validate_stable_id("readers.interpretation_fact_id", fact_id)?;
    }
    reader.evidence.validate("readers.evidence")
}

pub(super) fn validate_reconstruction_rule(
    rule: &ActorReconstructionRule,
) -> Result<(), PlannerContractError> {
    validate_stable_id("reconstruction_rules.id", &rule.id)?;
    validate_label("reconstruction_rules.label", &rule.label)?;
    rule.scope.validate("reconstruction_rules.scope")?;
    validate_stable_id("reconstruction_rules.actor_type", &rule.actor_type)?;
    rule.instantiate_when.validate()?;
    validate_operations(&rule.initialization_operations)?;
    rule.evidence.validate("reconstruction_rules.evidence")
}

pub(super) fn validate_obstruction(
    obstruction: &Obstruction,
    obligation_ids: &BTreeSet<&str>,
) -> Result<(), PlannerContractError> {
    validate_stable_id("obstructions.id", &obstruction.id)?;
    validate_label("obstructions.label", &obstruction.label)?;
    obstruction.scope.validate("obstructions.scope")?;
    validate_stable_id(
        "obstructions.blocked_action_id",
        &obstruction.blocked_action_id,
    )?;
    validate_stable_id("obstructions.approach_id", &obstruction.approach_id)?;
    obstruction.active_when.validate()?;
    validate_id_list(
        "obstructions.obligation_ids",
        &obstruction.obligation_ids,
        false,
    )?;
    require_known_ids(
        "obstructions.obligation_ids",
        &obstruction.obligation_ids,
        obligation_ids,
    )?;
    obstruction.evidence.validate("obstructions.evidence")
}

pub(super) fn validate_resolver(
    resolver: &ObstructionResolver,
    obstruction_ids: &BTreeSet<&str>,
) -> Result<(), PlannerContractError> {
    validate_stable_id("resolvers.id", &resolver.id)?;
    validate_label("resolvers.label", &resolver.label)?;
    resolver.scope.validate("resolvers.scope")?;
    validate_stable_id("resolvers.obstruction_id", &resolver.obstruction_id)?;
    if !obstruction_ids.contains(resolver.obstruction_id.as_str()) {
        return Err(PlannerContractError::new(
            "resolvers.obstruction_id",
            "references an unknown obstruction",
        ));
    }
    resolver.applicable_when.validate()?;
    validate_operations(&resolver.operations)?;
    resolver.evidence.validate("resolvers.evidence")
}

pub(super) fn validate_technique(
    technique: &Technique,
    obligation_ids: &BTreeSet<&str>,
) -> Result<(), PlannerContractError> {
    validate_stable_id("techniques.id", &technique.id)?;
    validate_label("techniques.label", &technique.label)?;
    technique.scope.validate("techniques.scope")?;
    technique.prerequisites.validate()?;
    validate_operations(&technique.operations)?;
    validate_id_list(
        "techniques.discharged_obligation_ids",
        &technique.discharged_obligation_ids,
        true,
    )?;
    validate_id_list(
        "techniques.introduced_obligation_ids",
        &technique.introduced_obligation_ids,
        true,
    )?;
    require_known_ids(
        "techniques.discharged_obligation_ids",
        &technique.discharged_obligation_ids,
        obligation_ids,
    )?;
    require_known_ids(
        "techniques.introduced_obligation_ids",
        &technique.introduced_obligation_ids,
        obligation_ids,
    )?;
    if technique.cost.axes.len() > 64 {
        return Err(PlannerContractError::new(
            "techniques.cost",
            "must contain at most 64 axes",
        ));
    }
    for axis in technique.cost.axes.keys() {
        validate_stable_id("techniques.cost.axis", axis)?;
    }
    technique.evidence.validate("techniques.evidence")
}

pub(super) fn validate_microtrace(trace: &WitnessedMicrotrace) -> Result<(), PlannerContractError> {
    validate_stable_id("microtraces.id", &trace.id)?;
    trace.scope.validate("microtraces.scope")?;
    trace.precondition.validate()?;
    validate_operations(&trace.operations)?;
    trace.postcondition.validate()?;
    trace.timing.validate()?;
    trace.evidence.validate("microtraces.evidence")
}

pub(super) fn validate_goal(goal: &Goal) -> Result<(), PlannerContractError> {
    validate_stable_id("goals.id", &goal.id)?;
    validate_label("goals.label", &goal.label)?;
    goal.predicate.validate()
}

pub(super) fn validate_operations(
    operations: &[StateOperation],
) -> Result<(), PlannerContractError> {
    if operations.len() > 4_096 {
        return Err(PlannerContractError::new(
            "operations",
            "must contain at most 4096 operations",
        ));
    }
    for operation in operations {
        operation.validate()?;
    }
    Ok(())
}

pub(super) fn validate_field_target(
    target: &ComponentFieldTarget,
) -> Result<(), PlannerContractError> {
    validate_stable_id("operation.target.component_id", &target.component_id)?;
    validate_stable_id("operation.target.field", &target.field)
}

pub(super) fn validate_state_value(value: &StateValue) -> Result<(), PlannerContractError> {
    match value {
        StateValue::Text(value) => validate_label("operation.value", value),
        StateValue::Bytes(value) if value.len() > 1024 * 1024 => Err(PlannerContractError::new(
            "operation.value",
            "byte values must contain at most 1 MiB",
        )),
        _ => Ok(()),
    }
}

pub(super) fn validate_component_selector(
    selector: &ComponentSelector,
) -> Result<(), PlannerContractError> {
    match selector {
        ComponentSelector::Id { component_id } => {
            validate_stable_id("operation.selector.component_id", component_id)
        }
        ComponentSelector::Kind { component_kind } => validate_component_kind(component_kind),
        ComponentSelector::Binding { binding } => validate_binding(binding),
    }
}

pub(super) fn validate_binding(binding: &ComponentBinding) -> Result<(), PlannerContractError> {
    validate_component_binding(binding)
}

pub(super) fn validate_owner(owner: &SerializationOwner) -> Result<(), PlannerContractError> {
    validate_serialization_owner(owner)
}

pub(super) fn validate_custom_store_owner(
    field: &str,
    owner: &SerializationOwner,
) -> Result<(), PlannerContractError> {
    validate_owner(owner)?;
    if !matches!(owner, SerializationOwner::Custom { .. }) {
        return Err(PlannerContractError::new(
            field,
            "must name a custom process/session backing store",
        ));
    }
    Ok(())
}

pub(super) fn validate_value_reference(
    reference: &ValueReference,
) -> Result<(), PlannerContractError> {
    PredicateExpression::Compare {
        left: reference.clone(),
        operator: crate::logic::ComparisonOperator::Equal,
        right: ValueReference::Literal {
            value: StateValue::Boolean(true),
        },
    }
    .validate()
}

pub(super) fn validate_volumes(volumes: &[VolumeReference]) -> Result<(), PlannerContractError> {
    if volumes.len() > 256 {
        return Err(PlannerContractError::new(
            "volumes",
            "must contain at most 256 records",
        ));
    }
    let mut unique = BTreeSet::new();
    for volume in volumes {
        validate_stable_id("volume.object_id", &volume.object_id)?;
        validate_stable_id("volume.volume_id", &volume.volume_id)?;
        if !unique.insert(volume) {
            return Err(PlannerContractError::new(
                "volumes",
                "contains a duplicate volume",
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_id_list(
    field: &str,
    ids: &[String],
    allow_empty: bool,
) -> Result<(), PlannerContractError> {
    if (!allow_empty && ids.is_empty()) || ids.len() > 4_096 {
        return Err(PlannerContractError::new(
            field,
            if allow_empty {
                "must contain at most 4096 IDs"
            } else {
                "must contain between 1 and 4096 IDs"
            },
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

pub(super) fn validate_stage_list(
    field: &str,
    stages: &[String],
) -> Result<(), PlannerContractError> {
    if stages.len() > 4_096 {
        return Err(PlannerContractError::new(
            field,
            "must contain at most 4096 stage names",
        ));
    }
    let mut previous = None;
    for stage in stages {
        validate_binding(&ComponentBinding::Stage {
            stage: stage.clone(),
        })?;
        if previous.is_some_and(|prior: &str| prior >= stage.as_str()) {
            return Err(PlannerContractError::new(
                field,
                "must be unique and sorted",
            ));
        }
        previous = Some(stage.as_str());
    }
    Ok(())
}

pub(super) fn validate_slot_list(
    field: &str,
    slots: &[PhysicalSlotId],
) -> Result<(), PlannerContractError> {
    if slots.is_empty() || slots.len() > 3 {
        return Err(PlannerContractError::new(
            field,
            "must contain between one and three physical slots",
        ));
    }
    let mut previous = None;
    for slot in slots {
        slot.validate(field)?;
        if previous.is_some_and(|prior: PhysicalSlotId| prior >= *slot) {
            return Err(PlannerContractError::new(
                field,
                "must be unique and sorted",
            ));
        }
        previous = Some(*slot);
    }
    Ok(())
}

pub(super) fn validate_sorted_records<'a, T>(
    field: &str,
    values: &'a [T],
    id: impl Fn(&'a T) -> &'a str,
    validate: impl Fn(&T) -> Result<(), PlannerContractError>,
) -> Result<BTreeSet<&'a str>, PlannerContractError> {
    let mut ids = BTreeSet::new();
    let mut previous = None;
    for value in values {
        validate(value)?;
        let current = id(value);
        if !ids.insert(current) || previous.is_some_and(|prior: &str| prior >= current) {
            return Err(PlannerContractError::new(
                field,
                "must be unique and sorted by ID",
            ));
        }
        previous = Some(current);
    }
    Ok(ids)
}

pub(super) fn require_known_ids(
    field: &str,
    ids: &[String],
    known: &BTreeSet<&str>,
) -> Result<(), PlannerContractError> {
    if let Some(id) = ids.iter().find(|id| !known.contains(id.as_str())) {
        return Err(PlannerContractError::new(
            field,
            format!("references unknown ID {id}"),
        ));
    }
    Ok(())
}
