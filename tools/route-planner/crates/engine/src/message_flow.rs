//! Compilation of extracted retail message-flow graphs into planner mechanics.
//!
//! The extractor reports what the selected language resource encodes. This
//! module adds the build-specific backing layouts needed to turn decidable
//! generic handlers into ordinary guards and state operations. Unsupported
//! handlers remain visible as unknown requirements; unknown node shapes never
//! acquire an invented successor.

use crate::artifact::Digest;
use crate::identity::{ContextSelector, RuntimeConfiguration};
use crate::logic::{
    ComparisonOperator, ContextScope, EvidenceKind, EvidenceRecord, FactCatalog, FriendlyAlias,
    PredicateExpression, RawFactBinding, RuleEvidence, TruthStatus, ValueReference,
};
use crate::orig_discovery::{ExtractedOrigBundle, ExtractedOrigMessageArchive};
use crate::orig_extraction::{
    ExtractedMessageFlow, MessageFlowNode, MessageFlowPersistentFlagAccess,
    MessageFlowPersistentFlagOperation, MessageFlowSwitchAccess, MessageFlowSwitchOperation,
    MessageFlowSwitchStore, MessageFlowTemporaryFlagAccess, MessageFlowTemporaryFlagOperation,
};
use crate::state::{
    ComponentBindingReference, ComponentKind, StateValue, validate_binding_reference,
    validate_component_kind,
};
use crate::transition::{
    ActivationContract, CandidateTransition, MECHANICS_CATALOG_SCHEMA, MechanicsCatalog,
    ReaderRule, StateOperation, TransitionKind, UnknownRequirement,
};
use crate::{PlannerContractError, canonical_json, validate_label, validate_stable_id};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const MESSAGE_FLOW_PROGRAM_SCHEMA: &str = "dusklight.route-planner.message-flow-program/v2";
pub const COMPILED_MESSAGE_FLOW_PROGRAM_SCHEMA: &str =
    "dusklight.route-planner.compiled-message-flow-program/v2";
pub const MESSAGE_FLOW_IMPORT_PROFILE_SCHEMA: &str =
    "dusklight.route-planner.message-flow-import-profile/v2";
pub const MESSAGE_FLOW_PROGRAM_SET_SCHEMA: &str =
    "dusklight.route-planner.message-flow-program-set/v2";
