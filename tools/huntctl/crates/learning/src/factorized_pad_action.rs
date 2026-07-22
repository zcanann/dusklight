//! Lossless, factorized frame-level controller actions.
//!
//! Learned policies may reason about continuous stick geometry, independent
//! analog controls, button bits, and duration without reducing PAD to a small
//! authored catalog. The exact integer action remains authoritative and expands
//! to the same native PAD state on every covered frame.

use dusklight_evidence::native_episode_shard::NativeRawPad;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;

pub const FACTORIZED_PAD_ACTION_SCHEMA_V1: &str = "dusklight-factorized-pad-action/v1";
pub const FACTORIZED_PAD_FEATURE_SCHEMA_V1: &str = "dusklight-factorized-pad-features/v1";
pub const FACTORIZED_PAD_FEATURE_WIDTH: usize = 33;
pub const FACTORIZED_PAD_POLICY_HEAD_SCHEMA_V1: &str = "dusklight-factorized-pad-policy-head/v1";
pub const FACTORIZED_PAD_POLICY_HEAD_WIDTH: usize = 25;
pub const ONLINE_FACTORIZED_PAD_ACTION_SCHEMA_SHA256: [u8; 32] = [
    0x48, 0x39, 0x53, 0xb6, 0x63, 0xe3, 0x27, 0xc6, 0xd8, 0x39, 0xd8, 0x8b, 0x72, 0xce, 0x65, 0x2c,
    0xfc, 0xc5, 0xd0, 0xae, 0xb4, 0x20, 0x57, 0x0e, 0x0c, 0x07, 0x6a, 0x4c, 0x62, 0xe3, 0xab, 0x7c,
];
pub const MAX_FACTORIZED_PAD_DURATION: u32 = 4096;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StickBytes {
    pub x: i8,
    pub y: i8,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FactorizedPadAction {
    pub schema: String,
    pub main_stick: StickBytes,
    pub camera_stick: StickBytes,
    pub trigger_left: u8,
    pub trigger_right: u8,
    pub analog_a: u8,
    pub analog_b: u8,
    /// Absolute held-button state. Press/release edges are derived relative to
    /// the preceding realized frame, so all button combinations remain usable.
    pub buttons: u16,
    pub duration_ticks: u32,
}

impl FactorizedPadAction {
    pub fn neutral(duration_ticks: u32) -> Result<Self, FactorizedPadActionError> {
        let action = Self {
            schema: FACTORIZED_PAD_ACTION_SCHEMA_V1.into(),
            main_stick: StickBytes { x: 0, y: 0 },
            camera_stick: StickBytes { x: 0, y: 0 },
            trigger_left: 0,
            trigger_right: 0,
            analog_a: 0,
            analog_b: 0,
            buttons: 0,
            duration_ticks,
        };
        action.validate()?;
        Ok(action)
    }

    pub fn from_pad(
        pad: NativeRawPad,
        duration_ticks: u32,
    ) -> Result<Self, FactorizedPadActionError> {
        if !pad.connected || pad.error != 0 {
            return Err(FactorizedPadActionError::UnavailablePad);
        }
        let action = Self {
            schema: FACTORIZED_PAD_ACTION_SCHEMA_V1.into(),
            main_stick: StickBytes {
                x: pad.stick_x,
                y: pad.stick_y,
            },
            camera_stick: StickBytes {
                x: pad.substick_x,
                y: pad.substick_y,
            },
            trigger_left: pad.trigger_left,
            trigger_right: pad.trigger_right,
            analog_a: pad.analog_a,
            analog_b: pad.analog_b,
            buttons: pad.buttons,
            duration_ticks,
        };
        action.validate()?;
        Ok(action)
    }

    pub fn validate(&self) -> Result<(), FactorizedPadActionError> {
        if self.schema != FACTORIZED_PAD_ACTION_SCHEMA_V1
            || self.duration_ticks == 0
            || self.duration_ticks > MAX_FACTORIZED_PAD_DURATION
        {
            return Err(FactorizedPadActionError::InvalidAction);
        }
        Ok(())
    }

    pub fn realized_pad(&self) -> Result<NativeRawPad, FactorizedPadActionError> {
        self.validate()?;
        Ok(NativeRawPad {
            buttons: self.buttons,
            stick_x: self.main_stick.x,
            stick_y: self.main_stick.y,
            substick_x: self.camera_stick.x,
            substick_y: self.camera_stick.y,
            trigger_left: self.trigger_left,
            trigger_right: self.trigger_right,
            analog_a: self.analog_a,
            analog_b: self.analog_b,
            connected: true,
            error: 0,
        })
    }

    pub fn button_edges(
        &self,
        previous_buttons: u16,
    ) -> Result<ButtonEdges, FactorizedPadActionError> {
        self.validate()?;
        Ok(ButtonEdges {
            pressed: self.buttons & !previous_buttons,
            released: previous_buttons & !self.buttons,
            held: self.buttons,
        })
    }

    /// Dense model features retain both Cartesian and polar stick views. The
    /// exact bytes live in `FactorizedPadAction`; these floats are not replay
    /// authority.
    pub fn model_features(
        &self,
    ) -> Result<[f32; FACTORIZED_PAD_FEATURE_WIDTH], FactorizedPadActionError> {
        self.validate()?;
        let mut output = [0.0; FACTORIZED_PAD_FEATURE_WIDTH];
        let main = stick_features(self.main_stick);
        let camera = stick_features(self.camera_stick);
        output[0..5].copy_from_slice(&main);
        output[5..10].copy_from_slice(&camera);
        output[10] = f32::from(self.trigger_left) / 255.0;
        output[11] = f32::from(self.trigger_right) / 255.0;
        output[12] = f32::from(self.analog_a) / 255.0;
        output[13] = f32::from(self.analog_b) / 255.0;
        for bit in 0..16 {
            output[14 + bit] = f32::from((self.buttons >> bit) & 1);
        }
        output[30] = self.duration_ticks as f32 / MAX_FACTORIZED_PAD_DURATION as f32;
        output[31] =
            (self.duration_ticks as f32).ln_1p() / (MAX_FACTORIZED_PAD_DURATION as f32).ln_1p();
        output[32] = 1.0;
        Ok(output)
    }

    pub fn feature_schema_sha256() -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(b"dusklight.factorized-pad-features/v1\0");
        hasher.update(FACTORIZED_PAD_FEATURE_SCHEMA_V1.as_bytes());
        hasher.update((FACTORIZED_PAD_FEATURE_WIDTH as u32).to_le_bytes());
        hasher.update(b"main[x,y,magnitude,sin,cos];camera[x,y,magnitude,sin,cos];analog[l,r,a,b];buttons[0..16);duration[linear,log];bias");
        hasher.finalize().into()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct ButtonEdges {
    pub pressed: u16,
    pub released: u16,
    pub held: u16,
}

/// Interpretation of one continuous policy-output row. Stick channels use
/// `[-1, 1]`, analog and duration channels use `[0, 1]`, and buttons use logits
/// with a strict `> threshold` decision. Finite values outside a bounded channel
/// saturate; NaN and infinities fail closed.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct FactorizedPadPolicyHead {
    pub maximum_duration_ticks: u32,
    pub button_logit_threshold: f32,
}

impl Default for FactorizedPadPolicyHead {
    fn default() -> Self {
        Self {
            maximum_duration_ticks: 1,
            button_logit_threshold: 0.0,
        }
    }
}

impl FactorizedPadPolicyHead {
    pub fn validate(&self) -> Result<(), FactorizedPadActionError> {
        if self.maximum_duration_ticks == 0
            || self.maximum_duration_ticks > MAX_FACTORIZED_PAD_DURATION
            || !self.button_logit_threshold.is_finite()
        {
            return Err(FactorizedPadActionError::InvalidPolicyHead);
        }
        Ok(())
    }

    pub fn decode(&self, output: &[f32]) -> Result<FactorizedPadAction, FactorizedPadActionError> {
        self.validate()?;
        if output.len() != FACTORIZED_PAD_POLICY_HEAD_WIDTH
            || output.iter().any(|value| !value.is_finite())
        {
            return Err(FactorizedPadActionError::InvalidPolicyOutput);
        }
        let mut buttons = 0_u16;
        for bit in 0..16 {
            if output[8 + bit] > self.button_logit_threshold {
                buttons |= 1 << bit;
            }
        }
        let duration_ticks = if self.maximum_duration_ticks == 1 {
            1
        } else {
            (output[24].clamp(0.0, 1.0) * (self.maximum_duration_ticks - 1) as f32).round() as u32
                + 1
        };
        let action = FactorizedPadAction {
            schema: FACTORIZED_PAD_ACTION_SCHEMA_V1.into(),
            main_stick: StickBytes {
                x: quantize_signed_axis(output[0]),
                y: quantize_signed_axis(output[1]),
            },
            camera_stick: StickBytes {
                x: quantize_signed_axis(output[2]),
                y: quantize_signed_axis(output[3]),
            },
            trigger_left: quantize_unit_byte(output[4]),
            trigger_right: quantize_unit_byte(output[5]),
            analog_a: quantize_unit_byte(output[6]),
            analog_b: quantize_unit_byte(output[7]),
            buttons,
            duration_ticks,
        };
        action.validate()?;
        Ok(action)
    }

    pub fn schema_sha256(&self) -> Result<[u8; 32], FactorizedPadActionError> {
        self.validate()?;
        let mut hasher = Sha256::new();
        hasher.update(b"dusklight.factorized-pad-policy-head/v1\0");
        hasher.update(FACTORIZED_PAD_POLICY_HEAD_SCHEMA_V1.as_bytes());
        hasher.update((FACTORIZED_PAD_POLICY_HEAD_WIDTH as u32).to_le_bytes());
        hasher.update(self.maximum_duration_ticks.to_le_bytes());
        hasher.update(self.button_logit_threshold.to_bits().to_le_bytes());
        Ok(hasher.finalize().into())
    }
}

pub fn expand_actions(
    actions: &[FactorizedPadAction],
) -> Result<Vec<NativeRawPad>, FactorizedPadActionError> {
    let frame_count = actions.iter().try_fold(0_usize, |total, action| {
        action.validate()?;
        total
            .checked_add(action.duration_ticks as usize)
            .filter(|count| *count <= 1_000_000)
            .ok_or(FactorizedPadActionError::ExpansionTooLarge)
    })?;
    let mut frames = Vec::with_capacity(frame_count);
    for action in actions {
        frames.extend(std::iter::repeat_n(
            action.realized_pad()?,
            action.duration_ticks as usize,
        ));
    }
    Ok(frames)
}

pub fn compress_frames(
    frames: &[NativeRawPad],
) -> Result<Vec<FactorizedPadAction>, FactorizedPadActionError> {
    if frames.is_empty() || frames.len() > 1_000_000 {
        return Err(FactorizedPadActionError::InvalidFrameSequence);
    }
    let mut actions = Vec::new();
    let mut start = 0;
    while start < frames.len() {
        let pad = frames[start];
        if !pad.connected || pad.error != 0 {
            return Err(FactorizedPadActionError::UnavailablePad);
        }
        let mut end = start + 1;
        while end < frames.len()
            && frames[end] == pad
            && end - start < MAX_FACTORIZED_PAD_DURATION as usize
        {
            end += 1;
        }
        actions.push(FactorizedPadAction::from_pad(pad, (end - start) as u32)?);
        start = end;
    }
    Ok(actions)
}

fn stick_features(stick: StickBytes) -> [f32; 5] {
    let x = signed_byte_unit(stick.x);
    let y = signed_byte_unit(stick.y);
    let radius = x.hypot(y);
    let magnitude = radius.min(1.0);
    if radius == 0.0 {
        [x, y, 0.0, 0.0, 0.0]
    } else {
        [x, y, magnitude, y / radius, x / radius]
    }
}

fn signed_byte_unit(value: i8) -> f32 {
    if value < 0 {
        f32::from(value) / 128.0
    } else {
        f32::from(value) / 127.0
    }
}

fn quantize_signed_axis(value: f32) -> i8 {
    let value = value.clamp(-1.0, 1.0);
    if value < 0.0 {
        (value * 128.0).round() as i8
    } else {
        (value * 127.0).round() as i8
    }
}

fn quantize_unit_byte(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FactorizedPadActionError {
    InvalidAction,
    UnavailablePad,
    InvalidFrameSequence,
    ExpansionTooLarge,
    InvalidPolicyHead,
    InvalidPolicyOutput,
}

impl fmt::Display for FactorizedPadActionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidAction => formatter.write_str("invalid factorized PAD action"),
            Self::UnavailablePad => {
                formatter.write_str("PAD action requires a connected, error-free controller")
            }
            Self::InvalidFrameSequence => formatter.write_str("invalid raw PAD frame sequence"),
            Self::ExpansionTooLarge => {
                formatter.write_str("factorized PAD expansion exceeds its bounded frame budget")
            }
            Self::InvalidPolicyHead => formatter.write_str("invalid factorized PAD policy head"),
            Self::InvalidPolicyOutput => {
                formatter.write_str("invalid factorized PAD policy output")
            }
        }
    }
}

