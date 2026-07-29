use super::*;

#[derive(Clone)]
pub(super) struct CompiledBranchAccess {
    pub(super) reference: ValueReference,
    pub(super) mask: u8,
    pub(super) alias_id: Option<String>,
}

pub(super) fn validate_extracted(
    extracted: &ExtractedMessageFlow,
) -> Result<(), PlannerContractError> {
    if extracted.resource_size == 0
        || extracted.header_declared_size > extracted.resource_size
        || extracted.nodes.is_empty()
        || extracted.nodes.len() > MAX_MESSAGE_FLOW_NODES
        || usize::from(extracted.node_count) != extracted.nodes.len()
        || usize::from(extracted.branch_target_count) != extracted.branch_targets.len()
    {
        return Err(PlannerContractError::new(
            "message_flow_program.extracted",
            "must have matching bounded node and branch-target counts",
        ));
    }
    for (index, node) in extracted.nodes.iter().enumerate() {
        let expected = index as u16;
        if message_node_index(node) != expected {
            return Err(PlannerContractError::new(
                "message_flow_program.extracted.nodes",
                "node indices must be dense and ordered",
            ));
        }
        match node {
            MessageFlowNode::Message {
                next_node_index, ..
            } => validate_node_target(*next_node_index, extracted.node_count)?,
            MessageFlowNode::Event {
                next_target_index, ..
            } => {
                let Some(target) = extracted
                    .branch_targets
                    .get(usize::from(*next_target_index))
                else {
                    return Err(PlannerContractError::new(
                        "message_flow_program.extracted.event",
                        "must reference one encoded target-table entry",
                    ));
                };
                validate_node_target(*target, extracted.node_count)?;
            }
            MessageFlowNode::Branch {
                next_target_index, ..
            } => {
                let start = usize::from(*next_target_index);
                if start
                    .checked_add(1)
                    .is_none_or(|last| last >= extracted.branch_targets.len())
                {
                    return Err(PlannerContractError::new(
                        "message_flow_program.extracted.branch",
                        "must reference two branch targets",
                    ));
                }
            }
            MessageFlowNode::Unknown { .. } => {}
        }
    }
    for target in &extracted.branch_targets {
        validate_node_target(*target, extracted.node_count)?;
    }
    let mut prior_flow = None;
    for label in &extracted.labels {
        validate_node_target(label.node_index, extracted.node_count)?;
        if prior_flow.is_some_and(|flow_id| flow_id >= label.flow_id) {
            return Err(PlannerContractError::new(
                "message_flow_program.extracted.labels",
                "must be unique and sorted by flow ID",
            ));
        }
        prior_flow = Some(label.flow_id);
    }
    validate_accesses(extracted)
}

pub(super) fn validate_accesses(
    extracted: &ExtractedMessageFlow,
) -> Result<(), PlannerContractError> {
    let mut prior_temporary = None;
    for access in &extracted.temporary_flag_accesses {
        if access.node_index >= extracted.node_count
            || prior_temporary
                .is_some_and(|key| key >= (access.node_index, access.parameter_ordinal))
        {
            return Err(PlannerContractError::new(
                "message_flow_program.extracted.temporary_flag_accesses",
                "must have unique in-range node/parameter coordinates",
            ));
        }
        prior_temporary = Some((access.node_index, access.parameter_ordinal));
        validate_temporary_access(access, extracted)?;
        if access
            .packed_backing_coordinate
            .is_some_and(|packed| !(packed as u8).is_power_of_two())
        {
            return Err(PlannerContractError::new(
                "message_flow_program.extracted.temporary_flag_accesses",
                "packed coordinates must contain exactly one selected bit",
            ));
        }
        validate_access_name(
            "message_flow_program.extracted.temporary_flag_accesses",
            access.packed_backing_coordinate,
            access.friendly_name.as_deref(),
        )?;
    }
    let mut prior_persistent = None;
    for access in &extracted.persistent_flag_accesses {
        if access.node_index >= extracted.node_count
            || prior_persistent
                .is_some_and(|key| key >= (access.node_index, access.parameter_ordinal))
        {
            return Err(PlannerContractError::new(
                "message_flow_program.extracted.persistent_flag_accesses",
                "must have unique in-range node/parameter coordinates",
            ));
        }
        prior_persistent = Some((access.node_index, access.parameter_ordinal));
        validate_persistent_access(access, extracted)?;
        if access
            .packed_backing_coordinate
            .is_some_and(|packed| !(packed as u8).is_power_of_two())
        {
            return Err(PlannerContractError::new(
                "message_flow_program.extracted.persistent_flag_accesses",
                "packed coordinates must contain exactly one selected bit",
            ));
        }
        validate_access_name(
            "message_flow_program.extracted.persistent_flag_accesses",
            access.packed_backing_coordinate,
            access.friendly_name.as_deref(),
        )?;
    }
    let mut prior_switch = None;
    for access in &extracted.switch_accesses {
        let key = (access.node_index, switch_store_key(access.store));
        if access.node_index >= extracted.node_count
            || prior_switch.is_some_and(|prior| prior >= key)
        {
            return Err(PlannerContractError::new(
                "message_flow_program.extracted.switch_accesses",
                "must have unique in-range node/store coordinates",
            ));
        }
        prior_switch = Some(key);
        validate_switch_access(access, extracted)?;
    }
    Ok(())
}

