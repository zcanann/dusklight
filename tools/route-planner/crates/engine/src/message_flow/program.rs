use super::*;

impl MessageFlowProgramSet {
    pub fn build(
        bundle: &ExtractedOrigBundle,
        runtime_configuration: &RuntimeConfiguration,
        profile: &MessageFlowImportProfile,
    ) -> Result<Self, PlannerContractError> {
        let programs = construct_message_flow_programs(bundle, runtime_configuration, profile)?;
        let locale_bundle = profile
            .language_bundles
            .get(&runtime_configuration.language)
            .expect("construction validated the language selection")
            .clone();
        let set = Self {
            schema: MESSAGE_FLOW_PROGRAM_SET_SCHEMA.into(),
            profile_sha256: profile.digest()?,
            bundle_sha256: bundle.digest()?,
            exact_context: runtime_configuration.exact_context()?,
            locale_bundle,
            programs,
        };
        set.validate()?;
        Ok(set)
    }

    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != MESSAGE_FLOW_PROGRAM_SET_SCHEMA {
            return Err(PlannerContractError::new(
                "message_flow_program_set.schema",
                "is unsupported",
            ));
        }
        if self.profile_sha256 == Digest::ZERO || self.bundle_sha256 == Digest::ZERO {
            return Err(PlannerContractError::new(
                "message_flow_program_set",
                "profile and bundle digests must be nonzero",
            ));
        }
        validate_language_token(
            "message_flow_program_set.locale_bundle",
            &self.locale_bundle,
            false,
        )?;
        if self.programs.is_empty() {
            return Err(PlannerContractError::new(
                "message_flow_program_set.programs",
                "must contain at least one selected program",
            ));
        }
        let expected_scope = ContextScope {
            selectors: vec![ContextSelector::Exact {
                context: self.exact_context.clone(),
            }],
        };
        let mut prior = None;
        for program in &self.programs {
            program.validate()?;
            if program.scope != expected_scope {
                return Err(PlannerContractError::new(
                    "message_flow_program_set.programs.scope",
                    "must name the set's exact context",
                ));
            }
            if prior.is_some_and(|group| group >= program.message_group) {
                return Err(PlannerContractError::new(
                    "message_flow_program_set.programs",
                    "must contain one program per group in ascending order",
                ));
            }
            prior = Some(program.message_group);
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PlannerContractError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, PlannerContractError> {
        let set: Self = serde_json::from_slice(bytes)?;
        set.validate()?;
        if set.canonical_bytes()? != bytes {
            return Err(PlannerContractError::new(
                "message_flow_program_set",
                "is not canonical JSON",
            ));
        }
        Ok(set)
    }

    pub fn digest(&self) -> Result<Digest, PlannerContractError> {
        Ok(Digest(Sha256::digest(self.canonical_bytes()?).into()))
    }
}

/// Construct one program for every message resource in the locale bundle
/// selected by the live runtime configuration. This intentionally supplies no
/// actor entry, event-handler, or cleanup contracts: those need their own
/// source-audited records and can be layered onto the generated programs.
pub fn construct_message_flow_programs(
    bundle: &ExtractedOrigBundle,
    runtime_configuration: &RuntimeConfiguration,
    profile: &MessageFlowImportProfile,
) -> Result<Vec<MessageFlowProgram>, PlannerContractError> {
    bundle.validate()?;
    runtime_configuration.validate()?;
    profile.validate()?;
    let content_sha256 = bundle.content.digest()?;
    if profile.content_sha256 != content_sha256
        || runtime_configuration.content_sha256 != content_sha256
    {
        return Err(PlannerContractError::new(
            "message_flow_import_profile.content_sha256",
            "does not match the extracted bundle and runtime configuration",
        ));
    }
    let locale_bundle = profile
        .language_bundles
        .get(&runtime_configuration.language)
        .ok_or_else(|| {
            PlannerContractError::new(
                "message_flow_import_profile.language_bundles",
                "does not select a bundle for the runtime language",
            )
        })?;
    let records = bundle
        .message_flows
        .iter()
        .filter(|record| &record.locale_bundle == locale_bundle)
        .collect::<Vec<_>>();
    construct_selected_message_flow_programs(
        content_sha256,
        runtime_configuration,
        profile,
        locale_bundle,
        &records,
    )
}

pub(super) fn construct_selected_message_flow_programs(
    content_sha256: Digest,
    runtime_configuration: &RuntimeConfiguration,
    profile: &MessageFlowImportProfile,
    locale_bundle: &str,
    records: &[&ExtractedOrigMessageArchive],
) -> Result<Vec<MessageFlowProgram>, PlannerContractError> {
    runtime_configuration.validate()?;
    profile.validate()?;
    if records.is_empty() {
        return Err(PlannerContractError::new(
            "message_flow_import_profile.language_bundles",
            "selects no extracted message resources",
        ));
    }
    let exact_context = runtime_configuration.exact_context()?;
    if exact_context.content_sha256 != content_sha256 {
        return Err(PlannerContractError::new(
            "runtime_configuration.content_sha256",
            "does not match the selected content",
        ));
    }
    let scope = ContextScope {
        selectors: vec![ContextSelector::Exact {
            context: exact_context,
        }],
    };
    let profile_token = short_token(profile.digest()?);
    let mut programs = Vec::with_capacity(records.len());
    let mut groups = BTreeSet::new();
    for record in records {
        if record.locale_bundle != locale_bundle || !groups.insert(record.message_group) {
            return Err(PlannerContractError::new(
                "message_flow_import_profile.selected_resources",
                "must contain exactly one selected resource per message group",
            ));
        }
        let token = short_token(record.resource_sha256);
        let mut evidence = profile.evidence.clone();
        evidence.records.push(EvidenceRecord {
            id: format!("evidence.message-resource.{token}"),
            kind: EvidenceKind::Extracted,
            source_sha256: Some(record.resource_sha256),
            note: format!(
                "Extracted message group {} from selected locale bundle {}.",
                record.message_group, locale_bundle
            ),
        });
        let program = MessageFlowProgram {
            schema: MESSAGE_FLOW_PROGRAM_SCHEMA.into(),
            id: format!(
                "message-program.{profile_token}.{}.group-{}.{token}",
                locale_bundle, record.message_group
            ),
            label: format!("Message group {} ({})", record.message_group, locale_bundle),
            scope: scope.clone(),
            message_group: record.message_group.try_into().map_err(|_| {
                PlannerContractError::new(
                    "message_flow_import_profile.message_group",
                    "exceeds the runtime message-group width",
                )
            })?,
            resource_sha256: record.resource_sha256,
            flow_component_id: profile.flow_component_id.clone(),
            extracted: record.flow.clone(),
            bindings: profile.bindings.clone(),
            event_contracts: Vec::new(),
            cleanup_edges: Vec::new(),
            evidence,
        };
        program.validate()?;
        programs.push(program);
    }
    programs.sort_by(|left, right| {
        left.message_group
            .cmp(&right.message_group)
            .then_with(|| left.resource_sha256.cmp(&right.resource_sha256))
    });
    Ok(programs)
}

impl MessageFlowProgram {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != MESSAGE_FLOW_PROGRAM_SCHEMA {
            return Err(PlannerContractError::new(
                "message_flow_program.schema",
                "is unsupported",
            ));
        }
        validate_stable_id("message_flow_program.id", &self.id)?;
        validate_label("message_flow_program.label", &self.label)?;
        self.scope.validate("message_flow_program.scope")?;
        if self.resource_sha256 == Digest::ZERO {
            return Err(PlannerContractError::new(
                "message_flow_program.resource_sha256",
                "must be nonzero",
            ));
        }
        validate_stable_id(
            "message_flow_program.flow_component_id",
            &self.flow_component_id,
        )?;
        self.evidence.validate("message_flow_program.evidence")?;
        if !self
            .evidence
            .records
            .iter()
            .any(|record| record.source_sha256 == Some(self.resource_sha256))
        {
            return Err(PlannerContractError::new(
                "message_flow_program.evidence",
                "must cite the selected message resource digest",
            ));
        }
        validate_extracted(&self.extracted)?;
        self.bindings.validate_for(&self.extracted)?;
        validate_event_contracts(
            &self.event_contracts,
            &self.extracted,
            &self.flow_component_id,
        )?;
        validate_cleanup_edges(&self.cleanup_edges, &self.bindings)?;
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PlannerContractError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, PlannerContractError> {
        let program: Self = serde_json::from_slice(bytes)?;
        program.validate()?;
        if program.canonical_bytes()? != bytes {
            return Err(PlannerContractError::new(
                "message_flow_program",
                "is not canonical JSON",
            ));
        }
        Ok(program)
    }

