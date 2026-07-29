//! Compilation and catalog integration for exact selected message programs.

use crate::artifact::Digest;
use crate::identity::{ContextSelector, ExactContext, RuntimeConfiguration};
use crate::logic::{
    ComparisonOperator, ContextScope, EvidenceKind, FACT_CATALOG_SCHEMA, FactCatalog,
    PredicateExpression, RuleEvidence, TruthStatus, ValueReference,
};
use crate::message_flow::{
    CompiledMessageFlowProgram, MessageCleanupEdge, MessageEventContinuation, MessageEventContract,
    MessageFlowImportProfile, MessageFlowProgram, MessageFlowProgramSet,
    MessageItemOwnershipBinding,
};
use crate::orig_discovery::ExtractedOrigBundle;
use crate::state::StateValue;
use crate::transition::{
    ActivationContract, CandidateTransition, ComponentFieldTarget, FeasibilityObligation,
    MECHANICS_CATALOG_SCHEMA, MechanicsCatalog, ObligationDetail, ObligationKind, ReaderRule,
    StateOperation, TransitionKind, UnknownRequirement,
};
use crate::{PlannerContractError, canonical_json, validate_label, validate_stable_id};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

mod overlay_compilation;
use overlay_compilation::*;

pub const MESSAGE_FLOW_RESOURCE_OVERLAY_SET_SCHEMA: &str =
    "dusklight.route-planner.message-flow-resource-overlay-set/v2";
pub const COMPILED_MESSAGE_FLOW_SET_SCHEMA: &str =
    "dusklight.route-planner.compiled-message-flow-set/v5";
pub const MESSAGE_FLOW_ENTRY_CONTRACT_SET_SCHEMA: &str =
    "dusklight.route-planner.message-flow-entry-contract-set/v4";
pub const COMPILED_MESSAGE_FLOW_ENTRY_SET_SCHEMA: &str =
    "dusklight.route-planner.compiled-message-flow-entry-set/v4";
const MAX_RESOURCE_OVERLAYS: usize = 256;
const MAX_ENTRY_CONTRACTS: usize = 65_536;
const BUNDLED_GZ2E01_ENGLISH_LANAYRU_ENTRY_CONTRACTS: &[u8] =
    include_bytes!("../data/message-entry-contracts/gz2e01-en-lanayru.json");

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

/// Authored entry edges connect static stage/actor evidence to an exact BMG
/// label. The BMG resource alone cannot identify which actor can start a flow
/// or from which interaction geometry.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MessageFlowEntryContractSet {
    pub schema: String,
    pub id: String,
    pub compiled_message_flow_set_schema: String,
    pub compiled_message_flow_set_sha256: Digest,
    pub entries: Vec<MessageFlowEntryContract>,
    pub presentation_requests: Vec<MessagePresentationRequestContract>,
}

/// An exact actor caller consuming `event008`'s flow fields and attempting a
/// presentation-item creation. The helper writes the recent-item byte before
/// actor creation can fail; later actor execution is intentionally separate.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MessagePresentationRequestContract {
    pub id: String,
    pub label: String,
    pub source_entry_id: String,
    pub event_id: u16,
    pub item_id: u16,
    pub recent_item_target: ComponentFieldTarget,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MessageFlowEntryContract {
    pub id: String,
    pub label: String,
    pub message_group: u8,
    pub resource_sha256: Digest,
    pub flow_id: u16,
    pub source_stage: String,
    pub source_room: Option<i8>,
    pub source_layer: Option<i8>,
    pub stage_archive_path: String,
    pub stage_resource_sha256: Digest,
    pub speaker: MessageSpeakerContext,
    pub additional_hard_guard: PredicateExpression,
    pub obligations: Vec<FeasibilityObligation>,
    pub unknown_requirements: Vec<UnknownRequirement>,
    pub evidence: RuleEvidence,
}

/// The zone is optional because an actor audit may not yet prove its room to
/// zone mapping. Omission leaves projected zone bindings unresolved instead of
/// falling back to the player's room.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MessageSpeakerContext {
    pub instance_id: Option<String>,
    pub placement: Option<MessageSpeakerPlacement>,
    pub stage: String,
    pub room: i8,
    pub zone: Option<i16>,
}

/// Exact actor record inside the pinned stage archive. `raw_hex` retains all
/// placement fields not otherwise named here, so an entry contract cannot drift
/// to a different actor record with the same friendly name.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MessageSpeakerPlacement {
    pub archive_path: String,
    pub resource_sha256: Digest,
    pub chunk_tag: String,
    pub record_index: u32,
    pub layer: Option<u8>,
    pub actor_name: String,
    pub raw_hex: String,
}

/// Portable, solver-ready result of resolving authored entry contracts against
/// one exact extracted stage set and one exact compiled message-flow set.
/// Keeping the source contracts and resolved labels inside the artifact makes
/// its mechanics reproducible after the user's `orig/` input is removed.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledMessageFlowEntrySet {
    pub schema: String,
    pub source_contracts: MessageFlowEntryContractSet,
    pub exact_context: ExactContext,
    pub resolved_entries: Vec<ResolvedMessageFlowEntry>,
    pub resolved_generic_item_grants: Vec<ResolvedGenericItemGrant>,
    pub mechanics: MechanicsCatalog,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedMessageFlowEntry {
    pub entry_id: String,
    pub flow_component_id: String,
    pub node_id: String,
    pub node_index: u16,
}

