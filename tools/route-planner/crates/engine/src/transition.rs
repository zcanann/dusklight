//! Causal transitions, activation obligations, mechanics, and techniques.

use crate::artifact::Digest;
use crate::logic::{ContextScope, EvidenceKind, PredicateExpression, RuleEvidence, ValueReference};
use crate::state::{
    BackingAttachment, ComponentBinding, ComponentBindingReference, ComponentKind,
    ComponentSelector, ExecutionContext, PhysicalSlotId, PlaneRelation, PlayerForm, PlayerMount,
    RuntimeFile, RuntimeFileLifecycle, RuntimeFileOrigin, SemanticLifetime, SerializationOwner,
    StateComponent, StateValue, validate_binding as validate_component_binding,
    validate_binding_reference, validate_component_kind, validate_serialization_owner,
    validate_state_fields,
};
use crate::{PlannerContractError, canonical_json, validate_label, validate_stable_id};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};

mod state_operations;
mod validation;
use validation::*;

pub const MECHANICS_CATALOG_SCHEMA: &str = "dusklight.route-planner.mechanics-catalog/v30";
pub const MAX_MECHANICS_RECORDS: usize = 65_536;

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentFieldTarget {
    pub component_id: String,
    pub field: String,
}

/// A deliberately bounded subset of state operations that may alter a private
/// runtime clone before it is sealed as a persistent image.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SaveProjectionOperation {
    Write {
        target: ComponentFieldTarget,
        value: StateValue,
    },
    WriteFields {
        component_id: String,
        fields: BTreeMap<String, StateValue>,
    },
    CopyValue {
        source: ComponentFieldTarget,
        target: ComponentFieldTarget,
    },
    WriteRaw {
        component_id: String,
        byte_offset: u32,
        mask: Vec<u8>,
        value: Vec<u8>,
    },
    WriteBytesField {
        target: ComponentFieldTarget,
        byte_offset: u32,
        mask: Vec<u8>,
        value: Vec<u8>,
    },
    InvalidateRaw {
        component_id: String,
        byte_offset: u32,
        mask: Vec<u8>,
    },
    InvalidateField {
        target: ComponentFieldTarget,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StateOperation {
    Write {
        target: ComponentFieldTarget,
        value: StateValue,
    },
    /// Atomically replaces several known fields on one structured component.
    /// This models one game writer whose record spans multiple scalar fields,
    /// such as a `Savmem` return-place update.
    WriteFields {
        component_id: String,
        fields: BTreeMap<String, StateValue>,
    },
    /// Replaces one live component's entire payload while retaining its
    /// identity, binding, lifetime, serialization owner, and provenance.
    ReplacePayload {
        component_id: String,
        payload: crate::state::ComponentPayload,
    },
    /// Invalidates every matching live payload and, when requested, matching
    /// serialized-store payloads owned by the active runtime file. Physical
    /// slot images and inactive runtime stores are never mutated.
    InvalidatePayloads {
        selector: ComponentSelector,
        include_active_runtime_serialized_stores: bool,
    },
    /// Invalidates matching serialized payloads owned by the active runtime
    /// without touching the corresponding live component. This models backing
    /// projections replaced by a file copy while live stage/temporary banks are
    /// outside that copy.
    InvalidateActiveRuntimeSerializedPayloads {
        selector: ComponentSelector,
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
    /// Writes masked bytes within a `StateValue::Bytes` structured field.
    WriteBytesField {
        target: ComponentFieldTarget,
        byte_offset: u32,
        mask: Vec<u8>,
        value: Vec<u8>,
    },
    WriteBoundRaw {
        component_kind: ComponentKind,
        binding: ComponentBindingReference,
        byte_offset: u32,
        mask: Vec<u8>,
        value: Vec<u8>,
    },
    InvalidateRaw {
        component_id: String,
        byte_offset: u32,
        mask: Vec<u8>,
    },
    InvalidateBoundRaw {
        component_kind: ComponentKind,
        binding: ComponentBindingReference,
        byte_offset: u32,
        mask: Vec<u8>,
    },
    Adjust {
        target: ComponentFieldTarget,
        delta: i64,
    },
    /// Subtracts an unsigned amount and clamps the result at zero. This is
    /// distinct from `Adjust`, whose underflow is an execution error.
    DebitUnsigned {
        target: ComponentFieldTarget,
        amount: u64,
    },
    /// Raises a known unsigned structured value to the supplied floor while
    /// preserving values already at or above it.
    ClampUnsignedMinimum {
        target: ComponentFieldTarget,
        minimum: u64,
    },
    /// Applies a two-slot item migration and rebuilds the deterministic lineup
    /// of occupied inventory-slot indices. Values are supplied by the exact
    /// content rule rather than embedded in the executor.
    NormalizeItemSlotsAndLineup {
        component_id: String,
        inventory_field: String,
        lineup_field: String,
        primary_slot: u8,
        secondary_slot: u8,
        single_item: u8,
        combined_item: u8,
        empty_item: u8,
        lineup_order: Vec<u8>,
    },
    AdjustBoundRawUnsigned {
        component_kind: ComponentKind,
        binding: ComponentBindingReference,
        byte_offset: u32,
        byte_width: u8,
        delta: i64,
    },
    ClearComponent {
        selector: ComponentSelector,
    },
    ClearField {
        target: ComponentFieldTarget,
    },
    /// Marks a structured field unobserved/unknown by removing its known value.
    /// This is distinct in provenance from a semantic clear performed by game
    /// logic, even though both leave no currently known structured value.
    InvalidateField {
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
    /// Replaces one explicit process/session-owned backing store with an exact
    /// authored component manifest. Physical-slot and runtime-file stores use
    /// their dedicated lifetime operations instead.
    ReplaceCustomStore {
        owner: SerializationOwner,
        components: Vec<StateComponent>,
    },
    /// Copies an exact custom-store payload manifest into same-ID live
    /// components while retaining the live components' ownership and binding.
    RestorePayloadsFromCustomStore {
        owner: SerializationOwner,
        component_ids: Vec<String>,
    },
    /// Commits the currently bound stage-local payload to its runtime-file-owned
    /// backing entry, then restores the destination stage's entry into the same
    /// live component. The execution engine checks all identities atomically.
    CommitLoadStageBank {
        component_id: String,
        runtime_file_id: String,
        source_stage: String,
        destination_stage: String,
        source_binding: ComponentBinding,
        destination_binding: ComponentBinding,
    },
    /// Restores one already-loaded stage-bank component into the live bank
    /// without committing a prior live stage. This is the `getSave(stage)` half
    /// used after a physical file load or new-file initialization.
    ActivateStageBank {
        component_id: String,
        runtime_file_id: String,
        stage: String,
        binding: ComponentBinding,
    },
    /// Projects the explicitly named runtime-file components and stage banks
    /// into a persistent file identity attached to a physical slot. The active
    /// runtime remains active until a separate load or lifecycle operation.
    SaveRuntimeToSlot {
        source_runtime_file_id: String,
        destination_slot: PhysicalSlotId,
        destination_persistent_file_id: String,
        runtime_component_ids: Vec<String>,
        stage_bank_stages: Vec<String>,
    },
    /// Active-runtime form of `save_runtime_to_slot`. The executor derives the
    /// persistent image ID from the active runtime ID and a stable suffix, and
    /// includes every available stage bank owned by that runtime, so an
    /// authored save-menu rule works after any prior load/lifetime handoff.
    SaveActiveRuntimeToSlot {
        destination_slot: PhysicalSlotId,
        destination_id_suffix: String,
        runtime_component_ids: Vec<String>,
        /// Ordered writes applied to a private runtime clone before its
        /// persistent image is sealed. They never mutate the live runtime.
        projection_operations: Vec<SaveProjectionOperation>,
    },
    /// Ends the current runtime-file lifetime, restores the exact persistent
    /// projection from a physical slot, explicitly carries selected non-card
    /// runtime metadata, and activates a new loaded runtime. Session-owned
    /// state is not part of the file projection and survives.
    LoadRuntimeFromSlot {
        source_runtime_file_id: String,
        source_slot: PhysicalSlotId,
        source_persistent_file_id: String,
        destination_runtime_file_id: String,
        destination_allowed_serialization_targets: Vec<PhysicalSlotId>,
        runtime_component_ids: Vec<String>,
        stage_bank_stages: Vec<String>,
        carried_runtime_component_ids: Vec<String>,
    },
    /// Active-runtime form of `load_runtime_from_slot`. The executor derives a
    /// fresh destination ID from the active runtime ID and a stable suffix,
    /// allowing authored mechanics to remain valid across nested file-0 and
    /// repeated load lifetimes without guessing an ephemeral runtime ID.
    LoadActiveRuntimeFromSlot {
        source_slot: PhysicalSlotId,
        destination_id_suffix: String,
        destination_allowed_serialization_targets: Vec<PhysicalSlotId>,
        carried_runtime_component_ids: Vec<String>,
    },
    /// Ends the active runtime-file lifetime, derives a fresh runtime ID from
    /// its old ID plus `destination_id_suffix`, and rekeys every live and
    /// serialized component owned by that lifetime. Physical file images and
    /// session/process state are not part of the handoff.
    BeginRuntimeFileLifetime {
        destination_id_suffix: String,
        origin: RuntimeFileOrigin,
        backing: BackingAttachment,
        allowed_serialization_targets: Vec<PhysicalSlotId>,
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
    SetExecutionContext {
        context: ExecutionContext,
    },
    /// Completes the pending world load while the non-world process remains
    /// active. This updates retained loaded-map state without authorizing
    /// traversal.
    CompletePendingWorldLoad,
    SetLocation {
        location: crate::state::SceneLocation,
    },
    /// Reads one structured backing record and changes map location only when
    /// its stage, room, and spawn fields are all known and well typed.
    SetLocationFromFields {
        component_id: String,
        stage_field: String,
        room_field: String,
        spawn_field: String,
        layer: i8,
    },
    /// Reads a structured return/next-stage record and attaches it to the
    /// currently active non-world process as a pending world load. This records
    /// a request without making the destination traversable.
    SetPendingWorldLoadFromFields {
        component_id: String,
        stage_field: String,
        room_field: String,
        spawn_field: String,
        layer: i8,
    },
    SetPlayerForm {
        form: PlayerForm,
    },
    SetPlayerMount {
        mount: Option<PlayerMount>,
    },
    SetPlayerControl {
        has_control: Option<bool>,
    },
    SetPlayerAction {
        action: String,
    },
    /// Recreates one live actor at a room-load boundary from its exact static
    /// placement, optional persisted control record, and audited initializer.
    /// Placement parameters are applied first, persisted fields second, and
    /// initializer fields last.
    ReconstructActor {
        static_object_id: String,
        instance_id: String,
        required_layer: i8,
        initialization_fields: BTreeMap<String, StateValue>,
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
    PortalWarp,
    SaveWarp,
    VoidReload,
    DeathReload,
    TitleReturn,
    WrongStateRespawn,
    ItemAcquisition,
    NpcReward,
    Cutscene,
    CutsceneSceneChange,
    ActorDriven,
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

impl CandidateTransition {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        validate_transition(self)
    }
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

/// The point in a candidate action at which a feasibility obligation must be
/// discharged. This prevents reachability, activation, committed effects, and
/// interruption timing from being treated as interchangeable evidence.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObligationStage {
    Reach,
    Activate,
    Effect,
    Interrupt,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VolumeReference {
    pub object_id: String,
    pub volume_id: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InteractionPosition {
    Player,
    PlayerAttention,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InteractionVolumeTest {
    pub position: InteractionPosition,
    pub volume: VolumeReference,
    pub must_be_inside: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InteractionBranch {
    pub when: PredicateExpression,
    pub volume_tests: Vec<InteractionVolumeTest>,
    pub pose_predicate: PredicateExpression,
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
    /// Form- or state-dependent interaction geometry where different observed
    /// points participate in one actor check. Branches are alternatives; every
    /// volume test inside the selected branch is conjunctive.
    CompoundInteraction {
        actor_instance_id: String,
        interaction_mode: String,
        branches: Vec<InteractionBranch>,
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
    /// Requires one observed 16-bit binary-angle yaw to be within the shortest
    /// circular distance of an authored target. Numeric less-than comparisons
    /// are not equivalent at the signed wrap boundary.
    Facing {
        yaw: ValueReference,
        target_yaw: i16,
        maximum_delta: u16,
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
    pub stage: ObligationStage,
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
    MaintainPredicate { predicate: PredicateExpression },
    RequireTransition { transition_id: String },
    ForbidTransition { transition_id: String },
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

#[cfg(test)]
#[path = "transition_tests.rs"]
mod tests;