pub(super) fn validate_temporary_access(
    access: &MessageFlowTemporaryFlagAccess,
    extracted: &ExtractedMessageFlow,
) -> Result<(), PlannerContractError> {
    let node = &extracted.nodes[usize::from(access.node_index)];
    let valid = match (access.operation, node) {
        (
            MessageFlowTemporaryFlagOperation::Set,
            MessageFlowNode::Event {
                event_index: 10,
                parameter_0,
                parameter_1,
                ..
            },
        )
        | (
            MessageFlowTemporaryFlagOperation::Clear,
            MessageFlowNode::Event {
                event_index: 11,
                parameter_0,
                parameter_1,
                ..
            },
        ) => event_parameter_matches(
            access.parameter_ordinal,
            access.label_index,
            *parameter_0,
            *parameter_1,
        ),
        (
            MessageFlowTemporaryFlagOperation::BranchTrueWhenClear,
            MessageFlowNode::Branch {
                query_handler_index: Some(11),
                parameter,
                ..
            },
        ) => access.parameter_ordinal == 0 && access.label_index == *parameter,
        _ => false,
    };
    require_valid_access(
        valid,
        "message_flow_program.extracted.temporary_flag_accesses",
    )
}

pub(super) fn validate_persistent_access(
    access: &MessageFlowPersistentFlagAccess,
    extracted: &ExtractedMessageFlow,
) -> Result<(), PlannerContractError> {
    let node = &extracted.nodes[usize::from(access.node_index)];
    let valid = match (access.operation, node) {
        (
            MessageFlowPersistentFlagOperation::Set,
            MessageFlowNode::Event {
                event_index: 0,
                parameter_0,
                parameter_1,
                ..
            },
        )
        | (
            MessageFlowPersistentFlagOperation::Clear,
            MessageFlowNode::Event {
                event_index: 1,
                parameter_0,
                parameter_1,
                ..
            },
        ) => event_parameter_matches(
            access.parameter_ordinal,
            access.label_index,
            *parameter_0,
            *parameter_1,
        ),
        (
            MessageFlowPersistentFlagOperation::BranchTrueWhenClear,
            MessageFlowNode::Branch {
                query_handler_index: Some(1),
                parameter,
                ..
            },
        ) => access.parameter_ordinal == 0 && access.label_index == *parameter,
        _ => false,
    };
    require_valid_access(
        valid,
        "message_flow_program.extracted.persistent_flag_accesses",
    )
}

pub(super) fn validate_switch_access(
    access: &MessageFlowSwitchAccess,
    extracted: &ExtractedMessageFlow,
) -> Result<(), PlannerContractError> {
    let node = &extracted.nodes[usize::from(access.node_index)];
    let valid = match (access.operation, node) {
        (
            MessageFlowSwitchOperation::Set,
            MessageFlowNode::Event {
                event_index: 14,
                parameter_0,
                parameter_1,
                ..
            },
        )
        | (
            MessageFlowSwitchOperation::Clear,
            MessageFlowNode::Event {
                event_index: 15,
                parameter_0,
                parameter_1,
                ..
            },
        ) => {
            switch_store_from_selector(*parameter_0) == Some(access.store)
                && *parameter_1 == access.switch_index
        }
        (
            MessageFlowSwitchOperation::BranchTrueWhenClear,
            MessageFlowNode::Branch {
                query_handler_index: Some(handler),
                parameter,
                ..
            },
        ) => {
            switch_store_from_query(*handler) == Some(access.store)
                && *parameter == access.switch_index
        }
        _ => false,
    };
    require_valid_access(valid, "message_flow_program.extracted.switch_accesses")
}