/// Exact item backing selected for the shared `execItemGet(mGtItm)` consumer.
/// Presentation callers remain independent producers of the recent-item field;
/// this record merely seals the generic consumer used by all of them.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedGenericItemGrant {
    pub item_id: u16,
    pub label: String,
    pub recent_item_source: ComponentFieldTarget,
    pub ownership: MessageItemOwnershipBinding,
    pub evidence: RuleEvidence,
}

pub fn bundled_gz2e01_english_lanayru_entry_contracts()
-> Result<MessageFlowEntryContractSet, PlannerContractError> {
    MessageFlowEntryContractSet::decode_canonical(BUNDLED_GZ2E01_ENGLISH_LANAYRU_ENTRY_CONTRACTS)
}

impl MessageFlowEntryContractSet {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != MESSAGE_FLOW_ENTRY_CONTRACT_SET_SCHEMA {
            return Err(PlannerContractError::new(
                "message_flow_entry_contract_set.schema",
                "is unsupported",
            ));
        }
        validate_stable_id("message_flow_entry_contract_set.id", &self.id)?;
        if self.compiled_message_flow_set_schema != COMPILED_MESSAGE_FLOW_SET_SCHEMA {
            return Err(PlannerContractError::new(
                "message_flow_entry_contract_set.compiled_message_flow_set_schema",
                "does not name the currently supported compiled message-flow schema",
            ));
        }
        require_digest(
            "message_flow_entry_contract_set.compiled_message_flow_set_sha256",
            self.compiled_message_flow_set_sha256,
        )?;
        if self.entries.is_empty() || self.entries.len() > MAX_ENTRY_CONTRACTS {
            return Err(PlannerContractError::new(
                "message_flow_entry_contract_set.entries",
                "must contain between 1 and 65536 entries",
            ));
        }
        let mut prior = None;
        for entry in &self.entries {
            if prior.is_some_and(|id: &str| id >= entry.id.as_str()) {
                return Err(PlannerContractError::new(
                    "message_flow_entry_contract_set.entries",
                    "must be unique and sorted by entry ID",
                ));
            }
            prior = Some(entry.id.as_str());
            entry.validate()?;
        }
        let entry_ids = self
            .entries
            .iter()
            .map(|entry| entry.id.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        let mut prior_request = None;
        for request in &self.presentation_requests {
            if prior_request.is_some_and(|id: &str| id >= request.id.as_str()) {
                return Err(PlannerContractError::new(
                    "message_flow_entry_contract_set.presentation_requests",
                    "must be unique and sorted by request ID",
                ));
            }
            prior_request = Some(request.id.as_str());
            request.validate()?;
            if !entry_ids.contains(request.source_entry_id.as_str()) {
                return Err(PlannerContractError::new(
                    "message_flow_entry_contract_set.presentation_requests.source_entry_id",
                    "must name an entry in the same contract set",
                ));
            }
            if self
                .entries
                .iter()
                .find(|entry| entry.id == request.source_entry_id)
                .is_some_and(|entry| entry.speaker.instance_id.is_none())
            {
                return Err(PlannerContractError::new(
                    "message_flow_entry_contract_set.presentation_requests.source_entry_id",
                    "must name an actor-backed entry",
                ));
            }
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
                "message_flow_entry_contract_set",
                "is not canonical JSON",
            ));
        }
        Ok(set)
    }

    pub fn digest(&self) -> Result<Digest, PlannerContractError> {
        Ok(Digest(Sha256::digest(self.canonical_bytes()?).into()))
    }

    pub fn compile(
        &self,
        bundle: &ExtractedOrigBundle,
        compiled: &CompiledMessageFlowSet,
    ) -> Result<CompiledMessageFlowEntrySet, PlannerContractError> {
        self.validate()?;
        bundle.validate()?;
        compiled.validate()?;
        if self.compiled_message_flow_set_sha256 != compiled.digest()? {
            return Err(PlannerContractError::new(
                "message_flow_entry_contract_set.compiled_message_flow_set_sha256",
                "does not match the selected compiled message-flow set",
            ));
        }
        if bundle.content.digest()? != compiled.exact_context.content_sha256 {
            return Err(PlannerContractError::new(
                "message_flow_entry_contract_set.bundle",
                "does not match the compiled set's exact content",
            ));
        }
        let flow_component_id = compiled
            .resources
            .first()
            .expect("compiled set validation requires resources")
            .source_program
            .flow_component_id
            .clone();
        let mut resolved_entries = Vec::with_capacity(self.entries.len());
        for entry in &self.entries {
            let stage = bundle
                .stages
                .iter()
                .find(|stage| stage.relative_path == entry.stage_archive_path)
                .ok_or_else(|| {
                    PlannerContractError::new(
                        "message_flow_entry_contract.stage_archive_path",
                        "does not name an extracted stage archive",
                    )
                })?;
            if stage.resource_sha256 != entry.stage_resource_sha256
                || !stage_path_names(&stage.relative_path, &entry.source_stage)
                || stage
                    .stage
                    .stage_information
                    .as_ref()
                    .map(|information| information.message_group)
                    != Some(entry.message_group)
            {
                return Err(PlannerContractError::new(
                    "message_flow_entry_contract.stage",
                    "does not reproduce the exact stage resource and selected message group",
                ));
            }
            if let Some(expected) = &entry.speaker.placement {
                let actor_resource = bundle
                    .stages
                    .iter()
                    .find(|stage| stage.relative_path == expected.archive_path)
                    .ok_or_else(|| {
                        PlannerContractError::new(
                            "message_flow_entry_contract.speaker.placement.archive_path",
                            "does not name an extracted stage or room archive",
                        )
                    })?;
                if actor_resource.resource_sha256 != expected.resource_sha256
                    || !stage_path_names(&actor_resource.relative_path, &entry.source_stage)
                {
                    return Err(PlannerContractError::new(
                        "message_flow_entry_contract.speaker.placement.resource_sha256",
                        "does not reproduce an actor resource in the entry's source stage",
                    ));
                }
                let matches = actor_resource
                    .stage
                    .actor_placements
                    .iter()
                    .filter(|placement| expected.matches(placement))
                    .count();
                if matches != 1 {
                    return Err(PlannerContractError::new(
                        "message_flow_entry_contract.speaker.placement",
                        "must identify exactly one actor in the pinned stage resource",
                    ));
                }
            }
            let resource = compiled
                .resources
                .iter()
                .find(|resource| resource.message_group == entry.message_group)
                .ok_or_else(|| {
                    PlannerContractError::new(
                        "message_flow_entry_contract.message_group",
                        "is absent from the compiled message-flow set",
                    )
                })?;
            if resource.resource_sha256 != entry.resource_sha256 {
                return Err(PlannerContractError::new(
                    "message_flow_entry_contract.resource_sha256",
                    "does not match the selected message resource",
                ));
            }
            let compiled_entry = resource
                .compiled_program
                .entry_points
                .iter()
                .find(|candidate| candidate.flow_id == entry.flow_id)
                .ok_or_else(|| {
                    PlannerContractError::new(
                        "message_flow_entry_contract.flow_id",
                        "is absent from the exact selected message resource",
                    )
                })?;
            let source_label = resource
                .source_program
                .extracted
                .labels
                .iter()
                .find(|candidate| candidate.flow_id == entry.flow_id)
                .expect("source and compiled program validation agree on entry labels");
            resolved_entries.push(ResolvedMessageFlowEntry {
                entry_id: entry.id.clone(),
                flow_component_id: flow_component_id.clone(),
                node_id: compiled_entry.node_id.clone(),
                node_index: source_label.node_index,
            });
        }
        let resolved_generic_item_grants =
            resolve_generic_item_grants(self, compiled, &resolved_entries)?;
        let mechanics = compile_entry_mechanics(
            self,
            &compiled.exact_context,
            &resolved_entries,
            &resolved_generic_item_grants,
        )?;
        let result = CompiledMessageFlowEntrySet {
            schema: COMPILED_MESSAGE_FLOW_ENTRY_SET_SCHEMA.into(),
            source_contracts: self.clone(),
            exact_context: compiled.exact_context.clone(),
            resolved_entries,
            resolved_generic_item_grants,
            mechanics,
        };
        result.validate()?;
        Ok(result)
    }
}

