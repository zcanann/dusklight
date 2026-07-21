//! Compilation and catalog integration for exact selected message programs.

use crate::artifact::Digest;
use crate::identity::{ContextSelector, ExactContext, RuntimeConfiguration};
use crate::logic::{ContextScope, FACT_CATALOG_SCHEMA, FactCatalog, TruthStatus};
use crate::message_flow::{
    CompiledMessageFlowProgram, MessageCleanupEdge, MessageEventContinuation, MessageEventContract,
    MessageFlowImportProfile, MessageFlowProgram, MessageFlowProgramSet,
};
use crate::orig_discovery::ExtractedOrigBundle;
use crate::transition::{MECHANICS_CATALOG_SCHEMA, MechanicsCatalog, StateOperation};
use crate::{PlannerContractError, canonical_json, validate_label, validate_stable_id};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

pub const MESSAGE_FLOW_RESOURCE_OVERLAY_SET_SCHEMA: &str =
    "dusklight.route-planner.message-flow-resource-overlay-set/v1";
pub const COMPILED_MESSAGE_FLOW_SET_SCHEMA: &str =
    "dusklight.route-planner.compiled-message-flow-set/v1";
const MAX_RESOURCE_OVERLAYS: usize = 256;

/// Node contracts are separate from the reusable content/language/backing
/// profile. Every node index is pinned to one extracted resource digest so a
/// language or revision change cannot silently reuse it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MessageFlowResourceOverlaySet {
    pub schema: String,
    pub id: String,
    pub import_profile_sha256: Digest,
    pub resources: Vec<MessageFlowResourceOverlay>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MessageFlowResourceOverlay {
    pub message_group: u8,
    pub resource_sha256: Digest,
    pub event_contracts: Vec<MessageEventContract>,
    pub cleanup_edges: Vec<MessageCleanupEdge>,
}

/// Solver-ready deterministic merge of all programs selected for one exact
/// runtime context.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledMessageFlowSet {
    pub schema: String,
    pub program_set_sha256: Digest,
    pub overlay_set_sha256: Option<Digest>,
    pub exact_context: ExactContext,
    pub locale_bundle: String,
    pub resources: Vec<CompiledMessageFlowResource>,
    pub facts: FactCatalog,
    pub mechanics: MechanicsCatalog,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledMessageFlowResource {
    pub message_group: u8,
    pub archive_sha256: Digest,
    pub resource_sha256: Digest,
    pub source_program: MessageFlowProgram,
    pub compiled_program: CompiledMessageFlowProgram,
}

impl MessageFlowResourceOverlaySet {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != MESSAGE_FLOW_RESOURCE_OVERLAY_SET_SCHEMA {
            return Err(PlannerContractError::new(
                "message_flow_resource_overlay_set.schema",
                "is unsupported",
            ));
        }
        validate_stable_id("message_flow_resource_overlay_set.id", &self.id)?;
        require_digest(
            "message_flow_resource_overlay_set.import_profile_sha256",
            self.import_profile_sha256,
        )?;
        if self.resources.is_empty() || self.resources.len() > MAX_RESOURCE_OVERLAYS {
            return Err(PlannerContractError::new(
                "message_flow_resource_overlay_set.resources",
                "must contain between 1 and 256 records",
            ));
        }
        let mut prior = None;
        for resource in &self.resources {
            if prior.is_some_and(|group| group >= resource.message_group) {
                return Err(PlannerContractError::new(
                    "message_flow_resource_overlay_set.resources",
                    "must be unique and sorted by message group",
                ));
            }
            prior = Some(resource.message_group);
            resource.validate()?;
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
                "message_flow_resource_overlay_set",
                "is not canonical JSON",
            ));
        }
        Ok(set)
    }

    pub fn digest(&self) -> Result<Digest, PlannerContractError> {
        Ok(Digest(Sha256::digest(self.canonical_bytes()?).into()))
    }
}

