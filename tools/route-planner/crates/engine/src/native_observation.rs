//! Planner-owned, producer-neutral native observation boundary.
//!
//! Native runners may adapt their richer telemetry into this contract. The
//! planner does not link a TAS runner or its evidence store to inspect state.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeChannelStatus {
    #[default]
    NotSampled,
    Present,
    Absent,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativePhysicalSlotObservation {
    pub number: u8,
    pub content_status: NativeChannelStatus,
    pub attached_to_runtime: bool,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeRuntimeFileObservation {
    pub no_file_raw: u8,
    pub data_num_raw: u8,
    pub backing_attachment_status: NativeChannelStatus,
    pub attached_physical_slot: Option<u8>,
    pub physical_slots: [NativePhysicalSlotObservation; 3],
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeReturnPlaceObservation {
    pub stage: String,
    pub room: i8,
    pub player_status: u8,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeRestartObservation {
    pub room: i8,
    pub start_point: i16,
    pub angle_y: i16,
    pub position: [f32; 3],
    pub room_param: u32,
    pub last_speed: f32,
    pub last_mode: u32,
    pub last_angle_y: i16,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeActorIdentity {
    pub present: bool,
    pub runtime_generation: u32,
    pub actor_name: i16,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeMessageFlowObservation {
    pub flow_id: u16,
    pub node_index: u16,
    pub cut_name_hash: u32,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativePlayerControlObservation {
    pub mode_flags: u32,
    pub do_status: u8,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeEventHandoffObservation {
    pub pre_item_no: u8,
    pub get_item_no: u8,
    pub event_flags: u16,
    pub secondary_flags: u16,
    pub hind_flags: u16,
    pub talk_xy_type: u8,
    pub compulsory: u8,
    pub room_info_set: bool,
    pub skip_timer: i32,
    pub skip_parameter: i32,
    pub item_partner: NativeActorIdentity,
    pub event_name_status: NativeChannelStatus,
    pub event_name: Option<String>,
    pub message_flow_status: NativeChannelStatus,
    pub message_flow: Option<NativeMessageFlowObservation>,
    pub message_cut_status: NativeChannelStatus,
    pub pending_cleanup_status: NativeChannelStatus,
    pub pending_cleanup_flags: Option<u32>,
    pub player_control_status: NativeChannelStatus,
    pub player_control: Option<NativePlayerControlObservation>,
    pub no_telop_status: NativeChannelStatus,
    pub no_telop: Option<bool>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativePlayerResourcesObservation {
    pub maximum_life: u16,
    pub life: u16,
    pub rupees: u16,
    pub small_keys: u8,
    pub dungeon_map: bool,
    pub dungeon_compass: bool,
    pub dungeon_boss_key: bool,
    pub dungeon_warp: bool,
    pub inventory: [u8; 24],
    pub selected_items: [u8; 4],
    pub mixed_items: [u8; 4],
    pub equipment: [u8; 6],
    pub bomb_counts: [u8; 3],
    pub bomb_capacities: [u8; 3],
    pub bottle_quantities: [u8; 4],
    pub acquired_item_bits: [u8; 32],
    pub collect_item_bits: [u8; 8],
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativePlayerRelationshipsObservation {
    pub ride_actor: Option<NativeActorIdentity>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativePlayerActionObservation {
    pub procedure_id: u16,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeReturnPlaceWriterObservation {
    pub save_room: i8,
    pub save_point: u8,
    pub switch_room: i8,
    pub required_event_set: u16,
    pub required_event_unset: u16,
    pub required_switch_set: u8,
    pub required_switch_unset: u8,
    pub no_telop_clear: bool,
    pub event_set_satisfied: bool,
    pub event_unset_satisfied: bool,
    pub switch_set_satisfied: bool,
    pub switch_unset_satisfied: bool,
    pub eligible: bool,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeActorObservation {
    pub runtime_generation: u64,
    pub return_place_writer: Option<NativeReturnPlaceWriterObservation>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeLearningObservation {
    pub stage: String,
    pub room: i8,
    pub layer: i8,
    pub point: i16,
    pub player_present: bool,
    pub player_is_link: bool,
    pub player_position: [f32; 3],
    pub player_current_angle: [i16; 3],
    pub player_form_present: bool,
    pub player_is_wolf: bool,
    pub player_action: Option<NativePlayerActionObservation>,
    pub actors: Vec<NativeActorObservation>,
    pub player_resources_status: NativeChannelStatus,
    pub player_resources: Option<NativePlayerResourcesObservation>,
    pub player_relationships_status: NativeChannelStatus,
    pub player_relationships: Option<NativePlayerRelationshipsObservation>,
    pub event_flags: Option<Vec<u8>>,
    pub temporary_flags: Option<Vec<u8>>,
    pub temporary_event_bytes: Option<Vec<u8>>,
    pub dungeon_flags: Option<Vec<u8>>,
    pub switch_flags: Option<Vec<u8>>,
    pub switch_flag_room: i8,
    pub runtime_file_status: NativeChannelStatus,
    pub runtime_file: Option<NativeRuntimeFileObservation>,
    pub return_place_status: NativeChannelStatus,
    pub return_place: Option<NativeReturnPlaceObservation>,
    pub restart_status: NativeChannelStatus,
    pub restart: Option<NativeRestartObservation>,
    pub event_handoff_status: NativeChannelStatus,
    pub event_handoff: Option<NativeEventHandoffObservation>,
}