impl CompiledMessageFlowEntrySet {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != COMPILED_MESSAGE_FLOW_ENTRY_SET_SCHEMA {
            return Err(PlannerContractError::new(
                "compiled_message_flow_entry_set.schema",
                "is unsupported",
            ));
        }
        self.source_contracts.validate()?;
        ContextScope {
            selectors: vec![ContextSelector::Exact {
                context: self.exact_context.clone(),
            }],
        }
        .validate("compiled_message_flow_entry_set.exact_context")?;
        if self.resolved_entries.len() != self.source_contracts.entries.len() {
            return Err(PlannerContractError::new(
                "compiled_message_flow_entry_set.resolved_entries",
                "must resolve every source entry exactly once",
            ));
        }
        for (source, resolved) in self
            .source_contracts
            .entries
            .iter()
            .zip(&self.resolved_entries)
        {
            if resolved.entry_id != source.id {
                return Err(PlannerContractError::new(
                    "compiled_message_flow_entry_set.resolved_entries",
                    "must be unique, sorted, and aligned with source entry IDs",
                ));
            }
            validate_stable_id(
                "compiled_message_flow_entry_set.flow_component_id",
                &resolved.flow_component_id,
            )?;
            validate_stable_id("compiled_message_flow_entry_set.node_id", &resolved.node_id)?;
        }
        let mut prior_item = None;
        for grant in &self.resolved_generic_item_grants {
            if prior_item.is_some_and(|item_id| item_id >= grant.item_id) {
                return Err(PlannerContractError::new(
                    "compiled_message_flow_entry_set.resolved_generic_item_grants",
                    "must be unique and sorted by item ID",
                ));
            }
            prior_item = Some(grant.item_id);
            grant.validate()?;
            if !self
                .source_contracts
                .presentation_requests
                .iter()
                .any(|request| {
                    request.item_id == grant.item_id
                        && request.recent_item_target == grant.recent_item_source
                })
            {
                return Err(PlannerContractError::new(
                    "compiled_message_flow_entry_set.resolved_generic_item_grants",
                    "must correspond to a presentation request with the same item and recent-item field",
                ));
            }
        }
        let requested_items = self
            .source_contracts
            .presentation_requests
            .iter()
            .map(|request| request.item_id)
            .collect::<std::collections::BTreeSet<_>>();
        if requested_items.len() != self.resolved_generic_item_grants.len()
            || !requested_items.iter().all(|item_id| {
                self.resolved_generic_item_grants
                    .iter()
                    .any(|grant| grant.item_id == *item_id)
            })
        {
            return Err(PlannerContractError::new(
                "compiled_message_flow_entry_set.resolved_generic_item_grants",
                "must resolve every distinct requested item exactly once",
            ));
        }
        let expected = compile_entry_mechanics(
            &self.source_contracts,
            &self.exact_context,
            &self.resolved_entries,
            &self.resolved_generic_item_grants,
        )?;
        if self.mechanics != expected {
            return Err(PlannerContractError::new(
                "compiled_message_flow_entry_set.mechanics",
                "must be the deterministic compilation of source contracts and resolved labels",
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
                "compiled_message_flow_entry_set",
                "is not canonical JSON",
            ));
        }
        Ok(set)
    }

    pub fn digest(&self) -> Result<Digest, PlannerContractError> {
        Ok(Digest(Sha256::digest(self.canonical_bytes()?).into()))
    }

    pub fn merge_into(&self, mechanics: &mut MechanicsCatalog) -> Result<(), PlannerContractError> {
        self.validate()?;
        mechanics.validate()?;
        let mut next_facts = empty_facts();
        let mut next_mechanics = mechanics.clone();
        append_catalogs(
            &mut next_facts,
            &mut next_mechanics,
            empty_facts(),
            self.mechanics.clone(),
        );
        sort_catalogs(&mut next_facts, &mut next_mechanics);
        next_mechanics.validate().map_err(|error| {
            PlannerContractError::new(
                "compiled_message_flow_entry_set.merge.mechanics",
                error.to_string(),
            )
        })?;
        *mechanics = next_mechanics;
        Ok(())
    }
}

