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
pub struct NativeMessageSessionObservation {
    pub procedure: u16,
    pub message_id: u32,
    pub message_index: i32,
    pub node_index: u16,
    pub flow_id: i16,
    pub selection_count: u8,
    pub selection_cursor: u8,
    pub selection_push: u8,
    pub output_type: u8,
    pub talk_now: bool,
    pub talk_message: bool,
    pub auto_message: bool,
    pub kill_pending: bool,
    pub camera_cancel: bool,
    pub send: bool,
    pub send_control: bool,
    pub talk_actor: NativeActorIdentity,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeEventActorReferenceObservation {
    pub status: NativeChannelStatus,
    pub actor: Option<NativeActorIdentity>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativePendingEventOrderObservation {
    pub event_type: u16,
    pub flags: u16,
    pub hind_flags: u16,
    pub event_id: i16,
    pub priority: u16,
    pub map_tool_id: u8,
    pub request_actor: NativeEventActorReferenceObservation,
    pub target_actor: NativeEventActorReferenceObservation,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeEventQueueObservation {
    pub pending_orders: Vec<NativePendingEventOrderObservation>,
    pub active_request_actor: NativeEventActorReferenceObservation,
    pub active_target_actor: NativeEventActorReferenceObservation,
    pub active_talk_actor: NativeEventActorReferenceObservation,
    pub active_item_actor: NativeEventActorReferenceObservation,
    pub active_door_actor: NativeEventActorReferenceObservation,
    pub change_actor: NativeEventActorReferenceObservation,
    pub skip_registered: bool,
    pub skip_actor: NativeEventActorReferenceObservation,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeAttentionCandidateObservation {
    pub actor: NativeEventActorReferenceObservation,
    pub weight: f32,
    pub distance: f32,
    pub angle: i16,
    pub attention_type: u32,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeAttentionCandidatesObservation {
    pub player_attention_flags: u32,
    pub attention_status: u8,
    pub attention_block_timer: i32,
    pub lock_offset: u8,
    pub action_offset: u8,
    pub check_offset: u8,
    pub lock_candidates: Vec<NativeAttentionCandidateObservation>,
    pub action_candidates: Vec<NativeAttentionCandidateObservation>,
    pub check_candidates: Vec<NativeAttentionCandidateObservation>,
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
    #[serde(default)]
    pub maximum_oil: Option<u16>,
    #[serde(default)]
    pub oil: Option<u16>,
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
    /// Exact live `dSv_event_c::mEvent` payload owned by the active runtime
    /// file. `event_flags` is the separate label-indexed diagnostic view.
    #[serde(default)]
    pub persistent_event_bytes: Option<Vec<u8>>,
    /// Exact `dSv_player_info_c::mLightDrop` payload: four tear-count bytes
    /// followed by the Vessel ownership bitfield.
    #[serde(default)]
    pub player_light_drop_bytes: Option<Vec<u8>>,
    pub event_flags: Option<Vec<u8>>,
    pub temporary_flags: Option<Vec<u8>>,
    pub temporary_event_bytes: Option<Vec<u8>>,
    /// Exact live `dSv_memBit_c` payload for the currently loaded stage bank.
    /// `dungeon_flags` is the separate label-indexed `dSv_danBit_c` view.
    #[serde(default)]
    pub loaded_stage_memory_bytes: Option<Vec<u8>>,
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
    pub message_session_status: NativeChannelStatus,
    pub message_session: Option<NativeMessageSessionObservation>,
    pub event_queue_status: NativeChannelStatus,
    pub event_queue: Option<NativeEventQueueObservation>,
    pub attention_candidates_status: NativeChannelStatus,
    pub attention_candidates: Option<NativeAttentionCandidatesObservation>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_observations_without_exact_stage_memory_remain_decodable() {
        let mut value = serde_json::to_value(NativeLearningObservation::default()).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .remove("persistent_event_bytes");
        value
            .as_object_mut()
            .unwrap()
            .remove("player_light_drop_bytes");
        value
            .as_object_mut()
            .unwrap()
            .remove("loaded_stage_memory_bytes");
        let decoded: NativeLearningObservation = serde_json::from_value(value).unwrap();
        assert_eq!(decoded.persistent_event_bytes, None);
        assert_eq!(decoded.player_light_drop_bytes, None);
        assert_eq!(decoded.loaded_stage_memory_bytes, None);
    }
}
