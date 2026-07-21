//! Causal transitions, activation obligations, mechanics, and techniques.

use crate::artifact::Digest;
use crate::logic::{ContextScope, PredicateExpression, RuleEvidence, ValueReference};
use crate::state::{
    ComponentBinding, ComponentSelector, PlaneRelation, RuntimeFile, RuntimeFileLifecycle,
    SemanticLifetime, SerializationOwner, StateComponent, StateValue,
    validate_binding as validate_component_binding, validate_component_kind,
    validate_serialization_owner,
};
use crate::{PlannerContractError, canonical_json, validate_label, validate_stable_id};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const MECHANICS_CATALOG_SCHEMA: &str = "dusklight.route-planner.mechanics-catalog/v7";
pub const MAX_MECHANICS_RECORDS: usize = 65_536;

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentFieldTarget {
    pub component_id: String,
    pub field: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StateOperation {
    Write {
        target: ComponentFieldTarget,
        value: StateValue,
    },
    CopyValue {
        source: ComponentFieldTarget,
        target: ComponentFieldTarget,
    },
    /// Inserts a runtime-selected nonnegative integer into a byte-backed set.
    /// Value `n` selects bit `n % 8` of byte `n / 8`; the operation never
    /// replaces existing members.
    SetBitFromValue {
        source: ComponentFieldTarget,
        target: ComponentFieldTarget,
    },
    WriteRaw {
        component_id: String,
        byte_offset: u32,
        mask: Vec<u8>,
        value: Vec<u8>,
    },
    InvalidateRaw {
        component_id: String,
        byte_offset: u32,
        mask: Vec<u8>,
    },
    Adjust {
        target: ComponentFieldTarget,
        delta: i64,
    },
    ClearComponent {
        selector: ComponentSelector,
    },
    ClearField {
        target: ComponentFieldTarget,
    },
    Initialize {
        component: StateComponent,
    },
    Copy {
        source: ComponentSelector,
        destination_component_id: String,
        binding: ComponentBinding,
        serialization_owner: SerializationOwner,
    },
    Move {
        source: ComponentSelector,
        destination_component_id: String,
        binding: ComponentBinding,
        serialization_owner: SerializationOwner,
    },
    Preserve {
        selector: ComponentSelector,
    },
    Serialize {
        selector: ComponentSelector,
        owner: SerializationOwner,
    },
    Restore {
        owner: SerializationOwner,
        destination_component_id: String,
    },
    Bind {
        selector: ComponentSelector,
        binding: ComponentBinding,
    },
    Rebind {
        selector: ComponentSelector,
        binding: ComponentBinding,
    },
    SetActiveRuntimeFile {
        runtime_file: RuntimeFile,
    },
    SetLocation {
        location: crate::state::SceneLocation,
    },
    Project {
        source_runtime_file_id: String,
        destination_runtime_file_id: String,
        component_ids: Vec<String>,
    },
    Consume {
        pending_operation_id: String,
    },
    SetGate {
        gate_id: String,
    },
    ClearGate {
        gate_id: String,
    },
    AdvanceFlow {
        flow_component_id: String,
        node_id: String,
    },
    BranchFlow {
        flow_component_id: String,
        edge_id: String,
        destination_node_id: String,
    },
    ScheduleCleanup {
        cleanup_id: String,
    },
    CancelCleanup {
        cleanup_id: String,
    },
    Interrupt {
        action_id: String,
        window: TemporalWindow,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TemporalWindow {
    pub earliest_frame: i32,
    pub latest_frame: i32,
    pub required_input: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TemporalRequirement {
    pub action_id: String,
    pub window: TemporalWindow,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TransitionKind {
    EncodedMapExit,
    Door,
    Spawn,
    SaveWarp,
    ItemAcquisition,
    NpcReward,
    Cutscene,
    CutsceneSceneChange,
    ResourceLoadFailure,
    BossCompletion,
    FormChange,
    MountChange,
    SaveLoad,
    MessageAction,
    ActorReload,
    Technique,
    Other,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UnknownRequirement {
    pub id: String,
    pub description: String,
    pub evidence: RuleEvidence,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActivationContract {
    pub hard_guards: PredicateExpression,
    pub physical_obligation_ids: Vec<String>,
    pub effects: Vec<StateOperation>,
    pub unknown_requirements: Vec<UnknownRequirement>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateTransition {
    pub id: String,
    pub label: String,
    pub scope: ContextScope,
    pub transition_kind: TransitionKind,
    pub approach_id: String,
    pub activation: ActivationContract,
    pub evidence: RuleEvidence,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObligationKind {
    Geometry,
    Interaction,
    Timing,
    PlayerControl,
    ActorState,
    Form,
    Mount,
    Twilight,
    VoidPlane,
    Layer,
    MessageState,
    Other,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VolumeReference {
    pub object_id: String,
    pub volume_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ObligationDetail {
    Predicate {
        predicate: PredicateExpression,
    },
    Interaction {
        actor_instance_id: String,
        interaction_mode: String,
        required_volumes: Vec<VolumeReference>,
        excluded_volumes: Vec<VolumeReference>,
        pose_predicate: PredicateExpression,
        temporal_requirement: Option<TemporalRequirement>,
    },
    Geometry {
        approach_id: String,
        source_region_id: String,
        destination_region_id: String,
    },
    PlaneSide {
        plane_id: String,
        relation: PlaneRelation,
    },
    Temporal {
        requirement: TemporalRequirement,
        precondition: PredicateExpression,
    },
    Unresolved {
        research_question: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FeasibilityObligation {
    pub id: String,
    pub label: String,
    pub scope: ContextScope,
    pub obligation_kind: ObligationKind,
    pub detail: ObligationDetail,
    pub evidence: RuleEvidence,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WriterRule {
    pub id: String,
    pub scope: ContextScope,
    pub activation: PredicateExpression,
    pub operation: StateOperation,
    pub evidence: RuleEvidence,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GateRule {
    pub id: String,
    pub scope: ContextScope,
    pub active_when: PredicateExpression,
    pub blocked_writer_ids: Vec<String>,
    pub lifetime: SemanticLifetime,
    pub evidence: RuleEvidence,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReaderRule {
    pub id: String,
    pub scope: ContextScope,
    pub source: ValueReference,
    pub consuming_transition_id: String,
    pub interpretation_fact_id: Option<String>,
    pub evidence: RuleEvidence,
}

/// Reconstructs a live actor from static placement plus persisted controls.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActorReconstructionRule {
    pub id: String,
    pub label: String,
    pub scope: ContextScope,
    pub actor_type: String,
    pub instantiate_when: PredicateExpression,
    pub initialization_operations: Vec<StateOperation>,
    pub evidence: RuleEvidence,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Obstruction {
    pub id: String,
    pub label: String,
    pub scope: ContextScope,
    pub blocked_action_id: String,
    pub approach_id: String,
    pub active_when: PredicateExpression,
    pub obligation_ids: Vec<String>,
    pub evidence: RuleEvidence,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionKind {
    Satisfy,
    Bypass,
    Avoid,
    Supersede,
    AssumeAbsent,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObstructionResolver {
    pub id: String,
    pub label: String,
    pub scope: ContextScope,
    pub obstruction_id: String,
    pub resolution_kind: ResolutionKind,
    pub applicable_when: PredicateExpression,
    pub operations: Vec<StateOperation>,
    pub evidence: RuleEvidence,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RouteCost {
    pub axes: BTreeMap<String, u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Technique {
    pub id: String,
    pub label: String,
    pub scope: ContextScope,
    pub prerequisites: PredicateExpression,
    pub operations: Vec<StateOperation>,
    pub discharged_obligation_ids: Vec<String>,
    pub introduced_obligation_ids: Vec<String>,
    pub cost: RouteCost,
    pub evidence: RuleEvidence,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MessageFlowState {
    pub component_id: String,
    pub flow_id: String,
    pub node_id: String,
    pub cut_id: Option<String>,
    pub pending_cleanup_ids: Vec<String>,
    pub player_has_control: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WitnessedMicrotrace {
    pub id: String,
    pub scope: ContextScope,
    pub precondition: PredicateExpression,
    pub operations: Vec<StateOperation>,
    pub postcondition: PredicateExpression,
    pub timing: TemporalWindow,
    pub evidence: RuleEvidence,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Goal {
    pub id: String,
    pub label: String,
    pub predicate: PredicateExpression,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PathConstraint {
    RequirePredicate { predicate: PredicateExpression },
    ForbidPredicate { predicate: PredicateExpression },
    RequireTechnique { technique_id: String },
    ForbidTechnique { technique_id: String },
    EvidenceAtLeast { minimum: String },
    CostAtMost { axis: String, maximum: u64 },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MechanicsCatalog {
    pub schema: String,
    pub transitions: Vec<CandidateTransition>,
    pub obligations: Vec<FeasibilityObligation>,
    pub writers: Vec<WriterRule>,
    pub gates: Vec<GateRule>,
    pub readers: Vec<ReaderRule>,
    pub reconstruction_rules: Vec<ActorReconstructionRule>,
    pub obstructions: Vec<Obstruction>,
    pub resolvers: Vec<ObstructionResolver>,
    pub techniques: Vec<Technique>,
    pub microtraces: Vec<WitnessedMicrotrace>,
    pub goals: Vec<Goal>,
}

impl StateOperation {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        match self {
            Self::Write { target, value } => {
                validate_field_target(target)?;
                validate_state_value(value)
            }
            Self::CopyValue { source, target } | Self::SetBitFromValue { source, target } => {
                validate_field_target(source)?;
                validate_field_target(target)?;
                if source == target {
                    return Err(PlannerContractError::new(
                        "operation.target",
                        "must differ from the source field",
                    ));
                }
                Ok(())
            }
            Self::WriteRaw {
                component_id,
                byte_offset: _,
                mask,
                value,
            } => {
                validate_stable_id("operation.component_id", component_id)?;
                if mask.is_empty()
                    || mask.len() != value.len()
                    || mask.len() > crate::state::MAX_COMPONENT_BYTES
                {
                    return Err(PlannerContractError::new(
                        "operation.write_raw",
                        "mask/value must have equal nonzero bounded lengths",
                    ));
                }
                if mask.iter().all(|byte| *byte == 0) {
                    return Err(PlannerContractError::new(
                        "operation.write_raw.mask",
                        "must select at least one bit",
                    ));
                }
                Ok(())
            }
            Self::InvalidateRaw {
                component_id,
                byte_offset: _,
                mask,
            } => {
                validate_stable_id("operation.component_id", component_id)?;
                if mask.is_empty() || mask.len() > crate::state::MAX_COMPONENT_BYTES {
                    return Err(PlannerContractError::new(
                        "operation.invalidate_raw.mask",
                        "must have a nonzero bounded length",
                    ));
                }
                if mask.iter().all(|byte| *byte == 0) {
                    return Err(PlannerContractError::new(
                        "operation.invalidate_raw.mask",
                        "must select at least one bit",
                    ));
                }
                Ok(())
            }
            Self::Adjust { target, delta } => {
                validate_field_target(target)?;
                if *delta == 0 {
                    return Err(PlannerContractError::new(
                        "operation.adjust.delta",
                        "must be nonzero",
                    ));
                }
                Ok(())
            }
            Self::ClearComponent { selector } | Self::Preserve { selector } => {
                validate_component_selector(selector)
            }
            Self::ClearField { target } => validate_field_target(target),
            Self::Initialize { component } => component.validate(),
            Self::Copy {
                source,
                destination_component_id,
                binding,
                serialization_owner,
            }
            | Self::Move {
                source,
                destination_component_id,
                binding,
                serialization_owner,
            } => {
                validate_component_selector(source)?;
                validate_stable_id(
                    "operation.destination_component_id",
                    destination_component_id,
                )?;
                validate_binding(binding)?;
                validate_owner(serialization_owner)
            }
            Self::Serialize { selector, owner } => {
                validate_component_selector(selector)?;
                validate_owner(owner)
            }
            Self::Restore {
                owner,
                destination_component_id,
            } => {
                validate_owner(owner)?;
                validate_stable_id(
                    "operation.destination_component_id",
                    destination_component_id,
                )
            }
            Self::Bind { selector, binding } | Self::Rebind { selector, binding } => {
                validate_component_selector(selector)?;
                validate_binding(binding)
            }
            Self::SetActiveRuntimeFile { runtime_file } => {
                runtime_file.validate()?;
                if runtime_file.lifecycle != RuntimeFileLifecycle::Active {
                    return Err(PlannerContractError::new(
                        "operation.runtime_file.lifecycle",
                        "must be active",
                    ));
                }
                Ok(())
            }
            Self::SetLocation { location } => location.validate(),
            Self::Project {
                source_runtime_file_id,
                destination_runtime_file_id,
                component_ids,
            } => {
                validate_stable_id("operation.source_runtime_file_id", source_runtime_file_id)?;
                validate_stable_id(
                    "operation.destination_runtime_file_id",
                    destination_runtime_file_id,
                )?;
                validate_id_list("operation.component_ids", component_ids, false)
            }
            Self::Consume {
                pending_operation_id,
            } => validate_stable_id("operation.pending_operation_id", pending_operation_id),
            Self::SetGate { gate_id } | Self::ClearGate { gate_id } => {
                validate_stable_id("operation.gate_id", gate_id)
            }
            Self::AdvanceFlow {
                flow_component_id,
                node_id,
            } => {
                validate_stable_id("operation.flow_component_id", flow_component_id)?;
                validate_stable_id("operation.node_id", node_id)
            }
            Self::BranchFlow {
                flow_component_id,
                edge_id,
                destination_node_id,
            } => {
                validate_stable_id("operation.flow_component_id", flow_component_id)?;
                validate_stable_id("operation.edge_id", edge_id)?;
                validate_stable_id("operation.destination_node_id", destination_node_id)
            }
            Self::ScheduleCleanup { cleanup_id } | Self::CancelCleanup { cleanup_id } => {
                validate_stable_id("operation.cleanup_id", cleanup_id)
            }
            Self::Interrupt { action_id, window } => {
                validate_stable_id("operation.action_id", action_id)?;
                window.validate()
            }
        }
    }
}

impl TemporalWindow {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.earliest_frame > self.latest_frame
            || self.latest_frame.saturating_sub(self.earliest_frame) > 1_000_000
        {
            return Err(PlannerContractError::new(
                "temporal_window",
                "must be ordered and span at most 1000000 frames",
            ));
        }
        if let Some(input) = &self.required_input {
            validate_stable_id("temporal_window.required_input", input)?;
        }
        Ok(())
    }

    pub fn satisfies(&self, requirement: &Self) -> bool {
        self.earliest_frame >= requirement.earliest_frame
            && self.latest_frame <= requirement.latest_frame
            && requirement
                .required_input
                .as_ref()
                .is_none_or(|required| self.required_input.as_ref() == Some(required))
    }
}

impl TemporalRequirement {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        validate_stable_id("temporal_requirement.action_id", &self.action_id)?;
        self.window.validate()
    }
}

impl WitnessedMicrotrace {
    pub fn witnesses(&self, requirement: &TemporalRequirement) -> bool {
        self.timing.satisfies(&requirement.window)
            && self.operations.iter().any(|operation| {
                matches!(
                    operation,
                    StateOperation::Interrupt { action_id, window }
                        if action_id == &requirement.action_id
                            && window.satisfies(&requirement.window)
                )
            })
    }
}

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

fn validate_transition(transition: &CandidateTransition) -> Result<(), PlannerContractError> {
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
    transition.evidence.validate("transitions.evidence")
}

fn validate_obligation(obligation: &FeasibilityObligation) -> Result<(), PlannerContractError> {
    validate_stable_id("obligations.id", &obligation.id)?;
    validate_label("obligations.label", &obligation.label)?;
    obligation.scope.validate("obligations.scope")?;
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

fn validate_writer(writer: &WriterRule) -> Result<(), PlannerContractError> {
    validate_stable_id("writers.id", &writer.id)?;
    writer.scope.validate("writers.scope")?;
    writer.activation.validate()?;
    writer.operation.validate()?;
    writer.evidence.validate("writers.evidence")
}

fn validate_gate(gate: &GateRule, writer_ids: &BTreeSet<&str>) -> Result<(), PlannerContractError> {
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

fn validate_reader(
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

fn validate_reconstruction_rule(
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

fn validate_obstruction(
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

fn validate_resolver(
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

fn validate_technique(
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

fn validate_microtrace(trace: &WitnessedMicrotrace) -> Result<(), PlannerContractError> {
    validate_stable_id("microtraces.id", &trace.id)?;
    trace.scope.validate("microtraces.scope")?;
    trace.precondition.validate()?;
    validate_operations(&trace.operations)?;
    trace.postcondition.validate()?;
    trace.timing.validate()?;
    trace.evidence.validate("microtraces.evidence")
}

fn validate_goal(goal: &Goal) -> Result<(), PlannerContractError> {
    validate_stable_id("goals.id", &goal.id)?;
    validate_label("goals.label", &goal.label)?;
    goal.predicate.validate()
}

fn validate_operations(operations: &[StateOperation]) -> Result<(), PlannerContractError> {
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

fn validate_field_target(target: &ComponentFieldTarget) -> Result<(), PlannerContractError> {
    validate_stable_id("operation.target.component_id", &target.component_id)?;
    validate_stable_id("operation.target.field", &target.field)
}

fn validate_state_value(value: &StateValue) -> Result<(), PlannerContractError> {
    match value {
        StateValue::Text(value) => validate_label("operation.value", value),
        StateValue::Bytes(value) if value.len() > 1024 * 1024 => Err(PlannerContractError::new(
            "operation.value",
            "byte values must contain at most 1 MiB",
        )),
        _ => Ok(()),
    }
}

fn validate_component_selector(selector: &ComponentSelector) -> Result<(), PlannerContractError> {
    match selector {
        ComponentSelector::Id { component_id } => {
            validate_stable_id("operation.selector.component_id", component_id)
        }
        ComponentSelector::Kind { component_kind } => validate_component_kind(component_kind),
        ComponentSelector::Binding { binding } => validate_binding(binding),
    }
}

fn validate_binding(binding: &ComponentBinding) -> Result<(), PlannerContractError> {
    validate_component_binding(binding)
}

fn validate_owner(owner: &SerializationOwner) -> Result<(), PlannerContractError> {
    validate_serialization_owner(owner)
}

fn validate_value_reference(reference: &ValueReference) -> Result<(), PlannerContractError> {
    PredicateExpression::Compare {
        left: reference.clone(),
        operator: crate::logic::ComparisonOperator::Equal,
        right: ValueReference::Literal {
            value: StateValue::Boolean(true),
        },
    }
    .validate()
}

fn validate_volumes(volumes: &[VolumeReference]) -> Result<(), PlannerContractError> {
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

fn validate_id_list(
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

fn validate_sorted_records<'a, T>(
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

fn require_known_ids(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{ContextSelector, ExactContext};
    use crate::logic::{EvidenceKind, EvidenceRecord, TruthStatus};

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

    fn evidence(truth: TruthStatus) -> RuleEvidence {
        RuleEvidence {
            truth,
            records: if truth == TruthStatus::Unknown {
                Vec::new()
            } else {
                vec![EvidenceRecord {
                    id: "source.test".into(),
                    kind: EvidenceKind::SourceAudited,
                    source_sha256: Some(Digest([3; 32])),
                    note: "Test source audit.".into(),
                }]
            },
        }
    }

    fn locked_door_catalog() -> MechanicsCatalog {
        MechanicsCatalog {
            schema: MECHANICS_CATALOG_SCHEMA.into(),
            transitions: vec![CandidateTransition {
                id: "transition.forest.door-1".into(),
                label: "Enter the next Forest Temple room".into(),
                scope: scope(),
                transition_kind: TransitionKind::Door,
                approach_id: "approach.front".into(),
                activation: ActivationContract {
                    hard_guards: PredicateExpression::Fact {
                        fact_id: "dungeon.forest.small-key-positive".into(),
                    },
                    physical_obligation_ids: vec!["obligation.reach-door".into()],
                    effects: vec![StateOperation::Write {
                        target: ComponentFieldTarget {
                            component_id: "forest-memory".into(),
                            field: "small-keys".into(),
                        },
                        value: StateValue::Unsigned(0),
                    }],
                    unknown_requirements: Vec::new(),
                },
                evidence: evidence(TruthStatus::Established),
            }],
            obligations: vec![FeasibilityObligation {
                id: "obligation.reach-door".into(),
                label: "Reach the front of the locked door".into(),
                scope: scope(),
                obligation_kind: ObligationKind::Geometry,
                detail: ObligationDetail::Geometry {
                    approach_id: "approach.front".into(),
                    source_region_id: "forest.room-0".into(),
                    destination_region_id: "forest.door-1.front".into(),
                },
                evidence: evidence(TruthStatus::Unknown),
            }],
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

    #[test]
    fn encoded_destination_requires_both_hard_guard_and_physical_obligation() {
        let catalog = locked_door_catalog();
        catalog.validate().unwrap();
        let bytes = catalog.canonical_bytes().unwrap();
        assert_eq!(MechanicsCatalog::decode_canonical(&bytes).unwrap(), catalog);
        assert_ne!(catalog.digest().unwrap(), Digest::ZERO);
        let transition = &catalog.transitions[0];
        assert!(matches!(
            transition.activation.hard_guards,
            PredicateExpression::Fact { .. }
        ));
        assert_eq!(
            transition.activation.physical_obligation_ids,
            vec!["obligation.reach-door"]
        );
    }

    #[test]
    fn missing_obligation_reference_fails_closed() {
        let mut catalog = locked_door_catalog();
        catalog.transitions[0].activation.physical_obligation_ids =
            vec!["obligation.missing".into()];
        assert_eq!(
            catalog.validate().unwrap_err().field(),
            "transitions.activation.physical_obligation_ids"
        );
    }

    #[test]
    fn obstruction_resolution_is_directional_and_does_not_delete_world_fact() {
        let mut catalog = locked_door_catalog();
        catalog.obstructions.push(Obstruction {
            id: "obstruction.wall".into(),
            label: "Wall blocks the front approach".into(),
            scope: scope(),
            blocked_action_id: "transition.forest.door-1".into(),
            approach_id: "approach.front".into(),
            active_when: PredicateExpression::True,
            obligation_ids: vec!["obligation.reach-door".into()],
            evidence: evidence(TruthStatus::Established),
        });
        catalog.resolvers.push(ObstructionResolver {
            id: "resolver.wall-clip".into(),
            label: "Clip around this wall".into(),
            scope: scope(),
            obstruction_id: "obstruction.wall".into(),
            resolution_kind: ResolutionKind::Bypass,
            applicable_when: PredicateExpression::True,
            operations: Vec::new(),
            evidence: evidence(TruthStatus::Hypothetical),
        });
        catalog.validate().unwrap();
        assert_eq!(catalog.obstructions.len(), 1);
        assert_eq!(catalog.resolvers[0].obstruction_id, "obstruction.wall");
    }

    #[test]
    fn dialogue_interruption_names_window_flow_and_cleanup_operations() {
        let trace = WitnessedMicrotrace {
            id: "microtrace.auru-sidehop".into(),
            scope: scope(),
            precondition: PredicateExpression::True,
            operations: vec![
                StateOperation::AdvanceFlow {
                    flow_component_id: "flow.auru".into(),
                    node_id: "node.item".into(),
                },
                StateOperation::CancelCleanup {
                    cleanup_id: "cleanup.message-progress".into(),
                },
                StateOperation::Interrupt {
                    action_id: "dialogue.auru".into(),
                    window: TemporalWindow {
                        earliest_frame: 0,
                        latest_frame: 0,
                        required_input: Some("sidehop".into()),
                    },
                },
            ],
            postcondition: PredicateExpression::Fact {
                fact_id: "message.temporary-item-state-held".into(),
            },
            timing: TemporalWindow {
                earliest_frame: 0,
                latest_frame: 0,
                required_input: Some("sidehop".into()),
            },
            evidence: evidence(TruthStatus::Established),
        };
        validate_microtrace(&trace).unwrap();
    }

    #[test]
    fn actor_reconstruction_consumes_persisted_state_explicitly() {
        let mut catalog = locked_door_catalog();
        catalog.reconstruction_rules.push(ActorReconstructionRule {
            id: "reconstruct.forest-door".into(),
            label: "Reconstruct the Forest Temple door actor".into(),
            scope: scope(),
            actor_type: "obj_door".into(),
            instantiate_when: PredicateExpression::Fact {
                fact_id: "world.forest-door.placed-on-layer".into(),
            },
            initialization_operations: vec![StateOperation::Write {
                target: ComponentFieldTarget {
                    component_id: "actor.forest-door/live".into(),
                    field: "opened".into(),
                },
                value: StateValue::Boolean(false),
            }],
            evidence: evidence(TruthStatus::Established),
        });
        catalog.validate().unwrap();
        assert_eq!(catalog.reconstruction_rules.len(), 1);
    }

    #[test]
    fn projection_requires_an_explicit_component_set() {
        let operation = StateOperation::Project {
            source_runtime_file_id: "file-0".into(),
            destination_runtime_file_id: "slot-1-runtime".into(),
            component_ids: Vec::new(),
        };
        assert_eq!(
            operation.validate().unwrap_err().field(),
            "operation.component_ids"
        );
    }
}