fn resolve_generic_item_grants(
    contracts: &MessageFlowEntryContractSet,
    compiled: &CompiledMessageFlowSet,
    resolved_entries: &[ResolvedMessageFlowEntry],
) -> Result<Vec<ResolvedGenericItemGrant>, PlannerContractError> {
    let mut grants = std::collections::BTreeMap::<u16, ResolvedGenericItemGrant>::new();
    for request in &contracts.presentation_requests {
        let (entry, _) = contracts
            .entries
            .iter()
            .zip(resolved_entries)
            .find(|(entry, _)| entry.id == request.source_entry_id)
            .expect("contract-set validation requires the source entry");
        let mut ownership = None::<MessageItemOwnershipBinding>;
        let mut records = std::collections::BTreeMap::new();
        for resource in &compiled.resources {
            if let Some(candidate) = resource
                .source_program
                .bindings
                .item_ownership
                .iter()
                .find(|candidate| candidate.item_id == request.item_id)
            {
                if ownership
                    .as_ref()
                    .is_some_and(|existing| existing != candidate)
                {
                    return Err(PlannerContractError::new(
                        "message_presentation_request.item_id",
                        "has inconsistent ownership backing across selected message resources",
                    ));
                }
                ownership = Some(candidate.clone());
                for record in &resource.source_program.evidence.records {
                    if record.kind == EvidenceKind::SourceAudited {
                        records.entry(record.id.clone()).or_insert(record.clone());
                    }
                }
            }
        }
        let ownership = ownership.ok_or_else(|| {
            PlannerContractError::new(
                "message_presentation_request.item_id",
                "has no exact item-ownership binding in the selected message-flow set",
            )
        })?;
        for record in &entry.evidence.records {
            if record.kind == EvidenceKind::SourceAudited {
                records.entry(record.id.clone()).or_insert(record.clone());
            }
        }
        let evidence = RuleEvidence {
            truth: TruthStatus::Established,
            records: records.into_values().collect(),
        };
        evidence.validate("resolved_generic_item_grant.evidence")?;
        let grant = ResolvedGenericItemGrant {
            item_id: request.item_id,
            label: ownership.label.clone(),
            recent_item_source: request.recent_item_target.clone(),
            ownership,
            evidence,
        };
        grant.validate()?;
        if let Some(existing) = grants.insert(request.item_id, grant.clone())
            && existing != grant
        {
            return Err(PlannerContractError::new(
                "message_flow_entry_contract_set.presentation_requests",
                "requests for the same item must use one recent-item source and ownership backing",
            ));
        }
    }
    Ok(grants.into_values().collect())
}