    pub fn digest(&self) -> Result<Digest, PlannerContractError> {
        Ok(Digest(Sha256::digest(self.canonical_bytes()?).into()))
    }

    pub fn compile(&self) -> Result<CompiledMessageFlowProgram, PlannerContractError> {
        self.validate()?;
        let program_sha256 = self.digest()?;
        let token = short_token(program_sha256);
        let terminal_node_id = format!("message-node.{token}.end");
        let mut transitions = Vec::new();
        let mut readers = Vec::new();
        let mut unresolved_nodes = Vec::new();
        let contracts = self
            .event_contracts
            .iter()
            .map(|contract| (contract.node_index, contract))
            .collect::<BTreeMap<_, _>>();

        for node in &self.extracted.nodes {
            match node {
                MessageFlowNode::Message {
                    index,
                    next_node_index,
                    ..
                } => transitions.push(self.direct_transition(
                    &token,
                    *index,
                    "advance-message",
                    format!("Advance message node {index}"),
                    Vec::new(),
                    Vec::new(),
                    *next_node_index,
                    &terminal_node_id,
                    &self.evidence,
                )?),
                MessageFlowNode::Event {
                    index,
                    event_index,
                    next_target_index,
                    parameter_0,
                    parameter_1,
                    raw_parameter_u32,
                    ..
                } => {
                    let encoded_successor =
                        self.extracted.branch_targets[usize::from(*next_target_index)];
                    let contract = contracts.get(index).copied();
                    let (mut operations, unknowns, fully_decoded, generic_continuation) = self
                        .compile_generic_event(
                            *index,
                            *event_index,
                            *parameter_0,
                            *parameter_1,
                            *raw_parameter_u32,
                            &token,
                        )?;
                    let evidence = if let Some(contract) = contract {
                        operations = contract.confirmed_operations.clone();
                        contract.evidence.clone()
                    } else {
                        self.evidence.clone()
                    };
                    let mut unknowns = unknowns;
                    if contract.is_none() && !fully_decoded {
                        unknowns.push(unknown_requirement(
                            &token,
                            *index,
                            "event-handler",
                            format!(
                                "Event handler {event_index} at node {index} has no exact imported state/control contract"
                            ),
                            &self.evidence,
                        ));
                    }
                    let continuation = contract
                        .map(|value| value.continuation)
                        .unwrap_or(generic_continuation);
                    transitions.push(self.direct_transition_with_continuation(
                        &token,
                        *index,
                        "execute-event",
                        format!("Execute message event {event_index} at node {index}"),
                        operations,
                        unknowns,
                        encoded_successor,
                        &terminal_node_id,
                        &evidence,
                        continuation == MessageEventContinuation::EncodedSuccessor,
                    )?);
                }
                MessageFlowNode::Branch {
                    index,
                    next_target_index,
                    query_handler_index,
                    parameter,
                    ..
                } => {
                    let target_index = usize::from(*next_target_index);
                    let targets = [
                        self.extracted.branch_targets[target_index],
                        self.extracted.branch_targets[target_index + 1],
                    ];
                    let branch_access = self.branch_access(*index, &token)?;
                    for (outcome, target) in targets.into_iter().enumerate() {
                        let outcome = outcome as u8;
                        let transition_id = format!(
                            "transition.message-flow.{token}.node-{index}.outcome-{outcome}"
                        );
                        let mut guards = vec![flow_node_guard(
                            &self.flow_component_id,
                            &node_id(&token, *index),
                        )];
                        let mut unknowns = Vec::new();
                        if let Some(access) = &branch_access {
                            guards.push(raw_branch_guard(access, outcome));
                            readers.push(ReaderRule {
                                id: format!(
                                    "reader.message-flow.{token}.node-{index}.outcome-{outcome}"
                                ),
                                scope: self.scope.clone(),
                                source: access.reference.clone(),
                                consuming_transition_id: transition_id.clone(),
                                interpretation_fact_id: access.alias_id.clone(),
                                evidence: self.evidence.clone(),
                            });
                        } else if let Some((guard, source)) =
                            self.compile_numeric_branch(*query_handler_index, *parameter, outcome)
                        {
                            guards.push(guard);
                            readers.push(ReaderRule {
                                id: format!(
                                    "reader.message-flow.{token}.node-{index}.outcome-{outcome}"
                                ),
                                scope: self.scope.clone(),
                                source,
                                consuming_transition_id: transition_id.clone(),
                                interpretation_fact_id: None,
                                evidence: self.evidence.clone(),
                            });
                        } else {
                            unknowns.push(unknown_requirement(
                                &token,
                                *index,
                                "branch-predicate",
                                format!(
                                    "Query handler {:?} at node {index} has no decidable imported predicate",
                                    query_handler_index
                                ),
                                &self.evidence,
                            ));
                        }
                        let transition = CandidateTransition {
                            id: transition_id,
                            label: format!("Take message branch {outcome} at node {index}"),
                            scope: self.scope.clone(),
                            transition_kind: TransitionKind::MessageAction,
                            approach_id: format!("approach.message-flow.{token}.node-{index}"),
                            activation: ActivationContract {
                                hard_guards: all_guards(guards),
                                physical_obligation_ids: Vec::new(),
                                effects: vec![StateOperation::BranchFlow {
                                    flow_component_id: self.flow_component_id.clone(),
                                    edge_id: format!(
                                        "message-edge.{token}.node-{index}.outcome-{outcome}"
                                    ),
                                    destination_node_id: target_node_id(
                                        &token,
                                        target,
                                        &terminal_node_id,
                                    ),
                                }],
                                unknown_requirements: unknowns,
                            },
                            evidence: self.evidence.clone(),
                        };
                        transition.validate()?;
                        transitions.push(transition);
                    }
                }
                MessageFlowNode::Unknown {
                    index, node_type, ..
                } => unresolved_nodes.push(UnresolvedMessageFlowNode {
                    node_index: *index,
                    reason: format!(
                        "Unknown message-flow node type {node_type}; no successor was invented"
                    ),
                }),
            }
        }

        for cleanup in &self.cleanup_edges {
            transitions.push(self.compile_cleanup(cleanup)?);
        }
        transitions.sort_by(|left, right| left.id.cmp(&right.id));
        readers.sort_by(|left, right| left.id.cmp(&right.id));
        let aliases = self.compile_aliases(&token)?;
        let mechanics = MechanicsCatalog {
            schema: MECHANICS_CATALOG_SCHEMA.into(),
            transitions,
            obligations: Vec::new(),
            writers: Vec::new(),
            gates: Vec::new(),
            readers,
            reconstruction_rules: Vec::new(),
            obstructions: Vec::new(),
            resolvers: Vec::new(),
            techniques: Vec::new(),
            microtraces: Vec::new(),
            goals: Vec::new(),
        };
        mechanics.validate()?;
        let entry_points = self
            .extracted
            .labels
            .iter()
            .map(|label| CompiledMessageFlowEntry {
                flow_id: label.flow_id,
                node_id: target_node_id(&token, label.node_index, &terminal_node_id),
            })
            .collect();
        let artifact = CompiledMessageFlowProgram {
            schema: COMPILED_MESSAGE_FLOW_PROGRAM_SCHEMA.into(),
            program_sha256,
            flow_component_id: self.flow_component_id.clone(),
            terminal_node_id,
            entry_points,
            unresolved_nodes,
            aliases,
            mechanics,
        };
        artifact.validate()?;
        Ok(artifact)
    }

