//! Independent Rust reinference of native frozen-policy episode shards.

use crate::artifact::Digest;
use crate::factorized_pad_action::{FACTORIZED_PAD_POLICY_HEAD_WIDTH, FactorizedPadPolicyHead};
use crate::frozen_inference::{FROZEN_INFERENCE_SCHEMA_V1, FrozenInferenceModel};
use crate::native_policy_features::{
    NATIVE_POLICY_FEATURE_SCHEMA_SHA256, NATIVE_POLICY_FEATURE_WIDTH,
    encode_native_policy_observation,
};
use crate::tape::{InputTape, RawPadState as TapeRawPadState};
use dusklight_evidence::native_episode_shard::{
    NATIVE_EPISODE_SHARD_SCHEMA_V3, NativeEpisodeShard, NativeRawPad,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;

pub const NATIVE_FROZEN_POLICY_REINFERENCE_SCHEMA_V1: &str =
    "dusklight-native-frozen-policy-reinference/v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeFrozenPolicyReinferenceReport {
    pub schema: String,
    pub shard_content_sha256: Digest,
    pub shard_schema: String,
    pub model_xxh3_128: String,
    pub feature_schema_sha256: Digest,
    pub action_schema_sha256: Digest,
    pub objective_sha256: Digest,
    pub checkpoint_identity: String,
    pub source_boundary_fingerprint: String,
    pub feature_width: usize,
    pub episode_count: usize,
    pub transition_count: usize,
    pub feature_rows_sha256: Digest,
    pub decoded_pad_sequence_sha256: Digest,
    pub chosen_pad_sequence_sha256: Digest,
    pub consumed_pad_sequence_sha256: Digest,
    pub decoded_matches_chosen: bool,
    pub chosen_matches_consumed: bool,
    pub verified: bool,
    pub report_sha256: Digest,
}

impl NativeFrozenPolicyReinferenceReport {
    pub fn validate(&self) -> Result<(), NativeFrozenPolicyReinferenceError> {
        if self.schema != NATIVE_FROZEN_POLICY_REINFERENCE_SCHEMA_V1
            || self.shard_content_sha256 == Digest::ZERO
            || self.shard_schema != NATIVE_EPISODE_SHARD_SCHEMA_V3
            || !is_lower_hex(&self.model_xxh3_128, 32)
            || self.feature_schema_sha256 == Digest::ZERO
            || self.action_schema_sha256 == Digest::ZERO
            || self.objective_sha256 == Digest::ZERO
            || !is_lower_hex(&self.checkpoint_identity, 32)
            || !is_lower_hex(&self.source_boundary_fingerprint, 32)
            || self.feature_width != NATIVE_POLICY_FEATURE_WIDTH
            || self.episode_count == 0
            || self.transition_count == 0
            || self.feature_rows_sha256 == Digest::ZERO
            || !self.decoded_matches_chosen
            || !self.chosen_matches_consumed
            || !self.verified
            || self.decoded_pad_sequence_sha256 != self.chosen_pad_sequence_sha256
            || self.chosen_pad_sequence_sha256 != self.consumed_pad_sequence_sha256
            || self.report_sha256 != self.compute_identity()?
        {
            return Err(error(
                "native frozen policy reinference report envelope or seal is invalid",
            ));
        }
        Ok(())
    }