fn compile_entry_mechanics(
    contracts: &MessageFlowEntryContractSet,
    exact_context: &ExactContext,
    resolved_entries: &[ResolvedMessageFlowEntry],
    resolved_generic_item_grants: &[ResolvedGenericItemGrant],
) -> Result<MechanicsCatalog, PlannerContractError> {
    let scope = ContextScope {
        selectors: vec![ContextSelector::Exact {
            context: exact_context.clone(),
        }],
    };
    let mut transitions = Vec::with_capacity(
        contracts.entries.len()
            + contracts.presentation_requests.len()
            + resolved_generic_item_grants.len(),
    );
    let mut obligations = Vec::new();
    let mut readers = Vec::new();
    for (entry, resolved) in contracts.entries.iter().zip(resolved_entries) {
        if entry
            .obligations
            .iter()
            .any(|obligation| obligation.scope != scope)
        {
            return Err(PlannerContractError::new(
                "message_flow_entry_contract.obligations.scope",
                "must match the compiled set's exact context",
            ));
        }
        let obligation_ids = entry
            .obligations
            .iter()
            .map(|obligation| obligation.id.clone())
            .collect::<Vec<_>>();
        obligations.extend(entry.obligations.clone());
        transitions.push(CandidateTransition {
            id: format!("transition.message-entry.{}", entry.id),
            label: entry.label.clone(),
            scope: scope.clone(),
            transition_kind: TransitionKind::MessageAction,
            approach_id: format!("approach.message-entry.{}", entry.id),
            activation: ActivationContract {
                hard_guards: entry_hard_guard(entry),
                physical_obligation_ids: obligation_ids,
                effects: entry_effects(
                    entry,
                    &resolved.flow_component_id,
                    &resolved.node_id,
                    resolved.node_index,
                ),
                unknown_requirements: entry.unknown_requirements.clone(),
            },
            evidence: entry.evidence.clone(),
        });
    }
    for request in &contracts.presentation_requests {
        let (entry, resolved) = contracts
            .entries
            .iter()
            .zip(resolved_entries)
            .find(|(entry, _)| entry.id == request.source_entry_id)
            .expect("contract-set validation requires the source entry");
        let speaker = entry.speaker.instance_id.as_ref().ok_or_else(|| {
            PlannerContractError::new(
                "message_presentation_request.source_entry_id",
                "must refer to an actor-backed message entry",
            )
        })?;
        let flow_field = |field: &str| ValueReference::ComponentField {
            component_id: resolved.flow_component_id.clone(),
            field: field.into(),
        };
        let transition_id = format!("transition.message-presentation-request.{}", request.id);
        let sources = [
            ("event-id", flow_field("event_id")),
            ("item-id", flow_field("item_id")),
            ("speaker", flow_field("speaker_instance_id")),
        ];
        let hard_guards = PredicateExpression::All {
            terms: vec![
                PredicateExpression::Compare {
                    left: sources[0].1.clone(),
                    operator: ComparisonOperator::Equal,
                    right: ValueReference::Literal {
                        value: StateValue::Unsigned(request.event_id.into()),
                    },
                },
                PredicateExpression::Compare {
                    left: sources[1].1.clone(),
                    operator: ComparisonOperator::Equal,
                    right: ValueReference::Literal {
                        value: StateValue::Unsigned(request.item_id.into()),
                    },
                },
                PredicateExpression::Compare {
                    left: sources[2].1.clone(),
                    operator: ComparisonOperator::Equal,
                    right: ValueReference::Literal {
                        value: StateValue::Text(speaker.clone()),
                    },
                },
            ],
        };
        transitions.push(CandidateTransition {
            id: transition_id.clone(),
            label: request.label.clone(),
            scope: scope.clone(),
            transition_kind: TransitionKind::ActorDriven,
            approach_id: format!("approach.message-presentation-request.{}", request.id),
            activation: ActivationContract {
                hard_guards,
                physical_obligation_ids: Vec::new(),
                effects: vec![StateOperation::CopyValue {
                    source: ComponentFieldTarget {
                        component_id: resolved.flow_component_id.clone(),
                        field: "item_id".into(),
                    },
                    target: request.recent_item_target.clone(),
                }],
                unknown_requirements: Vec::new(),
            },
            evidence: entry.evidence.clone(),
        });
        readers.extend(sources.into_iter().map(|(suffix, source)| ReaderRule {
            id: format!(
                "reader.message-presentation-request.{}.{suffix}",
                request.id
            ),
            scope: scope.clone(),
            source,
            consuming_transition_id: transition_id.clone(),
            interpretation_fact_id: None,
            evidence: entry.evidence.clone(),
        }));
    }
    for grant in resolved_generic_item_grants {
        let item_token = format!("item-{:04x}", grant.item_id);
        let obligation_id =
            format!("obligation.generic-get-item.{item_token}.presentation-actor-execution");
        obligations.push(FeasibilityObligation {
            id: obligation_id.clone(),
            label: format!(
                "Presentation actor for {} executes its grant path",
                grant.label
            ),
            scope: scope.clone(),
            obligation_kind: ObligationKind::ActorState,
            stage: crate::transition::ObligationStage::Effect,
            detail: ObligationDetail::Unresolved {
                research_question: format!(
                    "Prove that the presentation item actor for item 0x{:02x} exists, reaches its grant frame, and does not suppress execItemGet",
                    grant.item_id
                ),
            },
            evidence: grant.evidence.clone(),
        });
        let transition_id = format!("transition.generic-get-item.{item_token}");
        let recent_item_source = ValueReference::ComponentField {
            component_id: grant.recent_item_source.component_id.clone(),
            field: grant.recent_item_source.field.clone(),
        };
        transitions.push(CandidateTransition {
            id: transition_id.clone(),
            label: format!("Grant {} through generic get-item", grant.label),
            scope: scope.clone(),
            transition_kind: TransitionKind::ItemAcquisition,
            approach_id: format!("approach.generic-get-item.{item_token}"),
            activation: ActivationContract {
                hard_guards: PredicateExpression::Compare {
                    left: recent_item_source.clone(),
                    operator: ComparisonOperator::Equal,
                    right: ValueReference::Literal {
                        value: StateValue::Unsigned(grant.item_id.into()),
                    },
                },
                physical_obligation_ids: vec![obligation_id],
                effects: vec![StateOperation::WriteBoundRaw {
                    component_kind: grant.ownership.component_kind.clone(),
                    binding: grant.ownership.binding.clone(),
                    byte_offset: grant.ownership.byte_offset,
                    mask: vec![grant.ownership.mask],
                    value: vec![grant.ownership.mask],
                }],
                unknown_requirements: Vec::new(),
            },
            evidence: grant.evidence.clone(),
        });
        readers.push(ReaderRule {
            id: format!("reader.generic-get-item.{item_token}.recent-item"),
            scope: scope.clone(),
            source: recent_item_source,
            consuming_transition_id: transition_id,
            interpretation_fact_id: None,
            evidence: grant.evidence.clone(),
        });
    }
    transitions.sort_by(|left, right| left.id.cmp(&right.id));
    obligations.sort_by(|left, right| left.id.cmp(&right.id));
    readers.sort_by(|left, right| left.id.cmp(&right.id));
    let mechanics = MechanicsCatalog {
        schema: MECHANICS_CATALOG_SCHEMA.into(),
        transitions,
        obligations,
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
    Ok(mechanics)
}

impl MessageFlowEntryContract {
    fn validate(&self) -> Result<(), PlannerContractError> {
        validate_stable_id("message_flow_entry_contract.id", &self.id)?;
        validate_label("message_flow_entry_contract.label", &self.label)?;
        require_digest(
            "message_flow_entry_contract.resource_sha256",
            self.resource_sha256,
        )?;
        require_digest(
            "message_flow_entry_contract.stage_resource_sha256",
            self.stage_resource_sha256,
        )?;
        validate_game_token(
            "message_flow_entry_contract.source_stage",
            &self.source_stage,
        )?;
        validate_archive_path(
            "message_flow_entry_contract.stage_archive_path",
            &self.stage_archive_path,
        )?;
        self.speaker.validate()?;
        if self.speaker.stage != self.source_stage {
            return Err(PlannerContractError::new(
                "message_flow_entry_contract.speaker.stage",
                "must equal the entry's source stage",
            ));
        }
        if let Some(placement) = &self.speaker.placement {
            if self.source_room != Some(self.speaker.room) {
                return Err(PlannerContractError::new(
                    "message_flow_entry_contract.speaker.room",
                    "an actor caller must be activated from its speaker room",
                ));
            }
            if let Some(layer) = placement.layer {
                let layer = i8::try_from(layer).map_err(|_| {
                    PlannerContractError::new(
                        "message_flow_entry_contract.speaker.placement.layer",
                        "does not fit the runtime layer representation",
                    )
                })?;
                if self.source_layer != Some(layer) {
                    return Err(PlannerContractError::new(
                        "message_flow_entry_contract.speaker.placement.layer",
                        "must equal the actor caller's source layer",
                    ));
                }
            }
        }
        self.additional_hard_guard.validate()?;
        let mut obligation_ids = std::collections::BTreeSet::new();
        for obligation in &self.obligations {
            if !obligation_ids.insert(obligation.id.as_str()) {
                return Err(PlannerContractError::new(
                    "message_flow_entry_contract.obligations",
                    "must have unique IDs",
                ));
            }
        }
        let obligation_catalog = MechanicsCatalog {
            schema: MECHANICS_CATALOG_SCHEMA.into(),
            transitions: Vec::new(),
            obligations: self.obligations.clone(),
            writers: Vec::new(),
            gates: Vec::new(),
            readers: Vec::new(),
            reconstruction_rules: Vec::new(),
            obstructions: Vec::new(),
            resolvers: Vec::new(),
            techniques: Vec::new(),
            microtraces: Vec::new(),
            goals: Vec::new(),
        };
        obligation_catalog.validate()?;
        let mut unknown_ids = std::collections::BTreeSet::new();
        for unknown in &self.unknown_requirements {
            if !unknown_ids.insert(unknown.id.as_str()) {
                return Err(PlannerContractError::new(
                    "message_flow_entry_contract.unknown_requirements",
                    "must have unique IDs",
                ));
            }
            validate_stable_id("message_flow_entry_contract.unknown.id", &unknown.id)?;
            validate_label(
                "message_flow_entry_contract.unknown.description",
                &unknown.description,
            )?;
            unknown
                .evidence
                .validate("message_flow_entry_contract.unknown.evidence")?;
            if unknown.evidence.truth != TruthStatus::Unknown {
                return Err(PlannerContractError::new(
                    "message_flow_entry_contract.unknown.evidence",
                    "must retain unknown truth",
                ));
            }
        }
        self.evidence
            .validate("message_flow_entry_contract.evidence")?;
        if self.evidence.truth == TruthStatus::Unknown
            || !self
                .evidence
                .records
                .iter()
                .any(|record| record.source_sha256 == Some(self.resource_sha256))
            || !self
                .evidence
                .records
                .iter()
                .any(|record| record.source_sha256 == Some(self.stage_resource_sha256))
            || self.speaker.placement.as_ref().is_some_and(|placement| {
                !self
                    .evidence
                    .records
                    .iter()
                    .any(|record| record.source_sha256 == Some(placement.resource_sha256))
            })
        {
            return Err(PlannerContractError::new(
                "message_flow_entry_contract.evidence",
                "must be non-unknown and cite the exact message, stage, and actor resources",
            ));
        }
        Ok(())
    }
}

impl MessagePresentationRequestContract {
    fn validate(&self) -> Result<(), PlannerContractError> {
        validate_stable_id("message_presentation_request.id", &self.id)?;
        validate_label("message_presentation_request.label", &self.label)?;
        validate_stable_id(
            "message_presentation_request.source_entry_id",
            &self.source_entry_id,
        )?;
        if self.event_id > u16::from(u8::MAX) || self.item_id > u16::from(u8::MAX) {
            return Err(PlannerContractError::new(
                "message_presentation_request",
                "event and item IDs must fit their retail byte-width handoff fields",
            ));
        }
        StateOperation::CopyValue {
            source: ComponentFieldTarget {
                component_id: "message-session".into(),
                field: "item_id".into(),
            },
            target: self.recent_item_target.clone(),
        }
        .validate()
        .map_err(|error| {
            PlannerContractError::new(
                "message_presentation_request.recent_item_target",
                error.detail(),
            )
        })
    }
}

impl ResolvedGenericItemGrant {
    fn validate(&self) -> Result<(), PlannerContractError> {
        if self.item_id != self.ownership.item_id {
            return Err(PlannerContractError::new(
                "resolved_generic_item_grant.item_id",
                "must equal the resolved ownership binding's item ID",
            ));
        }
        validate_label("resolved_generic_item_grant.label", &self.label)?;
        if self.label != self.ownership.label {
            return Err(PlannerContractError::new(
                "resolved_generic_item_grant.label",
                "must equal the resolved ownership binding's label",
            ));
        }
        self.ownership.validate()?;
        StateOperation::CopyValue {
            source: ComponentFieldTarget {
                component_id: "validation-source".into(),
                field: "item-id".into(),
            },
            target: self.recent_item_source.clone(),
        }
        .validate()
        .map_err(|error| {
            PlannerContractError::new(
                "resolved_generic_item_grant.recent_item_source",
                error.detail(),
            )
        })?;
        self.evidence
            .validate("resolved_generic_item_grant.evidence")?;
        if self.evidence.truth == TruthStatus::Unknown
            || !self
                .evidence
                .records
                .iter()
                .any(|record| record.kind == EvidenceKind::SourceAudited)
        {
            return Err(PlannerContractError::new(
                "resolved_generic_item_grant.evidence",
                "must contain non-unknown source-audited evidence",
            ));
        }
        Ok(())
    }
}

impl MessageSpeakerContext {
    fn validate(&self) -> Result<(), PlannerContractError> {
        if let Some(instance_id) = &self.instance_id {
            validate_stable_id("message_speaker_context.instance_id", instance_id)?;
        }
        if self.instance_id.is_some() != self.placement.is_some() {
            return Err(PlannerContractError::new(
                "message_speaker_context.placement",
                "must accompany an actor instance ID and be absent for non-actor entries",
            ));
        }
        if let Some(placement) = &self.placement {
            placement.validate()?;
        }
        validate_game_token("message_speaker_context.stage", &self.stage)
    }
}

impl MessageSpeakerPlacement {
    fn validate(&self) -> Result<(), PlannerContractError> {
        validate_archive_path("message_speaker_placement.archive_path", &self.archive_path)?;
        require_digest(
            "message_speaker_placement.resource_sha256",
            self.resource_sha256,
        )?;
        validate_game_token("message_speaker_placement.chunk_tag", &self.chunk_tag)?;
        validate_game_token("message_speaker_placement.actor_name", &self.actor_name)?;
        if self.raw_hex.is_empty()
            || self.raw_hex.len() > 256
            || !self.raw_hex.len().is_multiple_of(2)
            || !self.raw_hex.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(PlannerContractError::new(
                "message_speaker_placement.raw_hex",
                "must be bounded even-length hexadecimal",
            ));
        }
        Ok(())
    }

    fn matches(&self, placement: &crate::orig_extraction::ExtractedActorPlacement) -> bool {
        self.chunk_tag == placement.chunk_tag
            && self.record_index == placement.record_index
            && self.layer == placement.layer
            && self.actor_name == placement.name
            && self.raw_hex == placement.raw_hex
    }
}

