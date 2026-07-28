//! Exact GZ2E01 reset-to-opening and opening-phase initialization mechanics.
//!
//! Later title input, name/file-select, slot-load, void, and death branches
//! remain separate audit targets.

use crate::PlannerContractError;
use crate::artifact::Digest;
use crate::identity::{ContentIdentity, ContextSelector, ExactContext, RuntimeConfiguration};
use crate::logic::{
    ComparisonOperator, ContextScope, EvidenceKind, EvidenceRecord, PredicateExpression,
    RuleEvidence, TruthStatus, ValueReference,
};
use crate::return_place::{GZ2E01_CONTENT_SHA256, GZ2E01_EN_RUNTIME_SHA256};
use crate::state::{
    BackingAttachment, ComponentBinding, ComponentKind, ComponentPayload, ComponentProvenance,
    ComponentSelector, ExecutionContext, PhysicalSlotId, ProvenanceSourceKind, RuntimeFileOrigin,
    SceneLocation, SemanticLifetime, SerializationOwner, StateComponent, StateValue,
};
use crate::transition::{
    ActivationContract, CandidateTransition, ComponentFieldTarget, Goal, MECHANICS_CATALOG_SCHEMA,
    MechanicsCatalog, SaveProjectionOperation, StateOperation, TransitionKind,
};
use std::collections::BTreeMap;

const RESET_CONTROL_COMPONENT: &str = "reset-control";
const RESTART_COMPONENT: &str = "restart";
const OPENING_PROCESS_CONTROL_COMPONENT: &str = "opening-process-control";
const TITLE_CONTROL_COMPONENT: &str = "title-control";
const NAME_SCENE_CONTROL_COMPONENT: &str = "name-scene-control";
const SAVE_MENU_CONTROL_COMPONENT: &str = "save-menu-control";
const RUNTIME_FILE_HEADER_COMPONENT: &str = "runtime-file.header";
const PERSISTENT_EVENT_COMPONENT: &str = "flags.persistent-event-registers";
const OBSERVED_EVENT_COMPONENT: &str = "flags.event";
const LIGHT_DROP_COMPONENT: &str = "save.player-light-drop";
const PLAYER_INFO_COMPONENT: &str = "save.player-info";
const OBSERVED_TEMPORARY_COMPONENT: &str = "flags.temporary";
const TEMPORARY_EVENT_COMPONENT: &str = "flags.temporary-event-registers";
const DUNGEON_SESSION_LABEL_COMPONENT: &str = "flags.dungeon-session-labels";
const LOADED_STAGE_MEMORY_COMPONENT: &str = "flags.loaded-stage-memory";
const DUNGEON_SIX_SAVE_COMPONENT: &str = "save.dungeon-memory.index-6";
const ROOM_SWITCH_LABEL_COMPONENT: &str = "flags.room-switch-labels";
const INVENTORY_COMPONENT: &str = "inventory-and-resources";
const RETURN_PLACE_COMPONENT: &str = "return-place";
const ACTIVE_VIBRATION_COMPONENT: &str = "session.active-vibration";
const SAVE_STAGE_DISPLAY_COMPONENT: &str = "session.save-stage-display";
const FILE_SELECT_BUFFER_OWNER_PREFIX: &str = "file-select-buffer.slot";
pub const GZ2E01_UNSAVED_FILE_ZERO_GOAL_ID: &str = "goal.gz2e01.unsaved-file-zero-world-active";

const ITEM_NONE: u8 = 0xff;
const ITEM_HOOKSHOT: u8 = 0x44;
const ITEM_DOUBLE_CLAWSHOT: u8 = 0x47;
const ITEM_LINEUP_ORDER: [u8; 23] = [
    10, 8, 6, 2, 9, 4, 3, 0, 1, 23, 20, 5, 15, 16, 17, 11, 12, 13, 14, 19, 18, 22, 21,
];
const DEFAULT_PLAYER_NAME_BYTES: &[u8] = b"Link\0";
const DEFAULT_HORSE_NAME_BYTES: &[u8] = b"Epona\0";

/// Compiles the exact successful prefix of `dComIfG_resetToOpening` for
/// GZ2E01. It records the scheduled opening process/load and the restart-room
/// parameter write without pretending that the pending F_SP102 request is an
/// already loaded, traversable world location.
mod catalog;
mod evidence;
mod initialization;
mod transitions;

pub use catalog::gz2e01_reset_to_opening_mechanics;

use evidence::*;
use initialization::*;
use transitions::*;

#[cfg(test)]
mod tests;
