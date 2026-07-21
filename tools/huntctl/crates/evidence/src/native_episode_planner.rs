use super::{NativeActorIdentity, NativeChannelStatus};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativePhysicalSlotObservation {
    pub number: u8,
    pub content_status: NativeChannelStatus,
    pub attached_to_runtime: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativeRuntimeFileObservation {
    pub no_file_raw: u8,
    pub data_num_raw: u8,
    pub backing_attachment_status: NativeChannelStatus,
    pub attached_physical_slot: Option<u8>,
    pub physical_slots: [NativePhysicalSlotObservation; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativeReturnPlaceObservation {
    pub stage: String,
    pub room: i8,
    pub player_status: u8,
}

#[derive(Clone, Debug, PartialEq)]
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeMessageFlowObservation {
    pub flow_id: u16,
    pub node_index: u16,
    pub cut_name_hash: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativePlayerControlObservation {
    pub mode_flags: u32,
    pub do_status: u8,
}

#[derive(Clone, Debug, PartialEq)]
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

#[derive(Clone, Debug, PartialEq)]
pub struct NativeEventActorReferenceObservation {
    pub status: NativeChannelStatus,
    pub actor: Option<NativeActorIdentity>,
}

#[derive(Clone, Debug, PartialEq)]
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

#[derive(Clone, Debug, PartialEq)]
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

#[derive(Clone, Debug, PartialEq)]
pub struct NativeAttentionCandidateObservation {
    pub actor: NativeEventActorReferenceObservation,
    pub weight: f32,
    pub distance: f32,
    pub angle: i16,
    pub attention_type: u32,
}

#[derive(Clone, Debug, PartialEq)]
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

#[derive(Clone, Debug, PartialEq)]
pub struct NativeCurrentEventObservation {
    pub event_id: i16,
    pub event_type: i32,
    pub room: i32,
    pub goal: [f32; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativePendingStageObservation {
    pub stage: String,
    pub room: i8,
    pub layer: i8,
    pub point: i16,
    pub wipe: i8,
    pub wipe_speed: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativeEventTransitionObservation {
    pub event_data_loaded: bool,
    pub camera_play: i32,
    pub current_event: Option<NativeCurrentEventObservation>,
    pub pending_stage: Option<NativePendingStageObservation>,
}

#[derive(Clone, Debug, PartialEq)]
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