    fn compute_identity(&self) -> Result<Digest, NativeFrozenPolicyReinferenceError> {
        let mut canonical = self.clone();
        canonical.report_sha256 = Digest::ZERO;
        let bytes = serde_json::to_vec(&canonical).map_err(|source| error(source.to_string()))?;
        let mut hasher = Sha256::new();
        hasher.update(b"dusklight.native-frozen-policy-reinference-report/v1\0");
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
        Ok(Digest(hasher.finalize().into()))
    }
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub fn verify_native_frozen_policy_reinference(
    model_bytes: &[u8],
    shard: &NativeEpisodeShard,
    expected_objective_sha256: Digest,
    expected_checkpoint_identity: &str,
    expected_source_boundary_fingerprint: &str,
) -> Result<NativeFrozenPolicyReinferenceReport, NativeFrozenPolicyReinferenceError> {
    if shard.metadata.shard_schema != NATIVE_EPISODE_SHARD_SCHEMA_V3 {
        return Err(error(
            "native policy episode shard is not identity-complete v3",
        ));
    }
    let policy = shard
        .metadata
        .policy_model
        .as_ref()
        .ok_or_else(|| error("native policy episode shard lacks its frozen model identity"))?;
    let model = FrozenInferenceModel::from_bytes(model_bytes)
        .map_err(|source| error(format!("frozen policy model is invalid: {source}")))?;
    let model_xxh3_128 = format!("{:032x}", xxhash_rust::xxh3::xxh3_128(model_bytes));
    let head = FactorizedPadPolicyHead::default();
    let action_schema_sha256 = Digest(
        head.schema_sha256()
            .map_err(|source| error(source.to_string()))?,
    );
    if policy.schema != FROZEN_INFERENCE_SCHEMA_V1
        || policy.model_xxh3_128 != model_xxh3_128
        || policy.feature_schema_sha256 != model.feature_schema_sha256
        || policy.feature_schema_sha256 != Digest(NATIVE_POLICY_FEATURE_SCHEMA_SHA256)
        || policy.action_schema_sha256 != model.action_schema_sha256
        || policy.action_schema_sha256 != action_schema_sha256
        || policy.objective_sha256 != model.objective_sha256
        || policy.objective_sha256 != expected_objective_sha256
        || policy.feature_width as usize != model.input_width
        || policy.feature_width as usize != NATIVE_POLICY_FEATURE_WIDTH
        || model.actions != (0..FACTORIZED_PAD_POLICY_HEAD_WIDTH as u32).collect::<Vec<_>>()
    {
        return Err(error(
            "frozen model, feature schema, action schema, objective, or feature width differs",
        ));
    }
    if shard.metadata.checkpoint_identity != expected_checkpoint_identity {
        return Err(error("native policy checkpoint identity differs"));
    }
    if shard.metadata.source_boundary_fingerprint != expected_source_boundary_fingerprint {
        return Err(error("native policy source boundary fingerprint differs"));
    }

    let mut feature_rows = Sha256::new();
    feature_rows.update(b"dusklight.native-policy-feature-rows/v1\0");
    let mut decoded_pads = Sha256::new();
    decoded_pads.update(b"dusklight.native-policy-pad-sequence/v1\0");
    let mut chosen_pads = Sha256::new();
    chosen_pads.update(b"dusklight.native-policy-pad-sequence/v1\0");
    let mut consumed_pads = Sha256::new();
    consumed_pads.update(b"dusklight.native-policy-pad-sequence/v1\0");
    let mut transition_count = 0_usize;

    for episode in &shard.episodes {
        let mut inputs = Vec::with_capacity(episode.steps.len());
        for step in &episode.steps {
            let row = encode_native_policy_observation(&step.pre_input)
                .map_err(|source| error(source.to_string()))?;
            feature_rows.update((episode.id.len() as u32).to_le_bytes());
            feature_rows.update(episode.id.as_bytes());
            feature_rows.update(step.pre_input.boundary_index.to_le_bytes());
            for value in row {
                feature_rows.update(value.to_bits().to_le_bytes());
            }
            inputs.push(row.to_vec());
        }
        let outputs = model
            .infer_batch(&inputs)
            .map_err(|source| error(source.to_string()))?;
        for (step, output) in episode.steps.iter().zip(outputs) {
            let decoded = head
                .decode(&output)
                .and_then(|action| action.realized_pad())
                .map_err(|source| error(source.to_string()))?;
            append_pad(&mut decoded_pads, decoded);
            append_pad(&mut chosen_pads, step.chosen_pad);
            append_pad(&mut consumed_pads, step.consumed_pad);
            if decoded != step.chosen_pad {
                return Err(error(format!(
                    "Rust decoded PAD differs from native chosen PAD in episode {} at transition {}",
                    episode.id, transition_count
                )));
            }
            if step.chosen_pad != step.consumed_pad {
                return Err(error(format!(
                    "native chosen PAD differs from consumed PAD in episode {} at transition {}",
                    episode.id, transition_count
                )));
            }
            transition_count += 1;
        }
    }
    if transition_count == 0 {
        return Err(error("native frozen policy shard contains no transitions"));
    }

    let mut report = NativeFrozenPolicyReinferenceReport {
        schema: NATIVE_FROZEN_POLICY_REINFERENCE_SCHEMA_V1.into(),
        shard_content_sha256: shard.content_sha256,
        shard_schema: shard.metadata.shard_schema.clone(),
        model_xxh3_128,
        feature_schema_sha256: model.feature_schema_sha256,
        action_schema_sha256: model.action_schema_sha256,
        objective_sha256: model.objective_sha256,
        checkpoint_identity: shard.metadata.checkpoint_identity.clone(),
        source_boundary_fingerprint: shard.metadata.source_boundary_fingerprint.clone(),
        feature_width: model.input_width,
        episode_count: shard.episodes.len(),
        transition_count,
        feature_rows_sha256: Digest(feature_rows.finalize().into()),
        decoded_pad_sequence_sha256: Digest(decoded_pads.finalize().into()),
        chosen_pad_sequence_sha256: Digest(chosen_pads.finalize().into()),
        consumed_pad_sequence_sha256: Digest(consumed_pads.finalize().into()),
        decoded_matches_chosen: true,
        chosen_matches_consumed: true,
        verified: true,
        report_sha256: Digest::ZERO,
    };
    report.report_sha256 = report.compute_identity()?;
    report.validate()?;
    Ok(report)
}

/// Materialize one retained policy episode as an ordinary absolute boot tape.
/// The source prefix remains byte-semantic input authority; only port zero in
/// the realized suffix is replaced with the shard's actually consumed PAD.
pub fn realize_native_frozen_policy_tape(
    source: &InputTape,
    shard: &NativeEpisodeShard,
    episode_id: &str,
) -> Result<InputTape, NativeFrozenPolicyReinferenceError> {
    if shard.metadata.shard_schema != NATIVE_EPISODE_SHARD_SCHEMA_V3
        || shard.metadata.policy_model.is_none()
    {
        return Err(error(
            "native policy episode shard is not identity-complete v3",
        ));
    }
    let episode = shard
        .episodes
        .iter()
        .find(|episode| episode.id == episode_id)
        .ok_or_else(|| error("native policy episode ID is absent from the shard"))?;
    let source_frame = usize::try_from(shard.source_frame)
        .map_err(|_| error("native policy source frame exceeds usize"))?;
    let end = source_frame
        .checked_add(episode.steps.len())
        .filter(|end| *end <= source.frames.len())
        .ok_or_else(|| error("native policy episode exceeds its source tape"))?;
    let mut realized = source.clone();
    realized.frames.truncate(end);
    for (offset, step) in episode.steps.iter().enumerate() {
        let frame = &mut realized.frames[source_frame + offset];
        frame.owned_ports |= 1;
        frame.pads[0] = TapeRawPadState {
            buttons: step.consumed_pad.buttons,
            stick_x: step.consumed_pad.stick_x,
            stick_y: step.consumed_pad.stick_y,
            substick_x: step.consumed_pad.substick_x,
            substick_y: step.consumed_pad.substick_y,
            trigger_left: step.consumed_pad.trigger_left,
            trigger_right: step.consumed_pad.trigger_right,
            analog_a: step.consumed_pad.analog_a,
            analog_b: step.consumed_pad.analog_b,
            connected: step.consumed_pad.connected,
            error: step.consumed_pad.error,
        };
    }
    realized
        .validate()
        .map_err(|source| error(format!("realized native policy tape is invalid: {source}")))?;
    Ok(realized)
}

fn append_pad(hasher: &mut Sha256, pad: NativeRawPad) {
    hasher.update(pad.buttons.to_le_bytes());
    hasher.update([pad.stick_x as u8, pad.stick_y as u8]);
    hasher.update([pad.substick_x as u8, pad.substick_y as u8]);
    hasher.update([
        pad.trigger_left,
        pad.trigger_right,
        pad.analog_a,
        pad.analog_b,
        u8::from(pad.connected),
        pad.error as u8,
    ]);
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeFrozenPolicyReinferenceError(String);

fn error(message: impl Into<String>) -> NativeFrozenPolicyReinferenceError {
    NativeFrozenPolicyReinferenceError(message.into())
}

impl fmt::Display for NativeFrozenPolicyReinferenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for NativeFrozenPolicyReinferenceError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_frozen_policy_suffix_batch::native_frozen_policy_probe_model;
    use dusklight_evidence::native_episode_shard::NativeEpisodePolicyModelIdentity;

    fn fixture() -> (Vec<u8>, NativeEpisodeShard, Digest, String, String) {
        let objective = Digest([0x43; 32]);
        let model = native_frozen_policy_probe_model(objective).unwrap();
        let bytes = model.to_bytes().unwrap();
        let model_xxh3_128 = format!("{:032x}", xxhash_rust::xxh3::xxh3_128(&bytes));
        let mut shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v27.dseps"
        ))
        .unwrap();
        shard.metadata.shard_schema = NATIVE_EPISODE_SHARD_SCHEMA_V3.into();
        shard.metadata.policy_model = Some(NativeEpisodePolicyModelIdentity {
            schema: FROZEN_INFERENCE_SCHEMA_V1.into(),
            model_xxh3_128,
            feature_schema_sha256: model.feature_schema_sha256,
            action_schema_sha256: model.action_schema_sha256,
            objective_sha256: objective,
            feature_width: model.input_width as u32,
        });
        for episode in &mut shard.episodes {
            for step in &mut episode.steps {
                let row = encode_native_policy_observation(&step.pre_input).unwrap();
                let output = model.infer_batch(&[row.to_vec()]).unwrap().remove(0);
                let pad = FactorizedPadPolicyHead::default()
                    .decode(&output)
                    .unwrap()
                    .realized_pad()
                    .unwrap();
                step.chosen_pad = pad;
                step.consumed_pad = pad;
                step.post_simulation.previous_input = pad;
            }
        }
        let checkpoint = shard.metadata.checkpoint_identity.clone();
        let boundary = shard.metadata.source_boundary_fingerprint.clone();
        (bytes, shard, objective, checkpoint, boundary)
    }

    #[test]
    fn reproduces_every_native_pad_from_pre_input_rows() {
        let (bytes, shard, objective, checkpoint, boundary) = fixture();
        let mut report = verify_native_frozen_policy_reinference(
            &bytes,
            &shard,
            objective,
            &checkpoint,
            &boundary,
        )
        .unwrap();
        assert!(report.verified);
        assert_eq!(
            report.transition_count,
            shard
                .episodes
                .iter()
                .map(|episode| episode.steps.len())
                .sum::<usize>()
        );
        assert_eq!(
            report.decoded_pad_sequence_sha256,
            report.chosen_pad_sequence_sha256
        );
        assert_eq!(
            report.chosen_pad_sequence_sha256,
            report.consumed_pad_sequence_sha256
        );
        report.transition_count += 1;
        assert!(report.validate().is_err());
    }

    #[test]
    fn rejects_every_detached_policy_and_source_identity() {
        let (bytes, shard, objective, checkpoint, boundary) = fixture();
        let verify = |shard: &NativeEpisodeShard, checkpoint: &str, boundary: &str| {
            verify_native_frozen_policy_reinference(&bytes, shard, objective, checkpoint, boundary)
                .unwrap_err()
                .to_string()
        };

        let mut detached = shard.clone();
        detached
            .metadata
            .policy_model
            .as_mut()
            .unwrap()
            .model_xxh3_128 = "1".repeat(32);
        assert!(verify(&detached, &checkpoint, &boundary).contains("differs"));
        detached = shard.clone();
        detached
            .metadata
            .policy_model
            .as_mut()
            .unwrap()
            .feature_schema_sha256 = Digest([1; 32]);
        assert!(verify(&detached, &checkpoint, &boundary).contains("differs"));
        detached = shard.clone();
        detached
            .metadata
            .policy_model
            .as_mut()
            .unwrap()
            .action_schema_sha256 = Digest([1; 32]);
        assert!(verify(&detached, &checkpoint, &boundary).contains("differs"));
        detached = shard.clone();
        detached
            .metadata
            .policy_model
            .as_mut()
            .unwrap()
            .objective_sha256 = Digest([1; 32]);
        assert!(verify(&detached, &checkpoint, &boundary).contains("differs"));
        detached = shard.clone();
        detached
            .metadata
            .policy_model
            .as_mut()
            .unwrap()
            .feature_width += 1;
        assert!(verify(&detached, &checkpoint, &boundary).contains("differs"));
        assert!(verify(&shard, &"1".repeat(32), &boundary).contains("checkpoint"));
        assert!(verify(&shard, &checkpoint, &"1".repeat(32)).contains("boundary"));

        detached = shard.clone();
        detached.episodes[0].steps[0].chosen_pad.stick_x += 1;
        assert!(verify(&detached, &checkpoint, &boundary).contains("decoded PAD differs"));
    }

    #[test]
    fn realizes_consumed_suffix_as_an_ordinary_source_tape() {
        let (_, shard, _, _, _) = fixture();
        let mut source = InputTape::default();
        source.frames.resize_with(
            shard.source_frame as usize + shard.episodes[0].steps.len() + 3,
            Default::default,
        );
        let realized =
            realize_native_frozen_policy_tape(&source, &shard, &shard.episodes[0].id).unwrap();
        assert_eq!(
            realized.frames.len(),
            shard.source_frame as usize + shard.episodes[0].steps.len()
        );
        for (frame, step) in realized.frames[shard.source_frame as usize..]
            .iter()
            .zip(&shard.episodes[0].steps)
        {
            assert_eq!(frame.pads[0].stick_x, step.consumed_pad.stick_x);
            assert_eq!(frame.pads[0].stick_y, step.consumed_pad.stick_y);
            assert_ne!(frame.owned_ports & 1, 0);
        }
    }
}