fn validate_archive_path(field: &str, path: &str) -> Result<(), PlannerContractError> {
    if path.is_empty()
        || path.len() > 512
        || path.contains(['\\', '\0'])
        || path.starts_with('/')
        || path.ends_with('/')
        || path
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
    {
        Err(PlannerContractError::new(
            field,
            "must be a bounded normalized archive path",
        ))
    } else {
        Ok(())
    }
}

fn validate_game_token(field: &str, value: &str) -> Result<(), PlannerContractError> {
    if value.is_empty()
        || value.len() > 16
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(PlannerContractError::new(
            field,
            "must be a bounded ASCII game identifier",
        ));
    }
    Ok(())
}

fn stage_path_names(path: &str, stage: &str) -> bool {
    path.split('/')
        .collect::<Vec<_>>()
        .windows(2)
        .any(|parts| parts[0] == "Stage" && parts[1] == stage)
}

fn entry_hard_guard(entry: &MessageFlowEntryContract) -> PredicateExpression {
    let mut terms = vec![PredicateExpression::Compare {
        left: ValueReference::LocationStage,
        operator: ComparisonOperator::Equal,
        right: ValueReference::Literal {
            value: StateValue::Text(entry.source_stage.clone()),
        },
    }];
    if let Some(room) = entry.source_room {
        terms.push(PredicateExpression::Compare {
            left: ValueReference::LocationRoom,
            operator: ComparisonOperator::Equal,
            right: ValueReference::Literal {
                value: StateValue::Signed(room.into()),
            },
        });
    }
    if let Some(layer) = entry.source_layer {
        terms.push(PredicateExpression::Compare {
            left: ValueReference::LocationLayer,
            operator: ComparisonOperator::Equal,
            right: ValueReference::Literal {
                value: StateValue::Signed(layer.into()),
            },
        });
    }
    terms.push(entry.additional_hard_guard.clone());
    PredicateExpression::All { terms }
}