pub(super) fn event_parameter_matches(
    ordinal: u8,
    label_index: u16,
    parameter_0: u16,
    parameter_1: u16,
) -> bool {
    match ordinal {
        0 => label_index != 0 && label_index == parameter_0,
        1 => label_index != 0 && label_index == parameter_1,
        _ => false,
    }
}

pub(super) fn require_valid_access(valid: bool, field: &str) -> Result<(), PlannerContractError> {
    if valid {
        Ok(())
    } else {
        Err(PlannerContractError::new(
            field,
            "does not match the referenced node handler and parameters",
        ))
    }
}

pub(super) fn validate_access_name(
    field: &str,
    coordinate: Option<u16>,
    friendly_name: Option<&str>,
) -> Result<(), PlannerContractError> {
    if coordinate.is_some() != friendly_name.is_some() {
        return Err(PlannerContractError::new(
            field,
            "a known coordinate and friendly name must be present together",
        ));
    }
    if let Some(name) = friendly_name {
        validate_stable_id(field, name)?;
    }
    Ok(())
}

pub(super) fn switch_store_from_selector(selector: u16) -> Option<MessageFlowSwitchStore> {
    Some(match selector {
        0 => MessageFlowSwitchStore::LoadedStageMemory,
        1 => MessageFlowSwitchStore::Dungeon,
        2 => MessageFlowSwitchStore::Zone,
        3 => MessageFlowSwitchStore::OneZone,
        _ => return None,
    })
}

pub(super) fn switch_store_from_query(handler: u16) -> Option<MessageFlowSwitchStore> {
    Some(match handler {
        13 => MessageFlowSwitchStore::LoadedStageMemory,
        15 => MessageFlowSwitchStore::Dungeon,
        17 => MessageFlowSwitchStore::Zone,
        19 => MessageFlowSwitchStore::OneZone,
        _ => return None,
    })
}

pub(super) fn validate_event_contracts(
    contracts: &[MessageEventContract],
    extracted: &ExtractedMessageFlow,
    flow_component_id: &str,
) -> Result<(), PlannerContractError> {
    if contracts.len() > MAX_EVENT_CONTRACTS {
        return Err(PlannerContractError::new(
            "message_flow_program.event_contracts",
            "contains too many records",
        ));
    }
    let mut prior = None;
    for contract in contracts {
        if prior.is_some_and(|index| index >= contract.node_index) {
            return Err(PlannerContractError::new(
                "message_flow_program.event_contracts",
                "must be unique and sorted by node index",
            ));
        }
        prior = Some(contract.node_index);
        let Some(node @ MessageFlowNode::Event { .. }) =
            extracted.nodes.get(usize::from(contract.node_index))
        else {
            return Err(PlannerContractError::new(
                "message_flow_program.event_contracts.node_index",
                "must reference an event node",
            ));
        };
        if generic_event_is_fully_decidable(node, extracted) {
            return Err(PlannerContractError::new(
                "message_flow_program.event_contracts.node_index",
                "must not replace a generic handler decoded by the compiler",
            ));
        }
        if contract.confirmed_operations.is_empty() {
            return Err(PlannerContractError::new(
                "message_flow_program.event_contracts.confirmed_operations",
                "must not be empty",
            ));
        }
        for operation in &contract.confirmed_operations {
            operation.validate()?;
        }
        contract
            .evidence
            .validate("message_flow_program.event_contracts.evidence")?;
        if contract.evidence.truth == TruthStatus::Unknown {
            return Err(PlannerContractError::new(
                "message_flow_program.event_contracts.evidence",
                "an exact contract cannot have unknown truth",
            ));
        }
        let flow_operations = contract
            .confirmed_operations
            .iter()
            .filter_map(|operation| match operation {
                StateOperation::AdvanceFlow {
                    flow_component_id, ..
                }
                | StateOperation::BranchFlow {
                    flow_component_id, ..
                } => Some(flow_component_id),
                _ => None,
            })
            .collect::<Vec<_>>();
        match contract.continuation {
            MessageEventContinuation::EncodedSuccessor if !flow_operations.is_empty() => {
                return Err(PlannerContractError::new(
                    "message_flow_program.event_contracts.continuation",
                    "encoded-successor contracts must not also write message flow",
                ));
            }
            MessageEventContinuation::ContractControlled
                if flow_operations.len() != 1 || flow_operations[0] != flow_component_id =>
            {
                return Err(PlannerContractError::new(
                    "message_flow_program.event_contracts.continuation",
                    "contract-controlled flow must contain exactly one operation for the program flow component",
                ));
            }
            MessageEventContinuation::EncodedSuccessor
            | MessageEventContinuation::ContractControlled => {}
        }
    }
    Ok(())
}