impl Error for FactorizedPadActionError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn pad(buttons: u16, x: i8, y: i8) -> NativeRawPad {
        NativeRawPad {
            buttons,
            stick_x: x,
            stick_y: y,
            substick_x: -128,
            substick_y: 127,
            trigger_left: 17,
            trigger_right: 255,
            analog_a: 3,
            analog_b: 99,
            connected: true,
            error: 0,
        }
    }

    #[test]
    fn losslessly_round_trips_every_raw_field_and_run_length() {
        let frames = [
            pad(0x0100, -128, 127),
            pad(0x0100, -128, 127),
            pad(0x0200, 1, -1),
        ];
        let actions = compress_frames(&frames).unwrap();
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].duration_ticks, 2);
        assert_eq!(expand_actions(&actions).unwrap(), frames);
    }

    #[test]
    fn exposes_button_edges_without_removing_absolute_replay_authority() {
        let action = FactorizedPadAction::from_pad(pad(0b0110, 0, 0), 4).unwrap();
        assert_eq!(
            action.button_edges(0b0011).unwrap(),
            ButtonEdges {
                pressed: 0b0100,
                released: 0b0001,
                held: 0b0110,
            }
        );
        assert_eq!(action.realized_pad().unwrap().buttons, 0b0110);
    }

    #[test]
    fn dense_features_are_finite_and_cover_independent_factor_blocks() {
        let action = FactorizedPadAction::from_pad(pad(0x8001, -128, 127), 4096).unwrap();
        let features = action.model_features().unwrap();
        assert!(features.iter().all(|value| value.is_finite()));
        assert_eq!(features[0], -1.0);
        assert_eq!(features[1], 1.0);
        assert!((features[3].hypot(features[4]) - 1.0).abs() < 1.0e-6);
        assert_eq!(features[14], 1.0);
        assert_eq!(features[29], 1.0);
        assert_eq!(features[30], 1.0);
        assert_ne!(FactorizedPadAction::feature_schema_sha256(), [0; 32]);
    }

    #[test]
    fn rejects_transport_state_and_unbounded_duration() {
        let mut unavailable = pad(0, 0, 0);
        unavailable.connected = false;
        assert_eq!(
            FactorizedPadAction::from_pad(unavailable, 1),
            Err(FactorizedPadActionError::UnavailablePad)
        );
        assert_eq!(
            FactorizedPadAction::neutral(MAX_FACTORIZED_PAD_DURATION + 1),
            Err(FactorizedPadActionError::InvalidAction)
        );
    }

    #[test]
    fn zero_policy_output_is_one_frame_of_neutral_pad() {
        let head = FactorizedPadPolicyHead {
            maximum_duration_ticks: 32,
            ..FactorizedPadPolicyHead::default()
        };
        let action = head
            .decode(&[0.0; FACTORIZED_PAD_POLICY_HEAD_WIDTH])
            .unwrap();
        assert_eq!(action, FactorizedPadAction::neutral(1).unwrap());
        assert_ne!(head.schema_sha256().unwrap(), [0; 32]);
    }

    #[test]
    fn policy_head_reaches_every_channel_extreme_and_independent_button_bit() {
        let head = FactorizedPadPolicyHead {
            maximum_duration_ticks: MAX_FACTORIZED_PAD_DURATION,
            button_logit_threshold: 0.25,
        };
        let mut output = [0.0; FACTORIZED_PAD_POLICY_HEAD_WIDTH];
        output[0] = -4.0;
        output[1] = 4.0;
        output[2] = 1.0;
        output[3] = -1.0;
        output[4] = -2.0;
        output[5] = 2.0;
        output[6] = 1.0;
        output[7] = 0.5;
        output[8 + 11] = 0.250_001;
        output[24] = 3.0;
        let action = head.decode(&output).unwrap();
        assert_eq!(action.main_stick, StickBytes { x: -128, y: 127 });
        assert_eq!(action.camera_stick, StickBytes { x: 127, y: -128 });
        assert_eq!(action.trigger_left, 0);
        assert_eq!(action.trigger_right, 255);
        assert_eq!(action.analog_a, 255);
        assert_eq!(action.analog_b, 128);
        assert_eq!(action.buttons, 1 << 11);
        assert_eq!(action.duration_ticks, MAX_FACTORIZED_PAD_DURATION);
    }

    #[test]
    fn policy_head_rejects_nonfinite_or_detached_rows() {
        let head = FactorizedPadPolicyHead::default();
        assert_eq!(
            head.decode(&[0.0; FACTORIZED_PAD_POLICY_HEAD_WIDTH - 1]),
            Err(FactorizedPadActionError::InvalidPolicyOutput)
        );
        let mut output = [0.0; FACTORIZED_PAD_POLICY_HEAD_WIDTH];
        output[3] = f32::NAN;
        assert_eq!(
            head.decode(&output),
            Err(FactorizedPadActionError::InvalidPolicyOutput)
        );
    }

    #[test]
    fn one_tick_online_head_matches_the_native_action_schema_identity() {
        assert_eq!(
            FactorizedPadPolicyHead::default().schema_sha256().unwrap(),
            ONLINE_FACTORIZED_PAD_ACTION_SCHEMA_SHA256
        );
    }

    #[test]
    fn matches_the_shared_native_policy_golden_fixture() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../../tests/fixtures/automation/factorized_pad_policy_v1.json"
        ))
        .unwrap();
        assert_eq!(
            fixture["schema"],
            "dusklight-factorized-pad-policy-golden/v1"
        );
        for case in fixture["cases"].as_array().unwrap() {
            let output = case["output"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| value.as_f64().unwrap() as f32)
                .collect::<Vec<_>>();
            let head = FactorizedPadPolicyHead {
                maximum_duration_ticks: case["maximum_duration_ticks"].as_u64().unwrap() as u32,
                button_logit_threshold: case["button_logit_threshold"].as_f64().unwrap() as f32,
            };
            let action = head.decode(&output).unwrap();
            let expected = &case["expected"];
            assert_eq!(action.buttons, expected["buttons"].as_u64().unwrap() as u16);
            assert_eq!(
                action.main_stick.x,
                expected["stick_x"].as_i64().unwrap() as i8
            );
            assert_eq!(
                action.main_stick.y,
                expected["stick_y"].as_i64().unwrap() as i8
            );
            assert_eq!(
                action.camera_stick.x,
                expected["substick_x"].as_i64().unwrap() as i8
            );
            assert_eq!(
                action.camera_stick.y,
                expected["substick_y"].as_i64().unwrap() as i8
            );
            assert_eq!(
                action.trigger_left,
                expected["trigger_left"].as_u64().unwrap() as u8
            );
            assert_eq!(
                action.trigger_right,
                expected["trigger_right"].as_u64().unwrap() as u8
            );
            assert_eq!(
                action.analog_a,
                expected["analog_a"].as_u64().unwrap() as u8
            );
            assert_eq!(
                action.analog_b,
                expected["analog_b"].as_u64().unwrap() as u8
            );
            assert_eq!(
                action.duration_ticks,
                expected["duration_ticks"].as_u64().unwrap() as u32
            );
        }
    }
}