const MAX_MESSAGE_FLOW_NODES: usize = 65_535;
const MAX_EVENT_CONTRACTS: usize = 16_384;
const MAX_CLEANUP_EDGES: usize = 256;
const BUNDLED_GZ2E01_ENGLISH_IMPORT_PROFILE: &[u8] =
    include_bytes!("../data/message-import-profiles/gz2e01-en.json");

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MessageFlowProgram {
    pub schema: String,
    pub id: String,
    pub label: String,
    pub scope: ContextScope,
    pub message_group: u8,
    pub resource_sha256: Digest,
    pub flow_component_id: String,
    pub extracted: ExtractedMessageFlow,
    pub bindings: MessageFlowBindings,
    pub event_contracts: Vec<MessageEventContract>,
    pub cleanup_edges: Vec<MessageCleanupEdge>,
    pub evidence: RuleEvidence,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MessageFlowBindings {
    pub temporary_flags: Option<MessageRawStoreBinding>,
    pub persistent_flags: Option<MessageRawStoreBinding>,
    pub switch_stores: Vec<MessageSwitchStoreBinding>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MessageRawStoreBinding {
    pub component_kind: ComponentKind,
    pub binding: ComponentBindingReference,
}

/// Maps a logical switch index into a byte-backed component. Retail switch
/// arrays are commonly arrays of big-endian words, so byte order within each
/// word is explicit instead of being hidden in a hard-coded offset formula.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MessageSwitchStoreBinding {
    pub store: MessageFlowSwitchStore,
    pub component_kind: ComponentKind,
    pub binding: ComponentBindingReference,
    pub byte_offset_base: u32,
    pub word_bytes: u8,
    pub reverse_bytes_within_word: bool,
    pub switch_count: u16,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageEventContinuation {
    /// Apply the contract and then follow the successor encoded in the event
    /// node.
    EncodedSuccessor,
    /// The contract operations own the control-flow update. This is required
    /// for handlers such as explicit flow jumps.
    ContractControlled,
}

/// Source-audited semantics for a node whose generic event handler is not one
/// of the flag/switch handlers decoded directly by the extractor.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MessageEventContract {
    pub node_index: u16,
    pub confirmed_operations: Vec<StateOperation>,
    pub continuation: MessageEventContinuation,
    pub evidence: RuleEvidence,
}

/// A separately evidenced invocation of a temporary-message-bit cleanup. The
/// activation predicate distinguishes central event completion from Ooccoo or
/// any future cleanup caller; cleanup is never inferred from room/load alone.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MessageCleanupEdge {
    pub transition_id: String,
    pub label: String,
    pub approach_id: String,
    pub activation: PredicateExpression,
    pub packed_backing_coordinates: Vec<u16>,
    pub evidence: RuleEvidence,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledMessageFlowProgram {
    pub schema: String,
    pub program_sha256: Digest,
    pub flow_component_id: String,
    pub terminal_node_id: String,
    pub entry_points: Vec<CompiledMessageFlowEntry>,
    pub unresolved_nodes: Vec<UnresolvedMessageFlowNode>,
    pub aliases: Vec<FriendlyAlias>,
    pub mechanics: MechanicsCatalog,
}

/// Exact-content policy needed to bind immutable BMG graphs to mutable planner
/// stores. Locale selection and backing layout are kept out of the extractor:
/// neither can be inferred from a resource filename alone.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MessageFlowImportProfile {
    pub schema: String,
    pub id: String,
    pub content_sha256: Digest,
    pub language_bundles: BTreeMap<String, String>,
    pub flow_component_id: String,
    pub bindings: MessageFlowBindings,
    pub evidence: RuleEvidence,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MessageFlowProgramSet {
    pub schema: String,
    pub profile_sha256: Digest,
    pub bundle_sha256: Digest,
    pub exact_context: crate::identity::ExactContext,
    pub locale_bundle: String,
    pub programs: Vec<MessageFlowProgram>,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledMessageFlowEntry {
    pub flow_id: u16,
    pub node_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UnresolvedMessageFlowNode {
    pub node_index: u16,
    pub reason: String,
}

impl MessageFlowImportProfile {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != MESSAGE_FLOW_IMPORT_PROFILE_SCHEMA {
            return Err(PlannerContractError::new(
                "message_flow_import_profile.schema",
                "is unsupported",
            ));
        }
        validate_stable_id("message_flow_import_profile.id", &self.id)?;
        if self.content_sha256 == Digest::ZERO {
            return Err(PlannerContractError::new(
                "message_flow_import_profile.content_sha256",
                "must be nonzero",
            ));
        }
        if self.language_bundles.is_empty() || self.language_bundles.len() > 64 {
            return Err(PlannerContractError::new(
                "message_flow_import_profile.language_bundles",
                "must contain between 1 and 64 language selections",
            ));
        }
        let mut selected_bundles = BTreeSet::new();
        for (language, locale_bundle) in &self.language_bundles {
            validate_language_token(
                "message_flow_import_profile.language_bundles.language",
                language,
                true,
            )?;
            validate_language_token(
                "message_flow_import_profile.language_bundles.locale_bundle",
                locale_bundle,
                false,
            )?;
            selected_bundles.insert(locale_bundle.as_str());
        }
        if selected_bundles.is_empty() {
            return Err(PlannerContractError::new(
                "message_flow_import_profile.language_bundles",
                "does not select a locale bundle",
            ));
        }
        validate_stable_id(
            "message_flow_import_profile.flow_component_id",
            &self.flow_component_id,
        )?;
        self.evidence
            .validate("message_flow_import_profile.evidence")?;
        self.bindings.validate()?;
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PlannerContractError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, PlannerContractError> {
        let profile: Self = serde_json::from_slice(bytes)?;
        profile.validate()?;
        if profile.canonical_bytes()? != bytes {
            return Err(PlannerContractError::new(
                "message_flow_import_profile",
                "is not canonical JSON",
            ));
        }
        Ok(profile)
    }

    pub fn digest(&self) -> Result<Digest, PlannerContractError> {
        Ok(Digest(Sha256::digest(self.canonical_bytes()?).into()))
    }
}

pub fn bundled_gz2e01_english_message_flow_profile()
-> Result<MessageFlowImportProfile, PlannerContractError> {
    MessageFlowImportProfile::decode_canonical(BUNDLED_GZ2E01_ENGLISH_IMPORT_PROFILE)
}

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

fn construct_selected_message_flow_programs(
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
                    ..
                } => {
                    let contract = contracts.get(index).copied();
                    let (mut operations, unknowns, fully_decoded) = self.compile_generic_event(
                        *index,
                        *event_index,
                        *parameter_0,
                        *parameter_1,
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
                    let contract_controls = contract.is_some_and(|value| {
                        value.continuation == MessageEventContinuation::ContractControlled
                    });
                    transitions.push(self.direct_transition_with_continuation(
                        &token,
                        *index,
                        "execute-event",
                        format!("Execute message event {event_index} at node {index}"),
                        operations,
                        unknowns,
                        *next_target_index,
                        &terminal_node_id,
                        &evidence,
                        !contract_controls,
                    )?);
                }
                MessageFlowNode::Branch {
                    index,
                    next_target_index,
                    query_handler_index,
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
        token: &str,
    ) -> Result<(Vec<StateOperation>, Vec<UnknownRequirement>, bool), PlannerContractError> {
        let mut operations = Vec::new();
        let mut unknowns = Vec::new();
        let fully_decoded = match event_index {
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
            _ => false,
        };
        operations.sort_by_key(|entry| entry.0);
        Ok((
            operations.into_iter().map(|entry| entry.1).collect(),
            unknowns,
            fully_decoded,
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
        Ok(aliases.into_values().collect())
    }

    fn alias_id_with_token(&self, token: &str, kind: &str, label_index: u16) -> String {
        format!("fact.message-flow.{token}.{kind}-label-{label_index}")
    }
}

impl MessageFlowBindings {
    pub(crate) fn validate(&self) -> Result<(), PlannerContractError> {
        if let Some(binding) = &self.temporary_flags {
            binding.validate("message_flow_program.bindings.temporary_flags")?;
        }
        if let Some(binding) = &self.persistent_flags {
            binding.validate("message_flow_program.bindings.persistent_flags")?;
        }
        let mut stores = BTreeSet::new();
        for binding in &self.switch_stores {
            binding.validate()?;
            if !stores.insert(switch_store_key(binding.store)) {
                return Err(PlannerContractError::new(
                    "message_flow_program.bindings.switch_stores",
                    "must contain at most one binding for each switch store",
                ));
            }
        }
        let mut prior = None;
        for binding in &self.switch_stores {
            let key = switch_store_key(binding.store);
            if prior.is_some_and(|value| value >= key) {
                return Err(PlannerContractError::new(
                    "message_flow_program.bindings.switch_stores",
                    "must be sorted by switch store",
                ));
            }
            prior = Some(key);
        }
        Ok(())
    }

    fn validate_for(&self, extracted: &ExtractedMessageFlow) -> Result<(), PlannerContractError> {
        self.validate()?;
        for access in &extracted.switch_accesses {
            if let Some(binding) = self.switch_store(access.store) {
                binding.raw_location(access.switch_index)?;
            }
        }
        Ok(())
    }

    fn switch_store(&self, store: MessageFlowSwitchStore) -> Option<&MessageSwitchStoreBinding> {
        self.switch_stores
            .iter()
            .find(|binding| binding.store == store)
    }
}

fn unknown_flag_backing(
    token: &str,
    node_index: u16,
    kind: &str,
    parameter_ordinal: u8,
    label_index: u16,
    evidence: &RuleEvidence,
) -> UnknownRequirement {
    unknown_requirement(
        token,
        node_index,
        &format!("{kind}-parameter-{parameter_ordinal}-backing"),
        format!(
            "Message {kind} flag label {label_index} at node {node_index} has no audited backing binding"
        ),
        evidence,
    )
}

impl MessageRawStoreBinding {
    fn validate(&self, field: &str) -> Result<(), PlannerContractError> {
        validate_component_kind(&self.component_kind)?;
        validate_binding_reference(&self.binding)?;
        StateOperation::WriteBoundRaw {
            component_kind: self.component_kind.clone(),
            binding: self.binding.clone(),
            byte_offset: 0,
            mask: vec![1],
            value: vec![0],
        }
        .validate()
        .map_err(|error| PlannerContractError::new(field, error.detail()))
    }
}

impl MessageSwitchStoreBinding {
    fn validate(&self) -> Result<(), PlannerContractError> {
        MessageRawStoreBinding {
            component_kind: self.component_kind.clone(),
            binding: self.binding.clone(),
        }
        .validate("message_flow_program.bindings.switch_store")?;
        if !(1..=8).contains(&self.word_bytes) || self.switch_count == 0 {
            return Err(PlannerContractError::new(
                "message_flow_program.bindings.switch_store.layout",
                "word_bytes must be 1..=8 and switch_count must be nonzero",
            ));
        }
        self.raw_location(self.switch_count - 1)?;
        Ok(())
    }

    fn raw_location(&self, switch_index: u16) -> Result<(u32, u8), PlannerContractError> {
        if switch_index >= self.switch_count {
            return Err(PlannerContractError::new(
                "message_flow_program.switch_index",
                format!(
                    "switch {switch_index} exceeds {:?} store capacity {}",
                    self.store, self.switch_count
                ),
            ));
        }
        let logical_byte = u32::from(switch_index / 8);
        let word_bytes = u32::from(self.word_bytes);
        let word = logical_byte / word_bytes;
        let byte_in_word = logical_byte % word_bytes;
        let stored_byte = if self.reverse_bytes_within_word {
            word_bytes - 1 - byte_in_word
        } else {
            byte_in_word
        };
        let byte_offset = self
            .byte_offset_base
            .checked_add(word.checked_mul(word_bytes).ok_or_else(|| {
                PlannerContractError::new(
                    "message_flow_program.switch_layout",
                    "word offset overflows",
                )
            })?)
            .and_then(|offset| offset.checked_add(stored_byte))
            .ok_or_else(|| {
                PlannerContractError::new(
                    "message_flow_program.switch_layout",
                    "byte offset overflows",
                )
            })?;
        Ok((byte_offset, 1_u8 << (switch_index % 8)))
    }
}

impl CompiledMessageFlowProgram {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != COMPILED_MESSAGE_FLOW_PROGRAM_SCHEMA
            || self.program_sha256 == Digest::ZERO
        {
            return Err(PlannerContractError::new(
                "compiled_message_flow_program",
                "has an unsupported schema or zero program digest",
            ));
        }
        validate_stable_id(
            "compiled_message_flow_program.flow_component_id",
            &self.flow_component_id,
        )?;
        validate_stable_id(
            "compiled_message_flow_program.terminal_node_id",
            &self.terminal_node_id,
        )?;
        let mut prior_flow = None;
        for entry in &self.entry_points {
            validate_stable_id(
                "compiled_message_flow_program.entry.node_id",
                &entry.node_id,
            )?;
            if prior_flow.is_some_and(|flow_id| flow_id >= entry.flow_id) {
                return Err(PlannerContractError::new(
                    "compiled_message_flow_program.entry_points",
                    "must be unique and sorted by flow ID",
                ));
            }
            prior_flow = Some(entry.flow_id);
        }
        let mut prior_node = None;
        for unresolved in &self.unresolved_nodes {
            validate_label(
                "compiled_message_flow_program.unresolved.reason",
                &unresolved.reason,
            )?;
            if prior_node.is_some_and(|index| index >= unresolved.node_index) {
                return Err(PlannerContractError::new(
                    "compiled_message_flow_program.unresolved_nodes",
                    "must be unique and sorted by node index",
                ));
            }
            prior_node = Some(unresolved.node_index);
        }
        FactCatalog {
            schema: crate::logic::FACT_CATALOG_SCHEMA.into(),
            aliases: self.aliases.clone(),
            derived_facts: Vec::new(),
        }
        .validate()?;
        self.mechanics.validate()
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PlannerContractError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, PlannerContractError> {
        let artifact: Self = serde_json::from_slice(bytes)?;
        artifact.validate()?;
        if artifact.canonical_bytes()? != bytes {
            return Err(PlannerContractError::new(
                "compiled_message_flow_program",
                "is not canonical JSON",
            ));
        }
        Ok(artifact)
    }
}

#[derive(Clone)]
struct CompiledBranchAccess {
    reference: ValueReference,
    mask: u8,
    alias_id: Option<String>,
}

fn validate_extracted(extracted: &ExtractedMessageFlow) -> Result<(), PlannerContractError> {
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
            }
            | MessageFlowNode::Event {
                next_target_index: next_node_index,
                ..
            } => validate_node_target(*next_node_index, extracted.node_count)?,
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

fn validate_accesses(extracted: &ExtractedMessageFlow) -> Result<(), PlannerContractError> {
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

fn validate_temporary_access(
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

fn validate_persistent_access(
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

fn validate_switch_access(
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

fn event_parameter_matches(
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

fn require_valid_access(valid: bool, field: &str) -> Result<(), PlannerContractError> {
    if valid {
        Ok(())
    } else {
        Err(PlannerContractError::new(
            field,
            "does not match the referenced node handler and parameters",
        ))
    }
}

fn validate_access_name(
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

fn switch_store_from_selector(selector: u16) -> Option<MessageFlowSwitchStore> {
    Some(match selector {
        0 => MessageFlowSwitchStore::LoadedStageMemory,
        1 => MessageFlowSwitchStore::Dungeon,
        2 => MessageFlowSwitchStore::Zone,
        3 => MessageFlowSwitchStore::OneZone,
        _ => return None,
    })
}

fn switch_store_from_query(handler: u16) -> Option<MessageFlowSwitchStore> {
    Some(match handler {
        13 => MessageFlowSwitchStore::LoadedStageMemory,
        15 => MessageFlowSwitchStore::Dungeon,
        17 => MessageFlowSwitchStore::Zone,
        19 => MessageFlowSwitchStore::OneZone,
        _ => return None,
    })
}

fn validate_event_contracts(
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
        let Some(MessageFlowNode::Event { event_index, .. }) =
            extracted.nodes.get(usize::from(contract.node_index))
        else {
            return Err(PlannerContractError::new(
                "message_flow_program.event_contracts.node_index",
                "must reference an event node",
            ));
        };
        if matches!(*event_index, 0 | 1 | 10 | 11 | 14 | 15) {
            return Err(PlannerContractError::new(
                "message_flow_program.event_contracts.node_index",
                "must not replace a generic flag/switch handler decoded by the extractor",
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

fn validate_cleanup_edges(
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

fn insert_alias(
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

fn raw_write(store: &MessageRawStoreBinding, packed: u16, set: bool) -> StateOperation {
    let (byte_offset, mask) = unpack_coordinate(packed);
    StateOperation::WriteBoundRaw {
        component_kind: store.component_kind.clone(),
        binding: store.binding.clone(),
        byte_offset,
        mask: vec![mask],
        value: vec![if set { mask } else { 0 }],
    }
}

fn compiled_raw_access(
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

fn raw_branch_guard(access: &CompiledBranchAccess, outcome: u8) -> PredicateExpression {
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

fn flow_node_guard(flow_component_id: &str, expected: &str) -> PredicateExpression {
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

fn all_guards(mut guards: Vec<PredicateExpression>) -> PredicateExpression {
    if guards.len() == 1 {
        guards.pop().unwrap()
    } else {
        PredicateExpression::All { terms: guards }
    }
}

fn unpack_coordinate(packed: u16) -> (u32, u8) {
    (u32::from(packed >> 8), packed as u8)
}

fn message_node_index(node: &MessageFlowNode) -> u16 {
    match node {
        MessageFlowNode::Message { index, .. }
        | MessageFlowNode::Branch { index, .. }
        | MessageFlowNode::Event { index, .. }
        | MessageFlowNode::Unknown { index, .. } => *index,
    }
}

fn validate_node_target(target: u16, node_count: u16) -> Result<(), PlannerContractError> {
    if target != u16::MAX && target >= node_count {
        return Err(PlannerContractError::new(
            "message_flow_program.extracted.target",
            format!("node target {target} exceeds node count {node_count}"),
        ));
    }
    Ok(())
}

fn node_id(token: &str, index: u16) -> String {
    format!("message-node.{token}.{index}")
}

fn target_node_id(token: &str, target: u16, terminal_node_id: &str) -> String {
    if target == u16::MAX {
        terminal_node_id.into()
    } else {
        node_id(token, target)
    }
}

fn validate_language_token(
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

fn short_token(digest: Digest) -> String {
    digest.as_bytes()[..12]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn switch_store_key(store: MessageFlowSwitchStore) -> u8 {
    match store {
        MessageFlowSwitchStore::LoadedStageMemory => 0,
        MessageFlowSwitchStore::Dungeon => 1,
        MessageFlowSwitchStore::Zone => 2,
        MessageFlowSwitchStore::OneZone => 3,
    }
}

fn unknown_requirement(
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

fn unknown_flag_coordinate(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{ContextSelector, ExactContext, RUNTIME_CONFIGURATION_SCHEMA};
    use crate::logic::{EvidenceKind, EvidenceRecord};
    use crate::orig_extraction::{MessageFlowLabel, MessageFlowSwitchStore};
    use crate::state::ComponentBinding;
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

    fn evidence(source: Digest) -> RuleEvidence {
        RuleEvidence {
            truth: TruthStatus::Established,
            records: vec![EvidenceRecord {
                id: "evidence.message-resource".into(),
                kind: EvidenceKind::Extracted,
                source_sha256: Some(source),
                note: "Extracted from the selected language resource.".into(),
            }],
        }
    }

    fn program() -> MessageFlowProgram {
        let source = Digest([3; 32]);
        MessageFlowProgram {
            schema: MESSAGE_FLOW_PROGRAM_SCHEMA.into(),
            id: "message-group-3-fixture".into(),
            label: "Message group 3 fixture".into(),
            scope: scope(),
            message_group: 3,
            resource_sha256: source,
            flow_component_id: "flow.active-message".into(),
            extracted: ExtractedMessageFlow {
                header_declared_size: 100,
                resource_size: 100,
                node_count: 6,
                branch_target_count: 2,
                labels: vec![MessageFlowLabel {
                    flow_id: 42,
                    node_index: 0,
                }],
                nodes: vec![
                    MessageFlowNode::Event {
                        index: 0,
                        event_index: 10,
                        next_target_index: 1,
                        parameter_0: 51,
                        parameter_1: 0,
                        raw_parameter_u32: 51 << 16,
                        raw_parameters: [0, 51, 0, 0],
                    },
                    MessageFlowNode::Branch {
                        index: 1,
                        flags: 0,
                        raw_query_index: 10,
                        query_handler_index: Some(11),
                        parameter: 11,
                        next_target_index: 0,
                    },
                    MessageFlowNode::Event {
                        index: 2,
                        event_index: 0,
                        next_target_index: u16::MAX,
                        parameter_0: 62,
                        parameter_1: 0,
                        raw_parameter_u32: 62 << 16,
                        raw_parameters: [0, 62, 0, 0],
                    },
                    MessageFlowNode::Event {
                        index: 3,
                        event_index: 14,
                        next_target_index: u16::MAX,
                        parameter_0: 0,
                        parameter_1: 12,
                        raw_parameter_u32: 12,
                        raw_parameters: [0, 0, 0, 12],
                    },
                    MessageFlowNode::Event {
                        index: 4,
                        event_index: 17,
                        next_target_index: u16::MAX,
                        parameter_0: 7,
                        parameter_1: 0,
                        raw_parameter_u32: 7 << 16,
                        raw_parameters: [0, 7, 0, 0],
                    },
                    MessageFlowNode::Unknown {
                        index: 5,
                        node_type: 9,
                        raw: [9; 8],
                    },
                ],
                branch_targets: vec![2, 3],
                temporary_flag_accesses: vec![
                    MessageFlowTemporaryFlagAccess {
                        node_index: 0,
                        operation: MessageFlowTemporaryFlagOperation::Set,
                        parameter_ordinal: 0,
                        label_index: 51,
                        packed_backing_coordinate: Some(0x0508),
                        friendly_name: Some("message_flow_control_f".into()),
                    },
                    MessageFlowTemporaryFlagAccess {
                        node_index: 1,
                        operation: MessageFlowTemporaryFlagOperation::BranchTrueWhenClear,
                        parameter_ordinal: 0,
                        label_index: 11,
                        packed_backing_coordinate: Some(0x0004),
                        friendly_name: Some("message_flow_control_a".into()),
                    },
                ],
                persistent_flag_accesses: vec![MessageFlowPersistentFlagAccess {
                    node_index: 2,
                    operation: MessageFlowPersistentFlagOperation::Set,
                    parameter_ordinal: 0,
                    label_index: 62,
                    packed_backing_coordinate: Some(0x0704),
                    friendly_name: Some("won_gor_coron_match".into()),
                }],
                switch_accesses: vec![MessageFlowSwitchAccess {
                    node_index: 3,
                    operation: MessageFlowSwitchOperation::Set,
                    store: MessageFlowSwitchStore::LoadedStageMemory,
                    switch_index: 12,
                }],
            },
            bindings: MessageFlowBindings {
                temporary_flags: Some(MessageRawStoreBinding {
                    component_kind: ComponentKind::TemporaryFlags,
                    binding: ComponentBindingReference::Exact {
                        binding: ComponentBinding::Session {
                            session_id: "session.main".into(),
                        },
                    },
                }),
                persistent_flags: Some(MessageRawStoreBinding {
                    component_kind: ComponentKind::PersistentSave,
                    binding: ComponentBindingReference::ActiveRuntimeFile,
                }),
                switch_stores: vec![MessageSwitchStoreBinding {
                    store: MessageFlowSwitchStore::LoadedStageMemory,
                    component_kind: ComponentKind::StageMemory,
                    binding: ComponentBindingReference::CurrentStage,
                    byte_offset_base: 8,
                    word_bytes: 4,
                    reverse_bytes_within_word: true,
                    switch_count: 128,
                }],
            },
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
                evidence: evidence(source),
            }],
            cleanup_edges: vec![MessageCleanupEdge {
                transition_id: "transition.cleanup.central-message".into(),
                label: "Central message completion cleanup".into(),
                approach_id: "approach.cleanup.central-message".into(),
                activation: PredicateExpression::Compare {
                    left: ValueReference::FlowNode {
                        flow_component_id: "flow.active-message".into(),
                    },
                    operator: ComparisonOperator::Equal,
                    right: ValueReference::Literal {
                        value: StateValue::Text("message-cleanup-ready".into()),
                    },
                },
                packed_backing_coordinates: vec![0x0001, 0x0002, 0x0004],
                evidence: evidence(source),
            }],
            evidence: evidence(source),
        }
    }

    fn import_profile() -> MessageFlowImportProfile {
        let template = program();
        MessageFlowImportProfile {
            schema: MESSAGE_FLOW_IMPORT_PROFILE_SCHEMA.into(),
            id: "gcn-us-fixture".into(),
            content_sha256: Digest([1; 32]),
            language_bundles: BTreeMap::from([("en".into(), "us".into())]),
            flow_component_id: template.flow_component_id,
            bindings: template.bindings,
            evidence: evidence(Digest([9; 32])),
        }
    }

    fn runtime_configuration() -> RuntimeConfiguration {
        RuntimeConfiguration {
            schema: RUNTIME_CONFIGURATION_SCHEMA.into(),
            content_sha256: Digest([1; 32]),
            language: "en".into(),
            settings: BTreeMap::new(),
        }
    }

    fn extracted_archive(group: u16, source: u8) -> ExtractedOrigMessageArchive {
        ExtractedOrigMessageArchive {
            relative_path: format!("files/res/Msgus/bmgres{group}.arc"),
            archive_sha256: Digest([source.wrapping_add(1); 32]),
            locale_bundle: "us".into(),
            message_group: group,
            resource_name: format!("zel_{group:02}.bmg"),
            resource_sha256: Digest([source; 32]),
            flow: program().extracted,
        }
    }

    #[test]
    fn compiles_known_handlers_branches_cleanup_and_event_handoffs() {
        let program = program();
        program.validate().unwrap();
        let compiled = program.compile().unwrap();
        assert_eq!(compiled.entry_points[0].flow_id, 42);
        assert_eq!(compiled.unresolved_nodes.len(), 1);
        assert_eq!(compiled.mechanics.transitions.len(), 7);
        assert_eq!(compiled.mechanics.readers.len(), 2);
        assert_eq!(compiled.aliases.len(), 3);

        let set_temp = compiled
            .mechanics
            .transitions
            .iter()
            .find(|transition| transition.label.contains("event 10"))
            .unwrap();
        assert!(matches!(
            &set_temp.activation.effects[0],
            StateOperation::WriteBoundRaw {
                component_kind: ComponentKind::TemporaryFlags,
                byte_offset: 5,
                mask,
                value,
                ..
            } if mask == &[0x08] && value == &[0x08]
        ));

        let branch_clear = compiled
            .mechanics
            .transitions
            .iter()
            .find(|transition| transition.label == "Take message branch 1 at node 1")
            .unwrap();
        assert!(matches!(
            &branch_clear.activation.hard_guards,
            PredicateExpression::All { terms }
                if matches!(
                    &terms[1],
                    PredicateExpression::Compare {
                        right: ValueReference::Literal {
                            value: StateValue::Unsigned(0)
                        },
                        ..
                    }
                )
        ));
        assert!(matches!(
            &branch_clear.activation.effects[0],
            StateOperation::BranchFlow {
                destination_node_id,
                ..
            } if destination_node_id.ends_with(".3")
        ));

        let switch = compiled
            .mechanics
            .transitions
            .iter()
            .find(|transition| transition.label.contains("event 14"))
            .unwrap();
        assert!(matches!(
            &switch.activation.effects[0],
            StateOperation::WriteBoundRaw {
                component_kind: ComponentKind::StageMemory,
                binding: ComponentBindingReference::CurrentStage,
                byte_offset: 10,
                mask,
                value,
            } if mask == &[0x10] && value == &[0x10]
        ));

        let cleanup = compiled
            .mechanics
            .transitions
            .iter()
            .find(|transition| transition.id == "transition.cleanup.central-message")
            .unwrap();
        assert_eq!(cleanup.activation.effects.len(), 3);
        assert!(cleanup.activation.effects.iter().all(|operation| matches!(
            operation,
            StateOperation::WriteBoundRaw { value, .. } if value == &[0]
        )));
        let event_handoff = compiled
            .mechanics
            .transitions
            .iter()
            .find(|transition| transition.label.contains("event 17"))
            .unwrap();
        assert!(event_handoff.activation.unknown_requirements.is_empty());
        assert!(matches!(
            event_handoff.activation.effects.as_slice(),
            [
                StateOperation::Write { .. },
                StateOperation::AdvanceFlow { .. }
            ]
        ));
        assert_eq!(
            MessageFlowProgram::decode_canonical(&program.canonical_bytes().unwrap()).unwrap(),
            program
        );
        assert_eq!(
            CompiledMessageFlowProgram::decode_canonical(&compiled.canonical_bytes().unwrap())
                .unwrap(),
            compiled
        );
    }

    #[test]
    fn unsupported_handlers_stay_unknown_and_unknown_nodes_have_no_edge() {
        let mut program = program();
        program.event_contracts.clear();
        let compiled = program.compile().unwrap();
        let unsupported = compiled
            .mechanics
            .transitions
            .iter()
            .find(|transition| transition.label.contains("event 17"))
            .unwrap();
        assert_eq!(unsupported.activation.unknown_requirements.len(), 1);
        assert!(
            !compiled
                .mechanics
                .transitions
                .iter()
                .any(|transition| transition.label.contains("node 5"))
        );
        assert_eq!(compiled.unresolved_nodes[0].node_index, 5);
    }

    #[test]
    fn constructs_every_selected_resource_with_exact_scope_and_profile_bindings() {
        let profile = import_profile();
        let runtime = runtime_configuration();
        let group_three = extracted_archive(3, 3);
        let group_zero = extracted_archive(0, 4);
        let programs = construct_selected_message_flow_programs(
            profile.content_sha256,
            &runtime,
            &profile,
            "us",
            &[&group_three, &group_zero],
        )
        .unwrap();

        assert_eq!(
            programs
                .iter()
                .map(|program| program.message_group)
                .collect::<Vec<_>>(),
            vec![0, 3]
        );
        assert!(programs.iter().all(|program| {
            program.bindings == profile.bindings
                && program.event_contracts.is_empty()
                && program.cleanup_edges.is_empty()
                && program.scope.selectors
                    == vec![ContextSelector::Exact {
                        context: runtime.exact_context().unwrap(),
                    }]
                && program.evidence.records.iter().any(|record| {
                    record.kind == EvidenceKind::Extracted
                        && record.source_sha256 == Some(program.resource_sha256)
                })
        }));
        assert!(programs.iter().all(|program| program.compile().is_ok()));
        let set = MessageFlowProgramSet {
            schema: MESSAGE_FLOW_PROGRAM_SET_SCHEMA.into(),
            profile_sha256: profile.digest().unwrap(),
            bundle_sha256: Digest([8; 32]),
            exact_context: runtime.exact_context().unwrap(),
            locale_bundle: "us".into(),
            programs,
        };
        assert_eq!(
            MessageFlowProgramSet::decode_canonical(&set.canonical_bytes().unwrap()).unwrap(),
            set
        );
        assert_eq!(
            MessageFlowImportProfile::decode_canonical(&profile.canonical_bytes().unwrap())
                .unwrap(),
            profile
        );

        let mut long_id_profile = profile.clone();
        long_id_profile.id = "a".repeat(128);
        assert!(
            construct_selected_message_flow_programs(
                long_id_profile.content_sha256,
                &runtime,
                &long_id_profile,
                "us",
                &[&group_zero],
            )
            .is_ok()
        );
    }

    #[test]
    fn bundled_gz2e01_profile_maps_only_source_audited_backings() {
        let profile = bundled_gz2e01_english_message_flow_profile().unwrap();
        assert_eq!(profile.id, "gcn-us-1.0-gz2e01-en");
        assert_eq!(profile.flow_component_id, "message-session");
        assert_eq!(
            profile.language_bundles.get("en").map(String::as_str),
            Some("us")
        );
        assert!(profile.bindings.temporary_flags.is_some());
        assert!(profile.bindings.persistent_flags.is_none());
        assert_eq!(profile.bindings.switch_stores.len(), 1);
        assert_eq!(
            profile.bindings.switch_stores[0].store,
            MessageFlowSwitchStore::LoadedStageMemory
        );
        assert_eq!(profile.bindings.switch_stores[0].byte_offset_base, 0x08);
        assert_eq!(profile.bindings.switch_stores[0].switch_count, 128);
        assert!(
            profile
                .evidence
                .records
                .iter()
                .all(|record| record.kind == EvidenceKind::SourceAudited)
        );
    }

    #[test]
    fn selected_resource_construction_rejects_ambiguity_and_keeps_unmapped_stores_unknown() {
        let profile = import_profile();
        let runtime = runtime_configuration();
        let first = extracted_archive(3, 3);
        let duplicate = extracted_archive(3, 4);
        assert_eq!(
            construct_selected_message_flow_programs(
                profile.content_sha256,
                &runtime,
                &profile,
                "us",
                &[&first, &duplicate],
            )
            .unwrap_err()
            .field(),
            "message_flow_import_profile.selected_resources"
        );

        let mut missing_store = profile;
        missing_store.bindings.temporary_flags = None;
        missing_store.bindings.persistent_flags = None;
        missing_store.bindings.switch_stores.clear();
        let programs = construct_selected_message_flow_programs(
            missing_store.content_sha256,
            &runtime,
            &missing_store,
            "us",
            &[&first],
        )
        .unwrap();
        let compiled = programs[0].compile().unwrap();
        let switch_event = compiled
            .mechanics
            .transitions
            .iter()
            .find(|transition| transition.label.contains("event 14"))
            .unwrap();
        assert!(
            switch_event
                .activation
                .effects
                .iter()
                .all(|effect| { !matches!(effect, StateOperation::WriteBoundRaw { .. }) })
        );
        assert!(
            switch_event
                .activation
                .unknown_requirements
                .iter()
                .any(|requirement| requirement.id.contains("switch-backing"))
        );

        let temporary_event = compiled
            .mechanics
            .transitions
            .iter()
            .find(|transition| transition.label.contains("event 10"))
            .unwrap();
        assert!(
            temporary_event
                .activation
                .effects
                .iter()
                .all(|effect| { !matches!(effect, StateOperation::WriteBoundRaw { .. }) })
        );
        assert!(
            temporary_event
                .activation
                .unknown_requirements
                .iter()
                .any(|requirement| requirement.id.contains("temporary-parameter-0-backing"))
        );

        let persistent_event = compiled
            .mechanics
            .transitions
            .iter()
            .find(|transition| transition.label.contains("event 0"))
            .unwrap();
        assert!(
            persistent_event
                .activation
                .effects
                .iter()
                .all(|effect| { !matches!(effect, StateOperation::WriteBoundRaw { .. }) })
        );
        assert!(
            persistent_event
                .activation
                .unknown_requirements
                .iter()
                .any(|requirement| requirement.id.contains("persistent-parameter-0-backing"))
        );
    }

    #[test]
    fn rejects_bad_targets_and_inexact_control_contracts() {
        let mut bad_target = program();
        let MessageFlowNode::Event {
            next_target_index, ..
        } = &mut bad_target.extracted.nodes[0]
        else {
            unreachable!()
        };
        *next_target_index = 99;
        assert_eq!(
            bad_target.validate().unwrap_err().field(),
            "message_flow_program.extracted.target"
        );

        let mut bad_contract = program();
        bad_contract.event_contracts[0].continuation = MessageEventContinuation::ContractControlled;
        assert_eq!(
            bad_contract.validate().unwrap_err().field(),
            "message_flow_program.event_contracts.continuation"
        );

        let mut mismatched_access = program();
        mismatched_access.extracted.temporary_flag_accesses[0].label_index = 52;
        assert_eq!(
            mismatched_access.validate().unwrap_err().field(),
            "message_flow_program.extracted.temporary_flag_accesses"
        );

        let mut unconditional_cleanup = program();
        unconditional_cleanup.cleanup_edges[0].activation = PredicateExpression::True;
        assert_eq!(
            unconditional_cleanup.validate().unwrap_err().field(),
            "message_flow_program.cleanup.activation"
        );
    }
}