impl MessageFlowResourceOverlay {
    fn validate(&self) -> Result<(), PlannerContractError> {
        require_digest(
            "message_flow_resource_overlay.resource_sha256",
            self.resource_sha256,
        )?;
        if self.event_contracts.is_empty() && self.cleanup_edges.is_empty() {
            return Err(PlannerContractError::new(
                "message_flow_resource_overlay",
                "must contain an event contract or cleanup edge",
            ));
        }
        let mut prior_node = None;
        for contract in &self.event_contracts {
            if prior_node.is_some_and(|node| node >= contract.node_index) {
                return Err(PlannerContractError::new(
                    "message_flow_resource_overlay.event_contracts",
                    "must be unique and sorted by node index",
                ));
            }
            prior_node = Some(contract.node_index);
            if contract.confirmed_operations.is_empty() {
                return Err(PlannerContractError::new(
                    "message_flow_resource_overlay.event_contracts",
                    "must contain confirmed operations",
                ));
            }
            for operation in &contract.confirmed_operations {
                operation.validate()?;
            }
            contract
                .evidence
                .validate("message_flow_resource_overlay.event_contracts.evidence")?;
            if contract.evidence.truth == TruthStatus::Unknown {
                return Err(PlannerContractError::new(
                    "message_flow_resource_overlay.event_contracts.evidence",
                    "an exact contract cannot have unknown truth",
                ));
            }
        }
        let mut prior_cleanup = None;
        for cleanup in &self.cleanup_edges {
            validate_stable_id(
                "message_flow_resource_overlay.cleanup.transition_id",
                &cleanup.transition_id,
            )?;
            validate_label(
                "message_flow_resource_overlay.cleanup.label",
                &cleanup.label,
            )?;
            validate_stable_id(
                "message_flow_resource_overlay.cleanup.approach_id",
                &cleanup.approach_id,
            )?;
            cleanup.activation.validate()?;
            if matches!(
                cleanup.activation,
                crate::logic::PredicateExpression::True | crate::logic::PredicateExpression::False
            ) {
                return Err(PlannerContractError::new(
                    "message_flow_resource_overlay.cleanup.activation",
                    "must name the caller-specific cleanup condition",
                ));
            }
            cleanup
                .evidence
                .validate("message_flow_resource_overlay.cleanup.evidence")?;
            if cleanup.evidence.truth == TruthStatus::Unknown {
                return Err(PlannerContractError::new(
                    "message_flow_resource_overlay.cleanup.evidence",
                    "an exact cleanup caller cannot have unknown truth",
                ));
            }
            if prior_cleanup.is_some_and(|id: &str| id >= cleanup.transition_id.as_str()) {
                return Err(PlannerContractError::new(
                    "message_flow_resource_overlay.cleanup_edges",
                    "must be unique and sorted by transition ID",
                ));
            }
            prior_cleanup = Some(cleanup.transition_id.as_str());
            if cleanup.packed_backing_coordinates.is_empty()
                || cleanup
                    .packed_backing_coordinates
                    .windows(2)
                    .any(|pair| pair[0] >= pair[1])
                || cleanup
                    .packed_backing_coordinates
                    .iter()
                    .any(|packed| !(*packed as u8).is_power_of_two())
            {
                return Err(PlannerContractError::new(
                    "message_flow_resource_overlay.cleanup.coordinates",
                    "must be sorted unique single-bit coordinates",
                ));
            }
        }
        Ok(())
    }
}