fn entry_effects(
    entry: &MessageFlowEntryContract,
    flow_component_id: &str,
    node_id: &str,
    node_index: u16,
) -> Vec<StateOperation> {
    let target = |field: &str| ComponentFieldTarget {
        component_id: flow_component_id.into(),
        field: field.into(),
    };
    let mut effects = vec![
        StateOperation::Write {
            target: target("message_group"),
            value: StateValue::Unsigned(entry.message_group.into()),
        },
        StateOperation::Write {
            target: target("resource_sha256"),
            value: StateValue::Bytes(entry.resource_sha256.as_bytes().to_vec()),
        },
        StateOperation::Write {
            target: target("flow_id"),
            value: StateValue::Unsigned(entry.flow_id.into()),
        },
        StateOperation::Write {
            target: target("node_index"),
            value: StateValue::Unsigned(node_index.into()),
        },
        StateOperation::Write {
            target: target("speaker_present"),
            value: StateValue::Boolean(entry.speaker.instance_id.is_some()),
        },
        StateOperation::Write {
            target: target("speaker_stage"),
            value: StateValue::Text(entry.speaker.stage.clone()),
        },
        StateOperation::Write {
            target: target("speaker_room"),
            value: StateValue::Signed(entry.speaker.room.into()),
        },
    ];
    match &entry.speaker.instance_id {
        Some(instance_id) => effects.push(StateOperation::Write {
            target: target("speaker_instance_id"),
            value: StateValue::Text(instance_id.clone()),
        }),
        None => effects.push(StateOperation::ClearField {
            target: target("speaker_instance_id"),
        }),
    }
    match entry.speaker.zone {
        Some(zone) => effects.push(StateOperation::Write {
            target: target("speaker_zone"),
            value: StateValue::Signed(zone.into()),
        }),
        None => effects.push(StateOperation::InvalidateField {
            target: target("speaker_zone"),
        }),
    }
    effects.push(StateOperation::ClearField {
        target: target("last_edge_id"),
    });
    effects.push(StateOperation::AdvanceFlow {
        flow_component_id: flow_component_id.into(),
        node_id: node_id.into(),
    });
    effects
}

#[cfg(test)]
#[path = "message_import_tests.rs"]
mod tests;
