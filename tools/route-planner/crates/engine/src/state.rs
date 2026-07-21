//! Typed execution environments and independently owned state components.

use crate::artifact::Digest;
use crate::identity::{RuntimeConfiguration, require_digest};
use crate::{PlannerContractError, canonical_json, validate_label, validate_stable_id};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const EXECUTION_ENVIRONMENT_SCHEMA: &str = "dusklight.route-planner.execution-environment/v5";
pub const BOUNDARY_POLICY_SCHEMA: &str = "dusklight.route-planner.boundary-policy/v2";
pub const MAX_COMPONENT_BYTES: usize = 1024 * 1024;
pub const MAX_STATE_COLLECTION: usize = 65_536;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct PhysicalSlotId(pub u8);

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RuntimeFileOrigin {
    TitleFile0,
    NewFile,
    LoadedSlot { slot: PhysicalSlotId },
    Unknown,
    Other { id: String },
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BackingAttachment {
    MemoryOnly,
    CardBacked { slot: PhysicalSlotId },
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeFileLifecycle {
    Active,
    Suspended,
    Ended,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeFile {
    pub id: String,
    pub origin: RuntimeFileOrigin,
    pub backing: BackingAttachment,
    pub allowed_serialization_targets: Vec<PhysicalSlotId>,
    pub lifecycle: RuntimeFileLifecycle,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PhysicalSlot {
    pub slot: PhysicalSlotId,
    pub persistent_file_id: String,
    pub serialized_state_sha256: Digest,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureStatus {
    NotSampled,
    Present,
    Absent,
    Unavailable,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PhysicalSlotObservation {
    pub slot: PhysicalSlotId,
    pub content_status: CaptureStatus,
    pub attached_to_active_runtime: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SceneLocation {
    pub stage: String,
    pub room: i8,
    pub layer: i8,
    pub spawn: i16,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlayerForm {
    Human,
    Wolf,
    Unknown,
    Other { id: String },
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlayerMount {
    Epona,
    Boar,
    Unknown,
    Other { id: String },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerState {
    pub form: PlayerForm,
    pub mount: Option<PlayerMount>,
    pub position: [f32; 3],
    pub rotation: [i16; 3],
    pub has_control: Option<bool>,
    pub action: String,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ComponentKind {
    PersistentSave,
    StageMemory,
    ZoneMemory,
    DungeonMemory,
    TemporaryFlags,
    Restart,
    Inventory,
    ActorInstance,
    MessageFlow,
    PendingOperation,
    Session,
    Title,
    Custom { id: String },
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ComponentBinding {
    Global,
    Stage { stage: String },
    Room { stage: String, room: i8 },
    Zone { stage: String, zone: i16 },
    Dungeon { dungeon: String },
    RuntimeFile { runtime_file_id: String },
    Actor { instance_id: String },
    Session { session_id: String },
    Unbound,
    Custom { kind_id: String, context_id: String },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticLifetime {
    Frame,
    Action,
    RoomLoad,
    StageLoad,
    Session,
    RuntimeFile,
    SaveSerialization,
    PhysicalSlot,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SerializationOwner {
    None,
    RuntimeFile {
        runtime_file_id: String,
    },
    PhysicalSlot {
        slot: PhysicalSlotId,
    },
    StageBank {
        runtime_file_id: String,
        stage: String,
    },
    Custom {
        id: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum StateValue {
    Boolean(bool),
    Signed(i64),
    Unsigned(u64),
    Text(String),
    Bytes(Vec<u8>),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ComponentPayload {
    Raw {
        bytes: Vec<u8>,
        known_mask: Vec<u8>,
    },
    Structured {
        fields: BTreeMap<String, StateValue>,
    },
    Unknown {
        expected_bytes: Option<u32>,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProvenanceSourceKind {
    Initialized,
    Transition,
    SaveRestore,
    TraceObservation,
    ExtractedFact,
    Theorycraft,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentProvenance {
    pub source_kind: ProvenanceSourceKind,
    pub source_id: String,
    pub source_sha256: Option<Digest>,
    pub transition_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StateComponent {
    pub id: String,
    pub component_kind: ComponentKind,
    pub payload: ComponentPayload,
    pub binding: ComponentBinding,
    pub lifetime: SemanticLifetime,
    pub serialization_owner: SerializationOwner,
    pub provenance: Vec<ComponentProvenance>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StaticWorldObject {
    pub id: String,
    pub actor_type: String,
    pub placement_sha256: Digest,
    pub binding: ComponentBinding,
    pub parameters: BTreeMap<String, StateValue>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SpatialVolumeShape {
    AxisAlignedBox {
        minimum: [f32; 3],
        maximum: [f32; 3],
    },
    Sphere {
        center: [f32; 3],
        radius: f32,
    },
    VerticalCylinder {
        center_xz: [f32; 2],
        minimum_y: f32,
        maximum_y: f32,
        radius: f32,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SpatialVolume {
    pub object_id: String,
    pub volume_id: String,
    pub shape: SpatialVolumeShape,
    pub source_sha256: Digest,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SpatialConnectionStatus {
    Traversable,
    Blocked,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SpatialConnection {
    pub approach_id: String,
    pub source_region_id: String,
    pub destination_region_id: String,
    pub status: SpatialConnectionStatus,
    pub source_sha256: Digest,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SpatialPlane {
    pub plane_id: String,
    /// Plane equation: `normal dot world_position + offset`.
    pub normal: [f32; 3],
    pub offset: f32,
    pub source_sha256: Digest,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaneRelation {
    Positive,
    NonNegative,
    Negative,
    NonPositive,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PersistedObjectControl {
    pub object_id: String,
    pub fields: BTreeMap<String, StateValue>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorLifecycle {
    Loaded,
    Unloading,
    Unloaded,
    Destroyed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LiveWorldObject {
    pub instance_id: String,
    pub static_object_id: Option<String>,
    pub actor_type: String,
    pub lifecycle: ActorLifecycle,
    pub fields: BTreeMap<String, StateValue>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionEnvironment {
    pub schema: String,
    pub runtime_configuration: RuntimeConfiguration,
    pub active_runtime_file: RuntimeFile,
    pub physical_slots: Vec<PhysicalSlot>,
    pub physical_slot_observations: Vec<PhysicalSlotObservation>,
    pub location: SceneLocation,
    pub player: PlayerState,
    pub components: Vec<StateComponent>,
    pub static_world_objects: Vec<StaticWorldObject>,
    pub spatial_volumes: Vec<SpatialVolume>,
    pub spatial_connections: Vec<SpatialConnection>,
    pub spatial_planes: Vec<SpatialPlane>,
    pub persisted_object_controls: Vec<PersistedObjectControl>,
    pub live_world_objects: Vec<LiveWorldObject>,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BoundaryKind {
    RoomTransition,
    StageTransition,
    VoidReload,
    SaveWarp,
    TitleReturn,
    ProcessRestart,
    LoadPhysicalSlot,
    SaveRuntimeToSlot,
    WrongStateRespawn,
    DialogueInterruption,
    Custom { id: String },
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ComponentSelector {
    Id { component_id: String },
    Kind { component_kind: ComponentKind },
    Binding { binding: ComponentBinding },
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BoundaryDisposition {
    Preserve,
    Clear,
    Reinitialize { initializer_id: String },
    Serialize { owner: SerializationOwner },
    Restore { owner: SerializationOwner },
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentBoundaryRule {
    pub selector: ComponentSelector,
    pub disposition: BoundaryDisposition,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BoundaryPolicy {
    pub schema: String,
    pub id: String,
    pub boundary: BoundaryKind,
    pub default_disposition: BoundaryDisposition,
    pub component_rules: Vec<ComponentBoundaryRule>,
}

impl PhysicalSlotId {
    pub fn validate(self, field: &str) -> Result<(), PlannerContractError> {
        if !(1..=3).contains(&self.0) {
            return Err(PlannerContractError::new(
                field,
                "must name physical slot 1, 2, or 3",
            ));
        }
        Ok(())
    }
}

impl RuntimeFile {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        validate_stable_id("active_runtime_file.id", &self.id)?;
        validate_runtime_origin(&self.origin)?;
        validate_backing(&self.backing)?;
        let mut previous = None;
        for slot in &self.allowed_serialization_targets {
            slot.validate("allowed_serialization_targets")?;
            if previous.is_some_and(|prior: PhysicalSlotId| prior >= *slot) {
                return Err(PlannerContractError::new(
                    "allowed_serialization_targets",
                    "must be unique and sorted",
                ));
            }
            previous = Some(*slot);
        }
        Ok(())
    }
}

impl SceneLocation {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        validate_location(self)
    }
}

impl StateComponent {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        validate_stable_id("components.id", &self.id)?;
        validate_component_kind(&self.component_kind)?;
        validate_binding(&self.binding)?;
        validate_serialization_owner(&self.serialization_owner)?;
        validate_payload(&self.payload)?;
        if self.provenance.is_empty() || self.provenance.len() > 1_024 {
            return Err(PlannerContractError::new(
                "components.provenance",
                "must contain between 1 and 1024 records",
            ));
        }
        for provenance in &self.provenance {
            validate_stable_id("components.provenance.source_id", &provenance.source_id)?;
            if provenance.source_sha256 == Some(Digest::ZERO) {
                return Err(PlannerContractError::new(
                    "components.provenance.source_sha256",
                    "must be absent or nonzero",
                ));
            }
            if let Some(transition_id) = &provenance.transition_id {
                validate_stable_id("components.provenance.transition_id", transition_id)?;
            }
        }
        Ok(())
    }
}

impl ExecutionEnvironment {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != EXECUTION_ENVIRONMENT_SCHEMA {
            return Err(PlannerContractError::new("schema", "is unsupported"));
        }
        self.runtime_configuration.validate()?;
        self.active_runtime_file.validate()?;
        validate_location(&self.location)?;
        validate_player(&self.player)?;
        validate_sorted_collection(
            "physical_slots",
            &self.physical_slots,
            |slot| slot.slot.0,
            |slot| {
                slot.slot.validate("physical_slots.slot")?;
                validate_stable_id(
                    "physical_slots.persistent_file_id",
                    &slot.persistent_file_id,
                )?;
                require_digest(
                    "physical_slots.serialized_state_sha256",
                    slot.serialized_state_sha256,
                )
            },
        )?;
        validate_sorted_collection(
            "physical_slot_observations",
            &self.physical_slot_observations,
            |slot| slot.slot.0,
            |slot| slot.slot.validate("physical_slot_observations.slot"),
        )?;
        validate_sorted_collection(
            "components",
            &self.components,
            |component| component.id.clone(),
            StateComponent::validate,
        )?;
        validate_sorted_collection(
            "static_world_objects",
            &self.static_world_objects,
            |object| object.id.clone(),
            validate_static_object,
        )?;
        validate_sorted_collection(
            "spatial_volumes",
            &self.spatial_volumes,
            |volume| (volume.object_id.clone(), volume.volume_id.clone()),
            validate_spatial_volume,
        )?;
        validate_sorted_collection(
            "spatial_connections",
            &self.spatial_connections,
            |connection| {
                (
                    connection.approach_id.clone(),
                    connection.source_region_id.clone(),
                    connection.destination_region_id.clone(),
                )
            },
            validate_spatial_connection,
        )?;
        validate_sorted_collection(
            "spatial_planes",
            &self.spatial_planes,
            |plane| plane.plane_id.clone(),
            validate_spatial_plane,
        )?;
        validate_sorted_collection(
            "persisted_object_controls",
            &self.persisted_object_controls,
            |control| control.object_id.clone(),
            validate_persisted_control,
        )?;
        validate_sorted_collection(
            "live_world_objects",
            &self.live_world_objects,
            |object| object.instance_id.clone(),
            validate_live_object,
        )
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PlannerContractError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn digest(&self) -> Result<Digest, PlannerContractError> {
        Ok(Digest(Sha256::digest(self.canonical_bytes()?).into()))
    }
}

impl BoundaryPolicy {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != BOUNDARY_POLICY_SCHEMA {
            return Err(PlannerContractError::new("schema", "is unsupported"));
        }
        validate_stable_id("id", &self.id)?;
        validate_boundary_kind(&self.boundary)?;
        validate_disposition(&self.default_disposition)?;
        if self.component_rules.len() > 4_096 {
            return Err(PlannerContractError::new(
                "component_rules",
                "must contain at most 4096 rules",
            ));
        }
        let mut selectors = BTreeSet::new();
        for rule in &self.component_rules {
            validate_selector(&rule.selector)?;
            validate_disposition(&rule.disposition)?;
            if !selectors.insert(&rule.selector) {
                return Err(PlannerContractError::new(
                    "component_rules",
                    "contains a duplicate selector",
                ));
            }
        }
        Ok(())
    }
}

fn validate_runtime_origin(origin: &RuntimeFileOrigin) -> Result<(), PlannerContractError> {
    match origin {
        RuntimeFileOrigin::LoadedSlot { slot } => slot.validate("runtime_file.origin.slot"),
        RuntimeFileOrigin::Other { id } => validate_stable_id("runtime_file.origin.id", id),
        RuntimeFileOrigin::TitleFile0 | RuntimeFileOrigin::NewFile | RuntimeFileOrigin::Unknown => {
            Ok(())
        }
    }
}

fn validate_backing(backing: &BackingAttachment) -> Result<(), PlannerContractError> {
    match backing {
        BackingAttachment::CardBacked { slot } => slot.validate("runtime_file.backing.slot"),
        BackingAttachment::MemoryOnly | BackingAttachment::Unknown => Ok(()),
    }
}

fn validate_location(location: &SceneLocation) -> Result<(), PlannerContractError> {
    validate_game_name("location.stage", &location.stage)
}

fn validate_player(player: &PlayerState) -> Result<(), PlannerContractError> {
    if !player
        .position
        .iter()
        .all(|value| value.is_finite() && value.to_bits() != (-0.0_f32).to_bits())
    {
        return Err(PlannerContractError::new(
            "player.position",
            "must contain finite canonical coordinates",
        ));
    }
    validate_label("player.action", &player.action)?;
    validate_player_form(&player.form)?;
    if let Some(mount) = &player.mount {
        validate_player_mount(mount)?;
    }
    Ok(())
}

fn validate_player_form(form: &PlayerForm) -> Result<(), PlannerContractError> {
    match form {
        PlayerForm::Other { id } => validate_stable_id("player.form.id", id),
        PlayerForm::Human | PlayerForm::Wolf | PlayerForm::Unknown => Ok(()),
    }
}

fn validate_player_mount(mount: &PlayerMount) -> Result<(), PlannerContractError> {
    match mount {
        PlayerMount::Other { id } => validate_stable_id("player.mount.id", id),
        PlayerMount::Epona | PlayerMount::Boar | PlayerMount::Unknown => Ok(()),
    }
}

pub(crate) fn validate_component_kind(kind: &ComponentKind) -> Result<(), PlannerContractError> {
    match kind {
        ComponentKind::Custom { id } => validate_stable_id("component_kind.id", id),
        _ => Ok(()),
    }
}

pub(crate) fn validate_binding(binding: &ComponentBinding) -> Result<(), PlannerContractError> {
    match binding {
        ComponentBinding::Stage { stage }
        | ComponentBinding::Room { stage, .. }
        | ComponentBinding::Zone { stage, .. } => validate_game_name("binding.stage", stage),
        ComponentBinding::Dungeon { dungeon } => validate_stable_id("binding.dungeon", dungeon),
        ComponentBinding::RuntimeFile { runtime_file_id } => {
            validate_stable_id("binding.runtime_file_id", runtime_file_id)
        }
        ComponentBinding::Actor { instance_id } => {
            validate_stable_id("binding.instance_id", instance_id)
        }
        ComponentBinding::Session { session_id } => {
            validate_stable_id("binding.session_id", session_id)
        }
        ComponentBinding::Custom {
            kind_id,
            context_id,
        } => {
            validate_stable_id("binding.kind_id", kind_id)?;
            validate_stable_id("binding.context_id", context_id)
        }
        ComponentBinding::Global | ComponentBinding::Unbound => Ok(()),
    }
}

pub(crate) fn validate_serialization_owner(
    owner: &SerializationOwner,
) -> Result<(), PlannerContractError> {
    match owner {
        SerializationOwner::RuntimeFile { runtime_file_id } => {
            validate_stable_id("serialization_owner.runtime_file_id", runtime_file_id)
        }
        SerializationOwner::PhysicalSlot { slot } => slot.validate("serialization_owner.slot"),
        SerializationOwner::StageBank {
            runtime_file_id,
            stage,
        } => {
            validate_stable_id("serialization_owner.runtime_file_id", runtime_file_id)?;
            validate_game_name("serialization_owner.stage", stage)
        }
        SerializationOwner::Custom { id } => validate_stable_id("serialization_owner.id", id),
        SerializationOwner::None => Ok(()),
    }
}

fn validate_payload(payload: &ComponentPayload) -> Result<(), PlannerContractError> {
    match payload {
        ComponentPayload::Raw { bytes, known_mask } => {
            if bytes.len() > MAX_COMPONENT_BYTES || bytes.len() != known_mask.len() {
                return Err(PlannerContractError::new(
                    "components.payload",
                    "raw bytes and known mask must have equal length up to 1 MiB",
                ));
            }
        }
        ComponentPayload::Structured { fields } => validate_state_fields(fields)?,
        ComponentPayload::Unknown { expected_bytes } => {
            if expected_bytes.is_some_and(|size| size as usize > MAX_COMPONENT_BYTES) {
                return Err(PlannerContractError::new(
                    "components.payload.expected_bytes",
                    "must be at most 1 MiB",
                ));
            }
        }
    }
    Ok(())
}

fn validate_state_fields(
    fields: &BTreeMap<String, StateValue>,
) -> Result<(), PlannerContractError> {
    if fields.len() > 16_384 {
        return Err(PlannerContractError::new(
            "fields",
            "must contain at most 16384 entries",
        ));
    }
    for (key, value) in fields {
        validate_stable_id("fields key", key)?;
        match value {
            StateValue::Text(value) => validate_label("fields text", value)?,
            StateValue::Bytes(value) if value.len() > MAX_COMPONENT_BYTES => {
                return Err(PlannerContractError::new(
                    "fields bytes",
                    "must contain at most 1 MiB",
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

pub(crate) fn validate_static_object(
    object: &StaticWorldObject,
) -> Result<(), PlannerContractError> {
    validate_stable_id("static_world_objects.id", &object.id)?;
    validate_stable_id("static_world_objects.actor_type", &object.actor_type)?;
    require_digest(
        "static_world_objects.placement_sha256",
        object.placement_sha256,
    )?;
    validate_binding(&object.binding)?;
    validate_state_fields(&object.parameters)
}

fn validate_spatial_volume(volume: &SpatialVolume) -> Result<(), PlannerContractError> {
    validate_stable_id("spatial_volumes.object_id", &volume.object_id)?;
    validate_stable_id("spatial_volumes.volume_id", &volume.volume_id)?;
    require_digest("spatial_volumes.source_sha256", volume.source_sha256)?;
    match &volume.shape {
        SpatialVolumeShape::AxisAlignedBox { minimum, maximum } => {
            if !canonical_floats(minimum.iter().chain(maximum))
                || minimum
                    .iter()
                    .zip(maximum)
                    .any(|(minimum, maximum)| minimum > maximum)
            {
                return Err(PlannerContractError::new(
                    "spatial_volumes.shape",
                    "must contain finite canonical bounds with minimum <= maximum",
                ));
            }
            Ok(())
        }
        SpatialVolumeShape::Sphere { center, radius } => {
            if !canonical_floats(center.iter().chain(std::iter::once(radius))) || *radius <= 0.0 {
                return Err(PlannerContractError::new(
                    "spatial_volumes.shape",
                    "sphere must contain finite canonical coordinates and a positive radius",
                ));
            }
            Ok(())
        }
        SpatialVolumeShape::VerticalCylinder {
            center_xz,
            minimum_y,
            maximum_y,
            radius,
        } => {
            if !canonical_floats(center_xz.iter().chain([minimum_y, maximum_y, radius]))
                || minimum_y > maximum_y
                || *radius <= 0.0
            {
                return Err(PlannerContractError::new(
                    "spatial_volumes.shape",
                    "cylinder must contain finite canonical ordered bounds and a positive radius",
                ));
            }
            Ok(())
        }
    }
}

fn validate_spatial_connection(connection: &SpatialConnection) -> Result<(), PlannerContractError> {
    validate_stable_id("spatial_connections.approach_id", &connection.approach_id)?;
    validate_stable_id(
        "spatial_connections.source_region_id",
        &connection.source_region_id,
    )?;
    validate_stable_id(
        "spatial_connections.destination_region_id",
        &connection.destination_region_id,
    )?;
    require_digest(
        "spatial_connections.source_sha256",
        connection.source_sha256,
    )
}

fn validate_spatial_plane(plane: &SpatialPlane) -> Result<(), PlannerContractError> {
    validate_stable_id("spatial_planes.plane_id", &plane.plane_id)?;
    require_digest("spatial_planes.source_sha256", plane.source_sha256)?;
    if !canonical_floats(plane.normal.iter().chain(std::iter::once(&plane.offset)))
        || plane.normal.iter().all(|component| *component == 0.0)
    {
        return Err(PlannerContractError::new(
            "spatial_planes",
            "must contain a finite canonical equation with a nonzero normal",
        ));
    }
    Ok(())
}

fn canonical_floats<'a>(values: impl IntoIterator<Item = &'a f32>) -> bool {
    values
        .into_iter()
        .all(|value| value.is_finite() && value.to_bits() != (-0.0_f32).to_bits())
}

fn validate_persisted_control(
    control: &PersistedObjectControl,
) -> Result<(), PlannerContractError> {
    validate_stable_id("persisted_object_controls.object_id", &control.object_id)?;
    validate_state_fields(&control.fields)
}

fn validate_live_object(object: &LiveWorldObject) -> Result<(), PlannerContractError> {
    validate_stable_id("live_world_objects.instance_id", &object.instance_id)?;
    if let Some(static_object_id) = &object.static_object_id {
        validate_stable_id("live_world_objects.static_object_id", static_object_id)?;
    }
    validate_stable_id("live_world_objects.actor_type", &object.actor_type)?;
    validate_state_fields(&object.fields)
}

fn validate_boundary_kind(boundary: &BoundaryKind) -> Result<(), PlannerContractError> {
    match boundary {
        BoundaryKind::Custom { id } => validate_stable_id("boundary.id", id),
        _ => Ok(()),
    }
}

fn validate_selector(selector: &ComponentSelector) -> Result<(), PlannerContractError> {
    match selector {
        ComponentSelector::Id { component_id } => {
            validate_stable_id("selector.component_id", component_id)
        }
        ComponentSelector::Kind { component_kind } => validate_component_kind(component_kind),
        ComponentSelector::Binding { binding } => validate_binding(binding),
    }
}

fn validate_disposition(disposition: &BoundaryDisposition) -> Result<(), PlannerContractError> {
    match disposition {
        BoundaryDisposition::Reinitialize { initializer_id } => {
            validate_stable_id("disposition.initializer_id", initializer_id)
        }
        BoundaryDisposition::Serialize { owner } | BoundaryDisposition::Restore { owner } => {
            validate_serialization_owner(owner)
        }
        BoundaryDisposition::Preserve
        | BoundaryDisposition::Clear
        | BoundaryDisposition::Unknown => Ok(()),
    }
}

fn validate_game_name(field: &str, value: &str) -> Result<(), PlannerContractError> {
    if value.is_empty()
        || value.len() > 32
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(PlannerContractError::new(
            field,
            "must be 1-32 ASCII letters, digits, '_' or '-'",
        ));
    }
    Ok(())
}

fn validate_sorted_collection<T, K: Ord>(
    field: &str,
    values: &[T],
    key: impl Fn(&T) -> K,
    validate: impl Fn(&T) -> Result<(), PlannerContractError>,
) -> Result<(), PlannerContractError> {
    if values.len() > MAX_STATE_COLLECTION {
        return Err(PlannerContractError::new(
            field,
            "contains too many records",
        ));
    }
    let mut previous = None;
    for value in values {
        validate(value)?;
        let current = key(value);
        if previous.as_ref().is_some_and(|prior| prior >= &current) {
            return Err(PlannerContractError::new(
                field,
                "must be unique and sorted canonically",
            ));
        }
        previous = Some(current);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::RUNTIME_CONFIGURATION_SCHEMA;

    fn provenance(source_id: &str) -> ComponentProvenance {
        ComponentProvenance {
            source_kind: ProvenanceSourceKind::Initialized,
            source_id: source_id.into(),
            source_sha256: Some(Digest([9; 32])),
            transition_id: None,
        }
    }

    fn component(id: &str, kind: ComponentKind) -> StateComponent {
        StateComponent {
            id: id.into(),
            component_kind: kind,
            payload: ComponentPayload::Structured {
                fields: BTreeMap::new(),
            },
            binding: ComponentBinding::RuntimeFile {
                runtime_file_id: "file-0".into(),
            },
            lifetime: SemanticLifetime::RuntimeFile,
            serialization_owner: SerializationOwner::RuntimeFile {
                runtime_file_id: "file-0".into(),
            },
            provenance: vec![provenance("title-init")],
        }
    }

    fn file_zero_environment() -> ExecutionEnvironment {
        ExecutionEnvironment {
            schema: EXECUTION_ENVIRONMENT_SCHEMA.into(),
            runtime_configuration: RuntimeConfiguration {
                schema: RUNTIME_CONFIGURATION_SCHEMA.into(),
                content_sha256: Digest([1; 32]),
                language: "en".into(),
                settings: BTreeMap::new(),
            },
            active_runtime_file: RuntimeFile {
                id: "file-0".into(),
                origin: RuntimeFileOrigin::TitleFile0,
                backing: BackingAttachment::MemoryOnly,
                allowed_serialization_targets: vec![
                    PhysicalSlotId(1),
                    PhysicalSlotId(2),
                    PhysicalSlotId(3),
                ],
                lifecycle: RuntimeFileLifecycle::Active,
            },
            physical_slots: Vec::new(),
            physical_slot_observations: Vec::new(),
            location: SceneLocation {
                stage: "F_SP103".into(),
                room: 0,
                layer: 0,
                spawn: 0,
            },
            player: PlayerState {
                form: PlayerForm::Human,
                mount: None,
                position: [0.0, 1.0, 2.0],
                rotation: [0, 0, 0],
                has_control: Some(true),
                action: "idle".into(),
            },
            components: vec![
                component("inventory", ComponentKind::Inventory),
                component("progress", ComponentKind::PersistentSave),
            ],
            static_world_objects: Vec::new(),
            spatial_volumes: Vec::new(),
            spatial_connections: Vec::new(),
            spatial_planes: Vec::new(),
            persisted_object_controls: Vec::new(),
            live_world_objects: Vec::new(),
        }
    }

    #[test]
    fn file_zero_is_memory_backed_but_can_hold_persistent_domain_components() {
        let environment = file_zero_environment();
        environment.validate().unwrap();
        assert_eq!(
            environment.active_runtime_file.backing,
            BackingAttachment::MemoryOnly
        );
        assert!(
            environment
                .components
                .iter()
                .any(|component| component.component_kind == ComponentKind::PersistentSave)
        );
        assert_ne!(environment.digest().unwrap(), Digest::ZERO);
    }

    #[test]
    fn slot_zero_and_duplicate_component_ids_fail_closed() {
        let mut environment = file_zero_environment();
        environment
            .active_runtime_file
            .allowed_serialization_targets = vec![PhysicalSlotId(0)];
        assert_eq!(
            environment.validate().unwrap_err().field(),
            "allowed_serialization_targets"
        );

        let mut environment = file_zero_environment();
        environment
            .components
            .push(component("progress", ComponentKind::Inventory));
        assert_eq!(environment.validate().unwrap_err().field(), "components");
    }

    #[test]
    fn raw_unknown_mask_and_component_provenance_are_explicit() {
        let mut component = component("stage-memory", ComponentKind::StageMemory);
        component.payload = ComponentPayload::Raw {
            bytes: vec![0xaa, 0x55],
            known_mask: vec![0xff, 0x00],
        };
        component.provenance.push(ComponentProvenance {
            source_kind: ProvenanceSourceKind::Transition,
            source_id: "bite-splice".into(),
            source_sha256: None,
            transition_id: Some("technique.bite".into()),
        });
        component.validate().unwrap();

        if let ComponentPayload::Raw { known_mask, .. } = component.payload {
            assert_eq!(known_mask, vec![0xff, 0x00]);
        } else {
            panic!("expected raw component payload");
        }
    }

    #[test]
    fn actor_placement_persistence_and_live_instance_are_independent() {
        let mut environment = file_zero_environment();
        environment.static_world_objects.push(StaticWorldObject {
            id: "gate.ordon".into(),
            actor_type: "obj_gate".into(),
            placement_sha256: Digest([3; 32]),
            binding: ComponentBinding::Room {
                stage: "F_SP103".into(),
                room: 0,
            },
            parameters: BTreeMap::new(),
        });
        environment
            .persisted_object_controls
            .push(PersistedObjectControl {
                object_id: "gate.ordon".into(),
                fields: BTreeMap::from([("open".into(), StateValue::Boolean(true))]),
            });
        environment.live_world_objects.push(LiveWorldObject {
            instance_id: "gate.ordon/live/1".into(),
            static_object_id: Some("gate.ordon".into()),
            actor_type: "obj_gate".into(),
            lifecycle: ActorLifecycle::Unloaded,
            fields: BTreeMap::new(),
        });
        environment.validate().unwrap();
        assert_eq!(environment.static_world_objects.len(), 1);
        assert_eq!(environment.persisted_object_controls.len(), 1);
        assert_eq!(environment.live_world_objects.len(), 1);
    }

    #[test]
    fn spatial_volumes_require_canonical_ordered_evidenced_bounds() {
        let mut environment = file_zero_environment();
        environment.spatial_volumes.push(SpatialVolume {
            object_id: "actor.auru".into(),
            volume_id: "talk".into(),
            shape: SpatialVolumeShape::AxisAlignedBox {
                minimum: [-1.0, 0.0, -2.0],
                maximum: [1.0, 2.0, 2.0],
            },
            source_sha256: Digest([4; 32]),
        });
        environment.validate().unwrap();

        let mut invalid_bounds = environment.clone();
        invalid_bounds.spatial_volumes[0].shape = SpatialVolumeShape::AxisAlignedBox {
            minimum: [2.0, 0.0, 0.0],
            maximum: [1.0, 1.0, 1.0],
        };
        assert_eq!(
            invalid_bounds.validate().unwrap_err().field(),
            "spatial_volumes.shape"
        );

        let mut invalid_digest = environment;
        invalid_digest.spatial_volumes[0].source_sha256 = Digest::ZERO;
        assert_eq!(
            invalid_digest.validate().unwrap_err().field(),
            "spatial_volumes.source_sha256"
        );

        let mut invalid_sphere = file_zero_environment();
        invalid_sphere.spatial_volumes.push(SpatialVolume {
            object_id: "actor.auru".into(),
            volume_id: "talk".into(),
            shape: SpatialVolumeShape::Sphere {
                center: [0.0; 3],
                radius: 0.0,
            },
            source_sha256: Digest([4; 32]),
        });
        assert_eq!(
            invalid_sphere.validate().unwrap_err().field(),
            "spatial_volumes.shape"
        );
    }

    #[test]
    fn spatial_connections_and_planes_are_directional_and_evidenced() {
        let mut environment = file_zero_environment();
        environment.spatial_connections.push(SpatialConnection {
            approach_id: "approach.front".into(),
            source_region_id: "region.a".into(),
            destination_region_id: "region.b".into(),
            status: SpatialConnectionStatus::Blocked,
            source_sha256: Digest([5; 32]),
        });
        environment.spatial_planes.push(SpatialPlane {
            plane_id: "void.room-0".into(),
            normal: [0.0, 1.0, 0.0],
            offset: 0.0,
            source_sha256: Digest([6; 32]),
        });
        environment.validate().unwrap();

        let mut reverse = environment.clone();
        reverse.spatial_connections.push(SpatialConnection {
            approach_id: "approach.front".into(),
            source_region_id: "region.b".into(),
            destination_region_id: "region.a".into(),
            status: SpatialConnectionStatus::Traversable,
            source_sha256: Digest([7; 32]),
        });
        reverse.validate().unwrap();

        let mut invalid_plane = environment;
        invalid_plane.spatial_planes[0].normal = [0.0; 3];
        assert_eq!(
            invalid_plane.validate().unwrap_err().field(),
            "spatial_planes"
        );
    }

    #[test]
    fn boundary_policy_never_implicitly_preserves_unmentioned_components() {
        let policy = BoundaryPolicy {
            schema: BOUNDARY_POLICY_SCHEMA.into(),
            id: "boundary.stage-transition".into(),
            boundary: BoundaryKind::StageTransition,
            default_disposition: BoundaryDisposition::Unknown,
            component_rules: vec![ComponentBoundaryRule {
                selector: ComponentSelector::Kind {
                    component_kind: ComponentKind::PersistentSave,
                },
                disposition: BoundaryDisposition::Preserve,
            }],
        };
        policy.validate().unwrap();
        assert_eq!(policy.default_disposition, BoundaryDisposition::Unknown);

        let mut duplicate = policy;
        duplicate
            .component_rules
            .push(duplicate.component_rules[0].clone());
        assert_eq!(duplicate.validate().unwrap_err().field(), "component_rules");
    }
}