impl CompiledMessageFlowSet {
    pub fn build(
        bundle: &ExtractedOrigBundle,
        runtime: &RuntimeConfiguration,
        profile: &MessageFlowImportProfile,
        overlays: Option<&MessageFlowResourceOverlaySet>,
    ) -> Result<Self, PlannerContractError> {
        bundle.validate()?;
        runtime.validate()?;
        profile.validate()?;
        let program_set = MessageFlowProgramSet::build(bundle, runtime, profile)?;
        if let Some(overlays) = overlays {
            overlays.validate()?;
            if overlays.import_profile_sha256 != program_set.profile_sha256 {
                return Err(PlannerContractError::new(
                    "message_flow_resource_overlay_set.import_profile_sha256",
                    "does not match the selected import profile",
                ));
            }
        }
        let selected = bundle
            .message_flows
            .iter()
            .filter(|record| record.locale_bundle == program_set.locale_bundle)
            .map(|record| (record.message_group, record))
            .collect::<std::collections::BTreeMap<_, _>>();
        let mut resources = Vec::with_capacity(program_set.programs.len());
        for mut program in program_set.programs.clone() {
            let record = selected
                .get(&u16::from(program.message_group))
                .ok_or_else(|| {
                    PlannerContractError::new(
                        "compiled_message_flow_set.resources",
                        "selected program has no exact source record",
                    )
                })?;
            if let Some(overlay) = overlays.and_then(|set| {
                set.resources
                    .iter()
                    .find(|overlay| overlay.message_group == program.message_group)
            }) {
                if overlay.resource_sha256 != program.resource_sha256 {
                    return Err(PlannerContractError::new(
                        "message_flow_resource_overlay.resource_sha256",
                        format!(
                            "does not match selected message group {}",
                            program.message_group
                        ),
                    ));
                }
                validate_contract_flow(&overlay.event_contracts, &program.flow_component_id)?;
                program.event_contracts = overlay.event_contracts.clone();
                program.cleanup_edges = overlay.cleanup_edges.clone();
            }
            resources.push(CompiledMessageFlowResource {
                message_group: program.message_group,
                archive_sha256: record.archive_sha256,
                resource_sha256: record.resource_sha256,
                compiled_program: program.compile()?,
                source_program: program,
            });
        }
        if let Some(overlays) = overlays {
            for overlay in &overlays.resources {
                if !resources.iter().any(|resource| {
                    resource.message_group == overlay.message_group
                        && resource.resource_sha256 == overlay.resource_sha256
                }) {
                    return Err(PlannerContractError::new(
                        "message_flow_resource_overlay_set.resources",
                        "contains an overlay outside the selected locale/resources",
                    ));
                }
            }
        }
        let (facts, mechanics) = merged_catalogs(&resources)?;
        let set = Self {
            schema: COMPILED_MESSAGE_FLOW_SET_SCHEMA.into(),
            program_set_sha256: program_set.digest()?,
            overlay_set_sha256: overlays
                .map(MessageFlowResourceOverlaySet::digest)
                .transpose()?,
            exact_context: program_set.exact_context,
            locale_bundle: program_set.locale_bundle,
            resources,
            facts,
            mechanics,
        };
        set.validate()?;
        Ok(set)
    }

    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != COMPILED_MESSAGE_FLOW_SET_SCHEMA {
            return Err(PlannerContractError::new(
                "compiled_message_flow_set.schema",
                "is unsupported",
            ));
        }
        require_digest(
            "compiled_message_flow_set.program_set_sha256",
            self.program_set_sha256,
        )?;
        if self.overlay_set_sha256 == Some(Digest::ZERO) {
            return Err(PlannerContractError::new(
                "compiled_message_flow_set.overlay_set_sha256",
                "must be absent or nonzero",
            ));
        }
        if self.locale_bundle.is_empty()
            || !self
                .locale_bundle
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric())
        {
            return Err(PlannerContractError::new(
                "compiled_message_flow_set.locale_bundle",
                "must be nonempty ASCII letters or digits",
            ));
        }
        if self.resources.is_empty() {
            return Err(PlannerContractError::new(
                "compiled_message_flow_set.resources",
                "must not be empty",
            ));
        }
        let expected_scope = ContextScope {
            selectors: vec![ContextSelector::Exact {
                context: self.exact_context.clone(),
            }],
        };
        expected_scope.validate("compiled_message_flow_set.exact_context")?;
        let mut prior = None;
        let mut flow_component_id = None;
        for resource in &self.resources {
            if prior.is_some_and(|group| group >= resource.message_group) {
                return Err(PlannerContractError::new(
                    "compiled_message_flow_set.resources",
                    "must be unique and sorted by message group",
                ));
            }
            prior = Some(resource.message_group);
            require_digest(
                "compiled_message_flow_set.resource.archive_sha256",
                resource.archive_sha256,
            )?;
            require_digest(
                "compiled_message_flow_set.resource.resource_sha256",
                resource.resource_sha256,
            )?;
            resource.source_program.validate()?;
            resource.compiled_program.validate()?;
            if resource.source_program.message_group != resource.message_group
                || resource.source_program.resource_sha256 != resource.resource_sha256
                || resource.source_program.scope != expected_scope
                || resource.source_program.compile()? != resource.compiled_program
            {
                return Err(PlannerContractError::new(
                    "compiled_message_flow_set.resource.program",
                    "does not reproduce the exact group, resource, scope, and compiled artifact",
                ));
            }
            if flow_component_id
                .as_ref()
                .is_some_and(|id| id != &resource.source_program.flow_component_id)
            {
                return Err(PlannerContractError::new(
                    "compiled_message_flow_set.resource.flow_component_id",
                    "must be identical across selected message groups",
                ));
            }
            flow_component_id = Some(resource.source_program.flow_component_id.clone());
            validate_program_scope(&resource.compiled_program, &expected_scope)?;
        }
        let (facts, mechanics) = merged_catalogs(&self.resources)?;
        if facts != self.facts || mechanics != self.mechanics {
            return Err(PlannerContractError::new(
                "compiled_message_flow_set.catalogs",
                "must be the deterministic merge of compiled resources",
            ));
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
                "compiled_message_flow_set",
                "is not canonical JSON",
            ));
        }
        Ok(set)
    }

    pub fn digest(&self) -> Result<Digest, PlannerContractError> {
        Ok(Digest(Sha256::digest(self.canonical_bytes()?).into()))
    }

    pub fn merge_into(
        &self,
        facts: &mut FactCatalog,
        mechanics: &mut MechanicsCatalog,
    ) -> Result<(), PlannerContractError> {
        self.validate()?;
        facts.validate()?;
        mechanics.validate()?;
        let mut next_facts = facts.clone();
        let mut next_mechanics = mechanics.clone();
        append_catalogs(
            &mut next_facts,
            &mut next_mechanics,
            self.facts.clone(),
            self.mechanics.clone(),
        );
        sort_catalogs(&mut next_facts, &mut next_mechanics);
        next_facts.validate().map_err(|error| {
            PlannerContractError::new("compiled_message_flow_set.merge.facts", error.to_string())
        })?;
        next_mechanics.validate().map_err(|error| {
            PlannerContractError::new(
                "compiled_message_flow_set.merge.mechanics",
                error.to_string(),
            )
        })?;
        *facts = next_facts;
        *mechanics = next_mechanics;
        Ok(())
    }
}

