//! Phase-correct fixed-width core features shared with native policy inference.

use dusklight_evidence::native_episode_shard::{NativeLearningObservation, NativeRawPad};
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;

pub const NATIVE_POLICY_FEATURE_WIDTH: usize = 120;
pub const NATIVE_POLICY_FEATURE_SCHEMA_SHA256: [u8; 32] = [
    0xb0, 0xb7, 0x08, 0xc7, 0x59, 0x4f, 0x25, 0xf7, 0x4e, 0x3f, 0x82, 0xc8, 0xeb, 0xbd, 0xe2, 0xb7,
    0x37, 0xcc, 0x4f, 0x1f, 0x00, 0x76, 0xe1, 0x14, 0xef, 0xdd, 0xb8, 0x23, 0xa5, 0x38, 0xd1, 0x47,
];

pub fn canonical_native_policy_feature_schema() -> String {
    include_str!("../../../../../tests/fixtures/automation/native_policy_features_v1.schema")
        .replace("\r\n", "\n")
}

pub fn native_policy_feature_schema_sha256() -> [u8; 32] {
    Sha256::digest(canonical_native_policy_feature_schema().as_bytes()).into()
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativePolicyFeatureInput {
    pub player_present: bool,
    pub player_is_link: bool,
    pub player_position: [f32; 3],
    pub player_velocity: [f32; 3],
    pub player_forward_speed: f32,
    pub player_current_yaw: i16,
    pub player_shape_yaw: i16,
    pub player_contacts: u8,
    pub player_ground_height: Option<f32>,
    pub player_roof_height: Option<f32>,
    pub event_running: bool,
    pub event_mode: u8,
    pub event_status: u8,
    pub event_map_tool_id: u8,
    pub next_stage_enabled: bool,
    pub camera_yaw_radians: Option<f32>,
    pub collision_correction: Option<[f32; 2]>,
    pub remaining_ticks: u32,
    pub previous_input: NativeRawPad,
    pub player_damage_wait_timer: i16,
    pub player_ice_damage_wait_timer: i16,
    pub player_sword_change_wait_timer: u8,
    pub player_do_status: u8,
    pub stage: String,
    pub room: i8,
    pub layer: i8,
    pub point: i16,
    pub player_procedure: u16,
    pub player_mode_flags: u32,
}

impl NativePolicyFeatureInput {
    pub fn from_observation(observation: &NativeLearningObservation) -> Self {
        Self {
            player_present: observation.player_present,
            player_is_link: observation.player_is_link,
            player_position: observation.player_position,
            player_velocity: observation.player_velocity,
            player_forward_speed: observation.player_forward_speed,
            player_current_yaw: observation.player_current_angle[1],
            player_shape_yaw: observation.player_shape_angle[1],
            player_contacts: observation.player_contacts,
            player_ground_height: observation.player_ground_height,
            player_roof_height: observation.player_roof_height,
            event_running: observation.event_running,
            event_mode: observation.event_mode,
            event_status: observation.event_status,
            event_map_tool_id: observation.event_map_tool_id,
            next_stage_enabled: observation.next_stage.is_some(),
            camera_yaw_radians: observation.camera_yaw_radians,
            collision_correction: observation.collision_correction,
            remaining_ticks: observation.remaining_ticks,
            previous_input: observation.previous_input,
            player_damage_wait_timer: observation.player_damage_wait_timer,
            player_ice_damage_wait_timer: observation.player_ice_damage_wait_timer,
            player_sword_change_wait_timer: observation.player_sword_change_wait_timer,
            player_do_status: observation.player_do_status,
            stage: observation.stage.clone(),
            room: observation.room,
            layer: observation.layer,
            point: observation.point,
            player_procedure: observation.player_procedure,
            player_mode_flags: observation.player_mode_flags,
        }
    }
}

pub fn encode_native_policy_features(
    input: &NativePolicyFeatureInput,
) -> Result<[f32; NATIVE_POLICY_FEATURE_WIDTH], NativePolicyFeatureError> {
    if input.player_contacts & !0x1f != 0 {
        return Err(NativePolicyFeatureError::new(
            "native policy player contacts contain unknown bits",
        ));
    }
    let stage = input.stage.as_bytes();
    if stage.len() > 8 || !stage.is_ascii() {
        return Err(NativePolicyFeatureError::new(
            "native policy stage name is not canonical eight-byte ASCII",
        ));
    }

    let mut values = Vec::with_capacity(NATIVE_POLICY_FEATURE_WIDTH);
    let push_bool = |values: &mut Vec<f32>, value: bool| values.push(f32::from(value));
    let push_player = |values: &mut Vec<f32>, value: f32| {
        values.push(if input.player_present { value } else { 0.0 });
    };
    push_bool(&mut values, input.player_present);
    push_bool(&mut values, input.player_present && input.player_is_link);
    for value in input.player_position {
        push_player(&mut values, value);
    }
    for value in input.player_velocity {
        push_player(&mut values, value);
    }
    push_player(&mut values, input.player_forward_speed);
    push_player(&mut values, f32::from(input.player_current_yaw) / 32768.0);
    push_player(&mut values, f32::from(input.player_shape_yaw) / 32768.0);
    for bit in 0..5 {
        push_bool(
            &mut values,
            input.player_present && input.player_contacts & (1 << bit) != 0,
        );
    }
    let ground_present = input.player_present && input.player_ground_height.is_some();
    push_bool(&mut values, ground_present);
    values.push(if ground_present {
        input.player_ground_height.unwrap_or(0.0)
    } else {
        0.0
    });
    let roof_present = input.player_present && input.player_roof_height.is_some();
    push_bool(&mut values, roof_present);
    values.push(if roof_present {
        input.player_roof_height.unwrap_or(0.0)
    } else {
        0.0
    });
    push_bool(&mut values, input.event_running);
    values.push(f32::from(input.event_mode) / 255.0);
    values.push(f32::from(input.event_status) / 255.0);
    values.push(f32::from(input.event_map_tool_id) / 255.0);
    push_bool(&mut values, input.next_stage_enabled);
    push_bool(&mut values, input.camera_yaw_radians.is_some());
    values.push(input.camera_yaw_radians.unwrap_or(0.0));
    push_bool(&mut values, input.collision_correction.is_some());
    values.extend(input.collision_correction.unwrap_or([0.0; 2]));
    values.push(input.remaining_ticks as f32);
    push_bool(&mut values, input.previous_input.connected);
    for value in [
        input.previous_input.stick_x,
        input.previous_input.stick_y,
        input.previous_input.substick_x,
        input.previous_input.substick_y,
    ] {
        values.push(if value < 0 {
            f32::from(value) / 128.0
        } else {
            f32::from(value) / 127.0
        });
    }
    for value in [
        input.previous_input.trigger_left,
        input.previous_input.trigger_right,
        input.previous_input.analog_a,
        input.previous_input.analog_b,
    ] {
        values.push(f32::from(value) / 255.0);
    }
    for bit in 0..16 {
        push_bool(&mut values, input.previous_input.buttons & (1 << bit) != 0);
    }
    values.push(f32::from(input.previous_input.error) / 128.0);
    push_player(
        &mut values,
        f32::from(input.player_damage_wait_timer) / 32768.0,
    );
    push_player(
        &mut values,
        f32::from(input.player_ice_damage_wait_timer) / 32768.0,
    );
    push_player(
        &mut values,
        f32::from(input.player_sword_change_wait_timer) / 255.0,
    );
    push_player(&mut values, f32::from(input.player_do_status) / 255.0);
    for index in 0..8 {
        values.push(f32::from(stage.get(index).copied().unwrap_or(0)) / 127.0);
    }
    values.push(f32::from(input.room) / 128.0);
    values.push(f32::from(input.layer) / 128.0);
    values.push(f32::from(input.point) / 32768.0);
    for bit in 0..16 {
        push_bool(
            &mut values,
            input.player_present && input.player_procedure & (1 << bit) != 0,
        );
    }
    for bit in 0..32 {
        push_bool(
            &mut values,
            input.player_present && input.player_mode_flags & (1 << bit) != 0,
        );
    }
    if values.len() != NATIVE_POLICY_FEATURE_WIDTH || values.iter().any(|value| !value.is_finite())
    {
        return Err(NativePolicyFeatureError::new(
            "native policy feature row has the wrong width or a non-finite value",
        ));
    }
    values
        .try_into()
        .map_err(|_| NativePolicyFeatureError::new("native policy feature width drifted"))
}

pub fn encode_native_policy_observation(
    observation: &NativeLearningObservation,
) -> Result<[f32; NATIVE_POLICY_FEATURE_WIDTH], NativePolicyFeatureError> {
    encode_native_policy_features(&NativePolicyFeatureInput::from_observation(observation))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativePolicyFeatureError(String);

impl NativePolicyFeatureError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for NativePolicyFeatureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for NativePolicyFeatureError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> NativePolicyFeatureInput {
        NativePolicyFeatureInput {
            player_present: true,
            player_is_link: true,
            player_position: [1.5, -2.0, 3.25],
            player_velocity: [-4.0, 5.0, -6.0],
            player_forward_speed: 7.0,
            player_current_yaw: -16384,
            player_shape_yaw: 8192,
            player_contacts: 0b1_0101,
            player_ground_height: Some(-12.5),
            player_roof_height: Some(35.0),
            event_running: true,
            event_mode: 128,
            event_status: 64,
            event_map_tool_id: 255,
            next_stage_enabled: true,
            camera_yaw_radians: Some(-1.25),
            collision_correction: Some([0.5, -0.75]),
            remaining_ticks: 123,
            previous_input: NativeRawPad {
                buttons: 0x8001,
                stick_x: -128,
                stick_y: 127,
                substick_x: -64,
                substick_y: 63,
                trigger_left: 255,
                trigger_right: 128,
                analog_a: 64,
                analog_b: 0,
                connected: true,
                error: -64,
            },
            player_damage_wait_timer: -16384,
            player_ice_damage_wait_timer: 8192,
            player_sword_change_wait_timer: 128,
            player_do_status: 255,
            stage: "F_SP103".into(),
            room: -64,
            layer: 32,
            point: -16384,
            player_procedure: 0x8001,
            player_mode_flags: 0x8000_0001,
        }
    }

    #[test]
    fn shared_schema_and_representative_row_match_native_contract() {
        assert_eq!(
            native_policy_feature_schema_sha256(),
            NATIVE_POLICY_FEATURE_SCHEMA_SHA256
        );
        let row = encode_native_policy_features(&input()).unwrap();
        assert_eq!(
            row[0..11],
            [1.0, 1.0, 1.5, -2.0, 3.25, -4.0, 5.0, -6.0, 7.0, -0.5, 0.25]
        );
        assert_eq!(row[11..16], [1.0, 0.0, 1.0, 0.0, 1.0]);
        assert_eq!(row[16..20], [1.0, -12.5, 1.0, 35.0]);
        assert_eq!(row[23], 1.0);
        assert_eq!(row[25..31], [1.0, -1.25, 1.0, 0.5, -0.75, 123.0]);
        assert_eq!(row[31..35], [1.0, -1.0, 1.0, -0.5]);
        assert_eq!(row[40], 1.0);
        assert_eq!(row[55], 1.0);
        assert_eq!(row[56..59], [-0.5, -0.5, 0.25]);
        assert_eq!(row[68], 0.0);
        assert_eq!(row[69..72], [-0.5, 0.25, -0.5]);
        assert_eq!(row[72], 1.0);
        assert_eq!(row[87], 1.0);
        assert_eq!(row[88], 1.0);
        assert_eq!(row[119], 1.0);
    }

    #[test]
    fn missing_values_are_masked_and_invalid_rows_fail_closed() {
        let mut missing = input();
        missing.player_present = false;
        let row = encode_native_policy_features(&missing).unwrap();
        assert_eq!(row[1], 0.0);
        assert_eq!(row[2], 0.0);
        assert_eq!(row[16], 0.0);
        assert_eq!(row[72], 0.0);
        assert_eq!(row[119], 0.0);

        let mut invalid = input();
        invalid.camera_yaw_radians = Some(f32::INFINITY);
        assert!(encode_native_policy_features(&invalid).is_err());
        invalid = input();
        invalid.stage = "TOO_LONG!".into();
        assert!(encode_native_policy_features(&invalid).is_err());
        invalid = input();
        invalid.player_contacts = 0x80;
        assert!(encode_native_policy_features(&invalid).is_err());
    }
}