pub(super) fn generic_event_is_fully_decidable(
    node: &MessageFlowNode,
    extracted: &ExtractedMessageFlow,
) -> bool {
    match node {
        MessageFlowNode::Event { event_index, .. }
            if matches!(*event_index, 0 | 1 | 10 | 11 | 12 | 14 | 15 | 19 | 42) =>
        {
            true
        }
        MessageFlowNode::Event {
            event_index: 9,
            raw_parameter_u32,
            ..
        } => {
            let flow_id = *raw_parameter_u32 as u16;
            flow_id != 0
                && extracted
                    .labels
                    .iter()
                    .any(|label| label.flow_id == flow_id)
        }
        _ => false,
    }
}

pub(super) fn validate_cleanup_edges(
    edges: &[MessageCleanupEdge],
    bindings: &MessageFlowBindings,
) -> Result<(), PlannerContractError> {
    if edges.len() > MAX_CLEANUP_EDGES {
        return Err(PlannerContractError::new(
            "message_flow_program.cleanup_edges",
            "contains too many records",
        ));
    }
    if !edges.is_empty() && bindings.temporary_flags.is_none() {
        return Err(PlannerContractError::new(
            "message_flow_program.cleanup_edges",
            "require a temporary-flag binding",
        ));
    }
    let mut prior = None;
    for edge in edges {
        validate_stable_id(
            "message_flow_program.cleanup.transition_id",
            &edge.transition_id,
        )?;
        validate_label("message_flow_program.cleanup.label", &edge.label)?;
        validate_stable_id(
            "message_flow_program.cleanup.approach_id",
            &edge.approach_id,
        )?;
        edge.activation.validate()?;
        if matches!(
            edge.activation,
            PredicateExpression::True | PredicateExpression::False
        ) {
            return Err(PlannerContractError::new(
                "message_flow_program.cleanup.activation",
                "must name the caller-specific cleanup condition",
            ));
        }
        edge.evidence
            .validate("message_flow_program.cleanup.evidence")?;
        if prior.is_some_and(|id: &str| id >= edge.transition_id.as_str()) {
            return Err(PlannerContractError::new(
                "message_flow_program.cleanup_edges",
                "must be unique and sorted by transition ID",
            ));
        }
        prior = Some(edge.transition_id.as_str());
        if edge.packed_backing_coordinates.is_empty()
            || edge
                .packed_backing_coordinates
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
            || edge
                .packed_backing_coordinates
                .iter()
                .any(|packed| !(*packed as u8).is_power_of_two())
        {
            return Err(PlannerContractError::new(
                "message_flow_program.cleanup.packed_backing_coordinates",
                "must be a nonempty sorted unique list of single-bit coordinates",
            ));
        }
    }
    Ok(())
}

pub(super) fn insert_alias(
    aliases: &mut BTreeMap<String, FriendlyAlias>,
    id: String,
    label: &str,
    store: &MessageRawStoreBinding,
    packed: u16,
    program: &MessageFlowProgram,
) -> Result<(), PlannerContractError> {
    let (byte_offset, mask) = unpack_coordinate(packed);
    let alias = FriendlyAlias {
        id: id.clone(),
        label: label.replace('_', " "),
        scope: program.scope.clone(),
        raw: RawFactBinding {
            component_kind: store.component_kind.clone(),
            binding: store.binding.clone(),
            byte_offset,
            mask: vec![mask],
            expected: vec![mask],
        },
        evidence: program.evidence.clone(),
    };
    if let Some(previous) = aliases.insert(id, alias.clone())
        && previous != alias
    {
        return Err(PlannerContractError::new(
            "message_flow_program.aliases",
            "one label resolves to conflicting backing coordinates",
        ));
    }
    Ok(())
}

pub(super) fn raw_write(store: &MessageRawStoreBinding, packed: u16, set: bool) -> StateOperation {
    let (byte_offset, mask) = unpack_coordinate(packed);
    StateOperation::WriteBoundRaw {
        component_kind: store.component_kind.clone(),
        binding: store.binding.clone(),
        byte_offset,
        mask: vec![mask],
        value: vec![if set { mask } else { 0 }],
    }
}