fn validate_contract_flow(
    contracts: &[MessageEventContract],
    flow_component_id: &str,
) -> Result<(), PlannerContractError> {
    for contract in contracts {
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
        let valid = match contract.continuation {
            MessageEventContinuation::EncodedSuccessor => flow_operations.is_empty(),
            MessageEventContinuation::ContractControlled => {
                flow_operations.len() == 1 && flow_operations[0] == flow_component_id
            }
        };
        if !valid {
            return Err(PlannerContractError::new(
                "message_flow_resource_overlay.event_contracts.continuation",
                "does not own exactly the declared flow continuation",
            ));
        }
    }
    Ok(())
}

fn validate_program_scope(
    program: &CompiledMessageFlowProgram,
    expected: &ContextScope,
) -> Result<(), PlannerContractError> {
    if program.aliases.iter().all(|alias| &alias.scope == expected)
        && program
            .mechanics
            .transitions
            .iter()
            .all(|transition| &transition.scope == expected)
        && program
            .mechanics
            .readers
            .iter()
            .all(|reader| &reader.scope == expected)
    {
        Ok(())
    } else {
        Err(PlannerContractError::new(
            "compiled_message_flow_set.resource.scope",
            "does not match the exact selected context",
        ))
    }
}

fn merged_catalogs(
    resources: &[CompiledMessageFlowResource],
) -> Result<(FactCatalog, MechanicsCatalog), PlannerContractError> {
    let mut facts = empty_facts();
    let mut mechanics = empty_mechanics();
    for resource in resources {
        append_catalogs(
            &mut facts,
            &mut mechanics,
            FactCatalog {
                schema: FACT_CATALOG_SCHEMA.into(),
                aliases: resource.compiled_program.aliases.clone(),
                derived_facts: Vec::new(),
            },
            resource.compiled_program.mechanics.clone(),
        );
    }
    sort_catalogs(&mut facts, &mut mechanics);
    facts.validate()?;
    mechanics.validate()?;
    Ok((facts, mechanics))
}

fn append_catalogs(
    facts: &mut FactCatalog,
    mechanics: &mut MechanicsCatalog,
    mut added_facts: FactCatalog,
    mut added_mechanics: MechanicsCatalog,
) {
    facts.aliases.append(&mut added_facts.aliases);
    facts.derived_facts.append(&mut added_facts.derived_facts);
    mechanics
        .transitions
        .append(&mut added_mechanics.transitions);
    mechanics
        .obligations
        .append(&mut added_mechanics.obligations);
    mechanics.writers.append(&mut added_mechanics.writers);
    mechanics.gates.append(&mut added_mechanics.gates);
    mechanics.readers.append(&mut added_mechanics.readers);
    mechanics
        .reconstruction_rules
        .append(&mut added_mechanics.reconstruction_rules);
    mechanics
        .obstructions
        .append(&mut added_mechanics.obstructions);
    mechanics.resolvers.append(&mut added_mechanics.resolvers);
    mechanics.techniques.append(&mut added_mechanics.techniques);
    mechanics
        .microtraces
        .append(&mut added_mechanics.microtraces);
    mechanics.goals.append(&mut added_mechanics.goals);
}