    #[allow(clippy::too_many_arguments)]
    fn direct_transition(
        &self,
        token: &str,
        index: u16,
        suffix: &str,
        label: String,
        operations: Vec<StateOperation>,
        unknown_requirements: Vec<UnknownRequirement>,
        destination: u16,
        terminal_node_id: &str,
        evidence: &RuleEvidence,
    ) -> Result<CandidateTransition, PlannerContractError> {
        self.direct_transition_with_continuation(
            token,
            index,
            suffix,
            label,
            operations,
            unknown_requirements,
            destination,
            terminal_node_id,
            evidence,
            true,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn direct_transition_with_continuation(
        &self,
        token: &str,
        index: u16,
        suffix: &str,
        label: String,
        mut operations: Vec<StateOperation>,
        unknown_requirements: Vec<UnknownRequirement>,
        destination: u16,
        terminal_node_id: &str,
        evidence: &RuleEvidence,
        append_encoded_successor: bool,
    ) -> Result<CandidateTransition, PlannerContractError> {
        if append_encoded_successor {
            operations.push(StateOperation::AdvanceFlow {
                flow_component_id: self.flow_component_id.clone(),
                node_id: target_node_id(token, destination, terminal_node_id),
            });
        }
        let transition = CandidateTransition {
            id: format!("transition.message-flow.{token}.node-{index}.{suffix}"),
            label,
            scope: self.scope.clone(),
            transition_kind: TransitionKind::MessageAction,
            approach_id: format!("approach.message-flow.{token}.node-{index}"),
            activation: ActivationContract {
                hard_guards: flow_node_guard(&self.flow_component_id, &node_id(token, index)),
                physical_obligation_ids: Vec::new(),
                effects: operations,
                unknown_requirements,
            },
            evidence: evidence.clone(),
        };
        transition.validate()?;
        Ok(transition)
    }

    fn compile_generic_event(
        &self,
        node_index: u16,
        event_index: u8,
        parameter_0: u16,
        parameter_1: u16,
        raw_parameter_u32: u32,
        token: &str,
    ) -> Result<
        (
            Vec<StateOperation>,
            Vec<UnknownRequirement>,
            bool,
            MessageEventContinuation,
        ),
        PlannerContractError,
    > {
        let mut operations = Vec::new();
        let mut unknowns = Vec::new();
        let mut continuation = MessageEventContinuation::EncodedSuccessor;
        let fully_decoded = match event_index {
            3 | 5 => {
                if raw_parameter_u32 == 0 {
                    true
                } else if let Some(target) = match event_index {
                    3 => self.bindings.rupees.as_ref(),
                    5 => self.bindings.life.as_ref(),
                    _ => unreachable!(),
                } {
                    operations.push((
                        0,
                        StateOperation::DebitUnsigned {
                            target: target.clone(),
                            amount: raw_parameter_u32.into(),
                        },
                    ));
                    true
                } else {
                    false
                }
            }
            0 | 1 => {
                for access in self
                    .extracted
                    .persistent_flag_accesses
                    .iter()
                    .filter(|access| {
                        access.node_index == node_index
                            && matches!(
                                access.operation,
                                MessageFlowPersistentFlagOperation::Set
                                    | MessageFlowPersistentFlagOperation::Clear
                            )
                    })
                {
                    if self.bindings.persistent_flags.is_none() {
                        unknowns.push(unknown_flag_backing(
                            token,
                            node_index,
                            "persistent",
                            access.parameter_ordinal,
                            access.label_index,
                            &self.evidence,
                        ));
                    } else if let Some(operation) = self.compile_persistent_write(access) {
                        operations.push((access.parameter_ordinal, operation));
                    } else {
                        unknowns.push(unknown_flag_coordinate(
                            token,
                            node_index,
                            &format!("persistent-parameter-{}", access.parameter_ordinal),
                            access.label_index,
                            &self.evidence,
                        ));
                    }
                }
                let expected = usize::from(parameter_0 != 0) + usize::from(parameter_1 != 0);
                operations.len() + unknowns.len() == expected
            }
            10 | 11 => {
                for access in self
                    .extracted
                    .temporary_flag_accesses
                    .iter()
                    .filter(|access| {
                        access.node_index == node_index
                            && matches!(
                                access.operation,
                                MessageFlowTemporaryFlagOperation::Set
                                    | MessageFlowTemporaryFlagOperation::Clear
                            )
                    })
                {
                    if self.bindings.temporary_flags.is_none() {
                        unknowns.push(unknown_flag_backing(
                            token,
                            node_index,
                            "temporary",
                            access.parameter_ordinal,
                            access.label_index,
                            &self.evidence,
                        ));
                    } else if let Some(operation) = self.compile_temporary_write(access) {
                        operations.push((access.parameter_ordinal, operation));
                    } else {
                        unknowns.push(unknown_flag_coordinate(
                            token,
                            node_index,
                            &format!("temporary-parameter-{}", access.parameter_ordinal),
                            access.label_index,
                            &self.evidence,
                        ));
                    }
                }
                let expected = usize::from(parameter_0 != 0) + usize::from(parameter_1 != 0);
                operations.len() + unknowns.len() == expected
            }
            14 | 15 => {
                let accesses = self
                    .extracted
                    .switch_accesses
                    .iter()
                    .filter(|access| {
                        access.node_index == node_index
                            && matches!(
                                access.operation,
                                MessageFlowSwitchOperation::Set | MessageFlowSwitchOperation::Clear
                            )
                    })
                    .collect::<Vec<_>>();
                for access in accesses {
                    if let Some(operation) = self.compile_switch_write(access)? {
                        operations.push((0, operation));
                    } else {
                        unknowns.push(unknown_requirement(
                            token,
                            node_index,
                            "switch-backing",
                            format!(
                                "Switch store {:?} at node {node_index} has no audited backing binding",
                                access.store
                            ),
                            &self.evidence,
                        ));
                    }
                }
                operations.len() + unknowns.len() == 1
            }
            17 if parameter_1 <= 1 => {
                if let Some(item) = self.bindings.item(parameter_0) {
                    operations.push((
                        0,
                        StateOperation::WriteBoundRaw {
                            component_kind: item.component_kind.clone(),
                            binding: item.binding.clone(),
                            byte_offset: item.byte_offset,
                            mask: vec![item.mask],
                            value: vec![item.mask],
                        },
                    ));
                    true
                } else {
                    false
                }
            }
            8 => {
                operations.extend([
                    (
                        0,
                        StateOperation::Write {
                            target: ComponentFieldTarget {
                                component_id: self.flow_component_id.clone(),
                                field: "event_id".into(),
                            },
                            value: StateValue::Unsigned(parameter_0.into()),
                        },
                    ),
                    (
                        1,
                        StateOperation::Write {
                            target: ComponentFieldTarget {
                                component_id: self.flow_component_id.clone(),
                                field: "item_id".into(),
                            },
                            value: StateValue::Unsigned(parameter_1.into()),
                        },
                    ),
                ]);
                if parameter_0 == 27 {
                    unknowns.push(unknown_requirement(
                        token,
                        node_index,
                        "fundraising-side-effect",
                        "Event request 27 also updates message-object fundraising state whose backing is not yet imported".into(),
                        &self.evidence,
                    ));
                }
                true
            }
            9 => {
                continuation = MessageEventContinuation::ContractControlled;
                let flow_id = raw_parameter_u32 as u16;
                if flow_id == 0 {
                    unknowns.push(unknown_requirement(
                        token,
                        node_index,
                        "dynamic-group-jump",
                        "Flow jump 0 selects a runtime Midna/current-room message group that is not encoded in this resource".into(),
                        &self.evidence,
                    ));
                    true
                } else if let Some(label) = self
                    .extracted
                    .labels
                    .iter()
                    .find(|label| label.flow_id == flow_id)
                {
                    operations.push((
                        0,
                        StateOperation::AdvanceFlow {
                            flow_component_id: self.flow_component_id.clone(),
                            node_id: node_id(token, label.node_index),
                        },
                    ));
                    true
                } else {
                    unknowns.push(unknown_requirement(
                        token,
                        node_index,
                        "missing-flow-label",
                        format!(
                            "Flow jump {flow_id} has no entry label in the exact selected message resource"
                        ),
                        &self.evidence,
                    ));
                    true
                }
            }
            // These retail handlers return success without mutating modeled
            // state. `event012` is only a named message signal here; no door
            // state is changed by its handler.
            12 | 19 | 42 => true,
            _ => false,
        };
        operations.sort_by_key(|entry| entry.0);
        Ok((
            operations.into_iter().map(|entry| entry.1).collect(),
            unknowns,
            fully_decoded,
            continuation,
        ))
    }

    fn compile_numeric_branch(
        &self,
        query_handler_index: Option<u16>,
        parameter: u16,
        outcome: u8,
    ) -> Option<(PredicateExpression, ValueReference)> {
        // query004 returns 0 when the current rupee count is at least a
        // nonzero parameter and 1 when it is below that threshold. Its
        // parameter zero compares against the runtime wallet maximum, which is
        // not represented. query032 uses the same outcome convention against
        // current life for every parameter.
        let target = match query_handler_index {
            Some(4) if parameter != 0 => self.bindings.rupees.as_ref()?,
            Some(32) => self.bindings.life.as_ref()?,
            _ => return None,
        };
        let source = ValueReference::ComponentField {
            component_id: target.component_id.clone(),
            field: target.field.clone(),
        };
        let operator = match outcome {
            0 => ComparisonOperator::GreaterThanOrEqual,
            1 => ComparisonOperator::LessThan,
            _ => return None,
        };
        Some((
            PredicateExpression::Compare {
                left: source.clone(),
                operator,
                right: ValueReference::Literal {
                    value: StateValue::Unsigned(parameter.into()),
                },
            },
            source,
        ))
    }

    fn compile_temporary_write(
        &self,
        access: &MessageFlowTemporaryFlagAccess,
    ) -> Option<StateOperation> {
        let store = self.bindings.temporary_flags.as_ref()?;
        let packed = access.packed_backing_coordinate?;
        Some(raw_write(
            store,
            packed,
            access.operation == MessageFlowTemporaryFlagOperation::Set,
        ))
    }

    fn compile_persistent_write(
        &self,
        access: &MessageFlowPersistentFlagAccess,
    ) -> Option<StateOperation> {
        let store = self.bindings.persistent_flags.as_ref()?;
        let packed = access.packed_backing_coordinate?;
        Some(raw_write(
            store,
            packed,
            access.operation == MessageFlowPersistentFlagOperation::Set,
        ))
    }

    fn compile_switch_write(
        &self,
        access: &MessageFlowSwitchAccess,
    ) -> Result<Option<StateOperation>, PlannerContractError> {
        let Some(store) = self.bindings.switch_store(access.store) else {
            return Ok(None);
        };
        let (byte_offset, mask) = store.raw_location(access.switch_index)?;
        Ok(Some(StateOperation::WriteBoundRaw {
            component_kind: store.component_kind.clone(),
            binding: store.binding.clone(),
            byte_offset,
            mask: vec![mask],
            value: vec![if access.operation == MessageFlowSwitchOperation::Set {
                mask
            } else {
                0
            }],
        }))
    }

    fn branch_access(
        &self,
        node_index: u16,
        token: &str,
    ) -> Result<Option<CompiledBranchAccess>, PlannerContractError> {
        if let Some(access) = self
            .extracted
            .temporary_flag_accesses
            .iter()
            .find(|access| {
                access.node_index == node_index
                    && access.operation == MessageFlowTemporaryFlagOperation::BranchTrueWhenClear
            })
        {
            let Some(store) = &self.bindings.temporary_flags else {
                return Ok(None);
            };
            let Some(packed) = access.packed_backing_coordinate else {
                return Ok(None);
            };
            return Ok(Some(compiled_raw_access(
                store,
                packed,
                access
                    .friendly_name
                    .as_ref()
                    .map(|_| self.alias_id_with_token(token, "temporary", access.label_index)),
            )));
        }
        if let Some(access) = self
            .extracted
            .persistent_flag_accesses
            .iter()
            .find(|access| {
                access.node_index == node_index
                    && access.operation == MessageFlowPersistentFlagOperation::BranchTrueWhenClear
            })
        {
            let Some(store) = &self.bindings.persistent_flags else {
                return Ok(None);
            };
            let Some(packed) = access.packed_backing_coordinate else {
                return Ok(None);
            };
            return Ok(Some(compiled_raw_access(
                store,
                packed,
                access
                    .friendly_name
                    .as_ref()
                    .map(|_| self.alias_id_with_token(token, "persistent", access.label_index)),
            )));
        }
        if let Some(access) = self.extracted.switch_accesses.iter().find(|access| {
            access.node_index == node_index
                && access.operation == MessageFlowSwitchOperation::BranchTrueWhenClear
        }) {
            let Some(store) = self.bindings.switch_store(access.store) else {
                return Ok(None);
            };
            let (byte_offset, mask) = store.raw_location(access.switch_index)?;
            return Ok(Some(CompiledBranchAccess {
                reference: ValueReference::BoundRawBits {
                    component_kind: store.component_kind.clone(),
                    binding: store.binding.clone(),
                    byte_offset,
                    byte_width: 1,
                    mask: u64::from(mask),
                },
                mask,
                alias_id: None,
            }));
        }
        if let Some(MessageFlowNode::Branch {
            query_handler_index: Some(22),
            parameter,
            ..
        }) = self.extracted.nodes.get(usize::from(node_index))
            && let Some(item) = self.bindings.item(*parameter & 0x00ff)
        {
            return Ok(Some(CompiledBranchAccess {
                reference: ValueReference::BoundRawBits {
                    component_kind: item.component_kind.clone(),
                    binding: item.binding.clone(),
                    byte_offset: item.byte_offset,
                    byte_width: 1,
                    mask: u64::from(item.mask),
                },
                mask: item.mask,
                alias_id: Some(format!(
                    "fact.message-flow.{token}.item-ownership-{}",
                    item.item_id
                )),
            }));
        }
        Ok(None)
    }

    fn compile_cleanup(
        &self,
        cleanup: &MessageCleanupEdge,
    ) -> Result<CandidateTransition, PlannerContractError> {
        let store = self.bindings.temporary_flags.as_ref().expect("validated");
        let effects = cleanup
            .packed_backing_coordinates
            .iter()
            .map(|packed| raw_write(store, *packed, false))
            .collect();
        let transition = CandidateTransition {
            id: cleanup.transition_id.clone(),
            label: cleanup.label.clone(),
            scope: self.scope.clone(),
            transition_kind: TransitionKind::MessageAction,
            approach_id: cleanup.approach_id.clone(),
            activation: ActivationContract {
                hard_guards: cleanup.activation.clone(),
                physical_obligation_ids: Vec::new(),
                effects,
                unknown_requirements: Vec::new(),
            },
            evidence: cleanup.evidence.clone(),
        };
        transition.validate()?;
        Ok(transition)
    }

    fn compile_aliases(&self, token: &str) -> Result<Vec<FriendlyAlias>, PlannerContractError> {
        let mut aliases = BTreeMap::<String, FriendlyAlias>::new();
        if let Some(store) = &self.bindings.temporary_flags {
            for access in &self.extracted.temporary_flag_accesses {
                if let (Some(packed), Some(label)) =
                    (access.packed_backing_coordinate, &access.friendly_name)
                {
                    let id = self.alias_id_with_token(token, "temporary", access.label_index);
                    insert_alias(&mut aliases, id, label, store, packed, self)?;
                }
            }
        }
        if let Some(store) = &self.bindings.persistent_flags {
            for access in &self.extracted.persistent_flag_accesses {
                if let (Some(packed), Some(label)) =
                    (access.packed_backing_coordinate, &access.friendly_name)
                {
                    let id = self.alias_id_with_token(token, "persistent", access.label_index);
                    insert_alias(&mut aliases, id, label, store, packed, self)?;
                }
            }
        }
        for item in &self.bindings.item_ownership {
            if !self.extracted.nodes.iter().any(|node| match node {
                MessageFlowNode::Branch {
                    query_handler_index: Some(22),
                    parameter,
                    ..
                } => (*parameter & 0x00ff) == item.item_id,
                MessageFlowNode::Event {
                    event_index: 17,
                    parameter_0,
                    parameter_1,
                    ..
                } => *parameter_0 == item.item_id && *parameter_1 <= 1,
                _ => false,
            }) {
                continue;
            }
            let id = format!("fact.message-flow.{token}.item-ownership-{}", item.item_id);
            let alias = FriendlyAlias {
                id: id.clone(),
                label: item.label.clone(),
                scope: self.scope.clone(),
                raw: RawFactBinding {
                    component_kind: item.component_kind.clone(),
                    binding: item.binding.clone(),
                    byte_offset: item.byte_offset,
                    mask: vec![item.mask],
                    expected: vec![item.mask],
                },
                evidence: self.evidence.clone(),
            };
            if aliases.insert(id, alias).is_some() {
                return Err(PlannerContractError::new(
                    "message_flow_program.aliases",
                    "item ownership alias is duplicated",
                ));
            }
        }
        Ok(aliases.into_values().collect())
    }

    fn alias_id_with_token(&self, token: &str, kind: &str, label_index: u16) -> String {
        format!("fact.message-flow.{token}.{kind}-label-{label_index}")
    }
}
