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
    ActivationContract, CandidateTransition, ComponentFieldTarget, MECHANICS_CATALOG_SCHEMA,
    MechanicsCatalog, ReaderRule, StateOperation, TransitionKind, UnknownRequirement,
};
use crate::{PlannerContractError, canonical_json, validate_label, validate_stable_id};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};

mod compilation;
mod program;
use compilation::*;
#[cfg(test)]
use program::construct_selected_message_flow_programs;

pub const MESSAGE_FLOW_PROGRAM_SCHEMA: &str = "dusklight.route-planner.message-flow-program/v3";
pub const COMPILED_MESSAGE_FLOW_PROGRAM_SCHEMA: &str =
    "dusklight.route-planner.compiled-message-flow-program/v5";
pub const MESSAGE_FLOW_IMPORT_PROFILE_SCHEMA: &str =
    "dusklight.route-planner.message-flow-import-profile/v3";
pub const MESSAGE_FLOW_PROGRAM_SET_SCHEMA: &str =
    "dusklight.route-planner.message-flow-program-set/v3";
const MAX_MESSAGE_FLOW_NODES: usize = 65_535;
const MAX_EVENT_CONTRACTS: usize = 16_384;
const MAX_CLEANUP_EDGES: usize = 256;
const BUNDLED_GZ2E01_ENGLISH_IMPORT_PROFILE: &[u8] =
    include_bytes!("../data/message-import-profiles/gz2e01-en.json");
const BUNDLED_GZ2P01_STRUCTURAL_IMPORT_PROFILE: &[u8] =
    include_bytes!("../data/message-import-profiles/gz2p01-structural.json");

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rupees: Option<ComponentFieldTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub life: Option<ComponentFieldTarget>,
    pub item_ownership: Vec<MessageItemOwnershipBinding>,
    pub switch_stores: Vec<MessageSwitchStoreBinding>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MessageRawStoreBinding {
    pub component_kind: ComponentKind,
    pub binding: ComponentBindingReference,
}

/// Exact raw ownership semantics for one item ID. Item ownership is not a
/// universal inventory bitset in TP; special items such as the Vessels of
/// Light live in dedicated save structures.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MessageItemOwnershipBinding {
    pub item_id: u16,
    pub label: String,
    pub component_kind: ComponentKind,
    pub binding: ComponentBindingReference,
    pub byte_offset: u32,
    pub mask: u8,
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

pub fn bundled_gz2p01_structural_message_flow_profile()
-> Result<MessageFlowImportProfile, PlannerContractError> {
    let profile =
        MessageFlowImportProfile::decode_canonical(BUNDLED_GZ2P01_STRUCTURAL_IMPORT_PROFILE)?;
    if profile.bindings.temporary_flags.is_some()
        || profile.bindings.persistent_flags.is_some()
        || profile.bindings.rupees.is_some()
        || !profile.bindings.item_ownership.is_empty()
        || !profile.bindings.switch_stores.is_empty()
    {
        return Err(PlannerContractError::new(
            "message_flow_import_profile.bindings",
            "GZ2P01 structural profile must not authorize handler backings",
        ));
    }
    Ok(profile)
}

impl MessageFlowBindings {
    pub(crate) fn validate(&self) -> Result<(), PlannerContractError> {
        if let Some(binding) = &self.temporary_flags {
            binding.validate("message_flow_program.bindings.temporary_flags")?;
        }
        if let Some(binding) = &self.persistent_flags {
            binding.validate("message_flow_program.bindings.persistent_flags")?;
        }
        if let Some(target) = &self.rupees {
            validate_stable_id(
                "message_flow_program.bindings.rupees.component_id",
                &target.component_id,
            )?;
            validate_stable_id("message_flow_program.bindings.rupees.field", &target.field)?;
        }
        if let Some(target) = &self.life {
            validate_stable_id(
                "message_flow_program.bindings.life.component_id",
                &target.component_id,
            )?;
            validate_stable_id("message_flow_program.bindings.life.field", &target.field)?;
        }
        let mut prior_item = None;
        for item in &self.item_ownership {
            item.validate()?;
            if prior_item.is_some_and(|item_id| item_id >= item.item_id) {
                return Err(PlannerContractError::new(
                    "message_flow_program.bindings.item_ownership",
                    "must be unique and sorted by item ID",
                ));
            }
            prior_item = Some(item.item_id);
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

    fn item(&self, item_id: u16) -> Option<&MessageItemOwnershipBinding> {
        self.item_ownership
            .binary_search_by_key(&item_id, |binding| binding.item_id)
            .ok()
            .map(|index| &self.item_ownership[index])
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

impl MessageItemOwnershipBinding {
    pub(crate) fn validate(&self) -> Result<(), PlannerContractError> {
        if self.item_id > u16::from(u8::MAX) {
            return Err(PlannerContractError::new(
                "message_flow_program.bindings.item_ownership.item_id",
                "must fit the retail item ID width",
            ));
        }
        validate_label(
            "message_flow_program.bindings.item_ownership.label",
            &self.label,
        )?;
        MessageRawStoreBinding {
            component_kind: self.component_kind.clone(),
            binding: self.binding.clone(),
        }
        .validate("message_flow_program.bindings.item_ownership")?;
        if !self.mask.is_power_of_two() {
            return Err(PlannerContractError::new(
                "message_flow_program.bindings.item_ownership.mask",
                "must select exactly one ownership bit",
            ));
        }
        Ok(())
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

#[cfg(test)]
#[path = "message_flow_tests.rs"]
mod tests;