fn sort_catalogs(facts: &mut FactCatalog, mechanics: &mut MechanicsCatalog) {
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

fn empty_facts() -> FactCatalog {
    FactCatalog {
        schema: FACT_CATALOG_SCHEMA.into(),
        aliases: Vec::new(),
        derived_facts: Vec::new(),
    }
}

fn empty_mechanics() -> MechanicsCatalog {
    MechanicsCatalog {
        schema: MECHANICS_CATALOG_SCHEMA.into(),
        transitions: Vec::new(),
        obligations: Vec::new(),
        writers: Vec::new(),
        gates: Vec::new(),
        readers: Vec::new(),
        reconstruction_rules: Vec::new(),
        obstructions: Vec::new(),
        resolvers: Vec::new(),
        techniques: Vec::new(),
        microtraces: Vec::new(),
        goals: Vec::new(),
    }
}

fn require_digest(field: &str, digest: Digest) -> Result<(), PlannerContractError> {
    if digest == Digest::ZERO {
        Err(PlannerContractError::new(field, "must be nonzero"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::ContextSelector;
    use crate::logic::{
        ComparisonOperator, EvidenceKind, EvidenceRecord, PredicateExpression, RuleEvidence,
        ValueReference,
    };
    use crate::message_flow::{MESSAGE_FLOW_PROGRAM_SCHEMA, MessageFlowBindings};
    use crate::orig_extraction::{ExtractedMessageFlow, MessageFlowLabel, MessageFlowNode};
    use crate::state::StateValue;
    use crate::transition::ComponentFieldTarget;

    fn scope() -> ContextScope {
        ContextScope {
            selectors: vec![ContextSelector::Exact {
                context: ExactContext {
                    content_sha256: Digest([1; 32]),
                    runtime_configuration_sha256: Digest([2; 32]),
                },
            }],
        }
    }

    fn evidence() -> RuleEvidence {
        RuleEvidence {
            truth: TruthStatus::Established,
            records: vec![EvidenceRecord {
                id: "evidence.fixture".into(),
                kind: EvidenceKind::SourceAudited,
                source_sha256: Some(Digest([8; 32])),
                note: "Source-audited fixture.".into(),
            }],
        }
    }

    fn resource(group: u8) -> CompiledMessageFlowResource {
        let resource_sha256 = Digest([group + 20; 32]);
        let source_program = MessageFlowProgram {
            schema: MESSAGE_FLOW_PROGRAM_SCHEMA.into(),
            id: format!("message-program.fixture.group-{group}"),
            label: format!("Message group {group}"),
            scope: scope(),
            message_group: group,
            resource_sha256,
            flow_component_id: "flow.active-message".into(),
            extracted: ExtractedMessageFlow {
                header_declared_size: 64,
                resource_size: 64,
                node_count: 1,
                branch_target_count: 0,
                labels: vec![MessageFlowLabel {
                    flow_id: 7,
                    node_index: 0,
                }],
                nodes: vec![MessageFlowNode::Message {
                    index: 0,
                    flags: 0,
                    message_index: 3,
                    next_node_index: u16::MAX,
                    unknown: 0,
                }],
                branch_targets: Vec::new(),
                temporary_flag_accesses: Vec::new(),
                persistent_flag_accesses: Vec::new(),
                switch_accesses: Vec::new(),
            },
            bindings: MessageFlowBindings {
                temporary_flags: None,
                persistent_flags: None,
                switch_stores: Vec::new(),
            },
            event_contracts: Vec::new(),
            cleanup_edges: Vec::new(),
            evidence: RuleEvidence {
                truth: TruthStatus::Established,
                records: vec![EvidenceRecord {
                    id: format!("evidence.message.group-{group}"),
                    kind: EvidenceKind::Extracted,
                    source_sha256: Some(resource_sha256),
                    note: format!("Extracted message group {group}."),
                }],
            },
        };
        let compiled_program = source_program.compile().unwrap();
        CompiledMessageFlowResource {
            message_group: group,
            archive_sha256: Digest([group + 10; 32]),
            resource_sha256,
            source_program,
            compiled_program,
        }
    }

    fn compiled_set() -> CompiledMessageFlowSet {
        let resources = vec![resource(0), resource(3)];
        let (facts, mechanics) = merged_catalogs(&resources).unwrap();
        CompiledMessageFlowSet {
            schema: COMPILED_MESSAGE_FLOW_SET_SCHEMA.into(),
            program_set_sha256: Digest([4; 32]),
            overlay_set_sha256: None,
            exact_context: scope()
                .selectors
                .into_iter()
                .next()
                .map(|selector| {
                    let ContextSelector::Exact { context } = selector else {
                        unreachable!()
                    };
                    context
                })
                .unwrap(),
            locale_bundle: "us".into(),
            resources,
            facts,
            mechanics,
        }
    }

    #[test]
    fn compiled_set_round_trips_and_merges_transactionally() {
        let set = compiled_set();
        set.validate().unwrap();
        assert_eq!(set.mechanics.transitions.len(), 2);
        assert_eq!(
            CompiledMessageFlowSet::decode_canonical(&set.canonical_bytes().unwrap()).unwrap(),
            set
        );
        let mut facts = empty_facts();
        let mut mechanics = empty_mechanics();
        set.merge_into(&mut facts, &mut mechanics).unwrap();
        assert_eq!(mechanics.transitions.len(), 2);
        let facts_before = facts.clone();
        let mechanics_before = mechanics.clone();
        assert!(set.merge_into(&mut facts, &mut mechanics).is_err());
        assert_eq!(facts, facts_before);
        assert_eq!(mechanics, mechanics_before);
    }

    #[test]
    fn overlay_contracts_are_digest_pinned_and_cleanup_is_conditional() {
        let overlay = MessageFlowResourceOverlaySet {
            schema: MESSAGE_FLOW_RESOURCE_OVERLAY_SET_SCHEMA.into(),
            id: "fixture-overlays".into(),
            import_profile_sha256: Digest([5; 32]),
            resources: vec![MessageFlowResourceOverlay {
                message_group: 3,
                resource_sha256: Digest([23; 32]),
                event_contracts: vec![MessageEventContract {
                    node_index: 4,
                    confirmed_operations: vec![StateOperation::Write {
                        target: ComponentFieldTarget {
                            component_id: "inventory.active".into(),
                            field: "last_granted_item".into(),
                        },
                        value: StateValue::Unsigned(7),
                    }],
                    continuation: MessageEventContinuation::EncodedSuccessor,
                    evidence: evidence(),
                }],
                cleanup_edges: vec![MessageCleanupEdge {
                    transition_id: "transition.cleanup.message".into(),
                    label: "Message cleanup".into(),
                    approach_id: "approach.cleanup.message".into(),
                    activation: PredicateExpression::Compare {
                        left: ValueReference::RuntimeLanguage,
                        operator: ComparisonOperator::Equal,
                        right: ValueReference::Literal {
                            value: StateValue::Text("en".into()),
                        },
                    },
                    packed_backing_coordinates: vec![0x0004],
                    evidence: evidence(),
                }],
            }],
        };
        overlay.validate().unwrap();
        assert_eq!(
            MessageFlowResourceOverlaySet::decode_canonical(&overlay.canonical_bytes().unwrap())
                .unwrap(),
            overlay
        );
        let mut unconditional = overlay.clone();
        unconditional.resources[0].cleanup_edges[0].activation = PredicateExpression::True;
        assert_eq!(
            unconditional.validate().unwrap_err().field(),
            "message_flow_resource_overlay.cleanup.activation"
        );
        let mut unknown_cleanup = overlay;
        unknown_cleanup.resources[0].cleanup_edges[0].evidence.truth = TruthStatus::Unknown;
        assert_eq!(
            unknown_cleanup.validate().unwrap_err().field(),
            "message_flow_resource_overlay.cleanup.evidence"
        );
    }

    #[test]
    fn tampered_merged_catalog_is_rejected() {
        let mut set = compiled_set();
        set.mechanics.transitions.pop();
        assert_eq!(
            set.validate().unwrap_err().field(),
            "compiled_message_flow_set.catalogs"
        );
    }
}