pub(super) fn compiled_raw_access(
    store: &MessageRawStoreBinding,
    packed: u16,
    alias_id: Option<String>,
) -> CompiledBranchAccess {
    let (byte_offset, mask) = unpack_coordinate(packed);
    CompiledBranchAccess {
        reference: ValueReference::BoundRawBits {
            component_kind: store.component_kind.clone(),
            binding: store.binding.clone(),
            byte_offset,
            byte_width: 1,
            mask: u64::from(mask),
        },
        mask,
        alias_id,
    }
}

pub(super) fn raw_branch_guard(access: &CompiledBranchAccess, outcome: u8) -> PredicateExpression {
    PredicateExpression::Compare {
        left: access.reference.clone(),
        operator: ComparisonOperator::Equal,
        right: ValueReference::Literal {
            value: StateValue::Unsigned(if outcome == 1 {
                0
            } else {
                u64::from(access.mask)
            }),
        },
    }
}

pub(super) fn flow_node_guard(flow_component_id: &str, expected: &str) -> PredicateExpression {
    PredicateExpression::Compare {
        left: ValueReference::FlowNode {
            flow_component_id: flow_component_id.into(),
        },
        operator: ComparisonOperator::Equal,
        right: ValueReference::Literal {
            value: StateValue::Text(expected.into()),
        },
    }
}

pub(super) fn all_guards(mut guards: Vec<PredicateExpression>) -> PredicateExpression {
    if guards.len() == 1 {
        guards.pop().unwrap()
    } else {
        PredicateExpression::All { terms: guards }
    }
}

pub(super) fn unpack_coordinate(packed: u16) -> (u32, u8) {
    (u32::from(packed >> 8), packed as u8)
}

pub(super) fn message_node_index(node: &MessageFlowNode) -> u16 {
    match node {
        MessageFlowNode::Message { index, .. }
        | MessageFlowNode::Branch { index, .. }
        | MessageFlowNode::Event { index, .. }
        | MessageFlowNode::Unknown { index, .. } => *index,
    }
}

pub(super) fn validate_node_target(
    target: u16,
    node_count: u16,
) -> Result<(), PlannerContractError> {
    if target != u16::MAX && target >= node_count {
        return Err(PlannerContractError::new(
            "message_flow_program.extracted.target",
            format!("node target {target} exceeds node count {node_count}"),
        ));
    }
    Ok(())
}

pub(super) fn node_id(token: &str, index: u16) -> String {
    format!("message-node.{token}.{index}")
}

pub(super) fn target_node_id(token: &str, target: u16, terminal_node_id: &str) -> String {
    if target == u16::MAX {
        terminal_node_id.into()
    } else {
        node_id(token, target)
    }
}

pub(super) fn validate_language_token(
    field: &str,
    value: &str,
    allow_separator: bool,
) -> Result<(), PlannerContractError> {
    let valid = !value.is_empty()
        && value.len() <= 32
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || (allow_separator && byte == b'-')
        });
    if !valid {
        return Err(PlannerContractError::new(
            field,
            "must be a lowercase ASCII language or locale token",
        ));
    }
    Ok(())
}

pub(super) fn short_token(digest: Digest) -> String {
    digest.as_bytes()[..12]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(super) fn switch_store_key(store: MessageFlowSwitchStore) -> u8 {
    match store {
        MessageFlowSwitchStore::LoadedStageMemory => 0,
        MessageFlowSwitchStore::Dungeon => 1,
        MessageFlowSwitchStore::Zone => 2,
        MessageFlowSwitchStore::OneZone => 3,
    }
}

pub(super) fn unknown_requirement(
    token: &str,
    node_index: u16,
    kind: &str,
    description: String,
    evidence: &RuleEvidence,
) -> UnknownRequirement {
    UnknownRequirement {
        id: format!("unknown.message-flow.{token}.node-{node_index}.{kind}"),
        description,
        evidence: RuleEvidence {
            truth: TruthStatus::Unknown,
            records: evidence.records.clone(),
        },
    }
}

pub(super) fn unknown_flag_coordinate(
    token: &str,
    node_index: u16,
    kind: &str,
    label_index: u16,
    evidence: &RuleEvidence,
) -> UnknownRequirement {
    unknown_requirement(
        token,
        node_index,
        &format!("{kind}-label-{label_index}"),
        format!(
            "{kind} flag label {label_index} at node {node_index} has no imported backing coordinate"
        ),
        evidence,
    )
}
