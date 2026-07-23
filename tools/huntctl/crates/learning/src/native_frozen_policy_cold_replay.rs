//! Fail-closed proof that a native frozen-policy episode replays as an ordinary tape.

use crate::artifact::Digest;
use crate::native_frozen_policy_reinference::{
    NativeFrozenPolicyReinferenceReport, realize_native_frozen_policy_tape,
};
use crate::tape::InputTape;
use dusklight_evidence::native_episode_shard::{
    NATIVE_EPISODE_SHARD_SCHEMA_V3, NativeEpisodeShard,
};
use dusklight_evidence::observation_parity::ObservationParityReport;
use dusklight_evidence::trace::DecodedTrace;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;

pub const NATIVE_FROZEN_POLICY_COLD_REPLAY_SCHEMA_V1: &str =
    "dusklight-native-frozen-policy-cold-replay/v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeFrozenPolicyColdReplayReport {
    pub schema: String,
    pub batch_result_sha256: Digest,
    pub reinference_report_sha256: Digest,
    pub learning_shard_sha256: Digest,
    pub source_tape_sha256: Digest,
    pub realized_tape_sha256: Digest,
    pub live_trace_sha256: Digest,
    pub cold_trace_sha256: Digest,
    pub cold_milestone_result_sha256: Digest,
    pub episode_id: String,
    pub source_frame: u64,
    pub transition_count: u64,
    pub compared_boundaries: u64,
    pub goal: String,
    pub goal_reached: bool,
    pub realized_tape_exact: bool,
    pub terminal_verdict_exact: bool,
    pub complete_gameplay_state_exact: bool,
    pub exact_pad_sequence: bool,
    pub gameplay_state_series_sha256: Digest,
    pub full_raw_pad_series_sha256: Digest,
    pub policy_pad_series_sha256: Digest,
    pub parity_report_sha256: Digest,
    pub verified: bool,
    pub report_sha256: Digest,
}

impl NativeFrozenPolicyColdReplayReport {
    pub fn validate(&self) -> Result<(), NativeFrozenPolicyColdReplayError> {
        if self.schema != NATIVE_FROZEN_POLICY_COLD_REPLAY_SCHEMA_V1
            || self.batch_result_sha256 == Digest::ZERO
            || self.reinference_report_sha256 == Digest::ZERO
            || self.learning_shard_sha256 == Digest::ZERO
            || self.source_tape_sha256 == Digest::ZERO
            || self.realized_tape_sha256 == Digest::ZERO
            || self.live_trace_sha256 == Digest::ZERO
            || self.cold_trace_sha256 == Digest::ZERO
            || self.cold_milestone_result_sha256 == Digest::ZERO
            || self.episode_id.is_empty()
            || self.transition_count == 0
            || self.compared_boundaries == 0
            || self.goal.is_empty()
            || !self.realized_tape_exact
            || !self.terminal_verdict_exact
            || !self.complete_gameplay_state_exact
            || !self.exact_pad_sequence
            || self.gameplay_state_series_sha256 == Digest::ZERO
            || self.full_raw_pad_series_sha256 == Digest::ZERO
            || self.policy_pad_series_sha256 == Digest::ZERO
            || self.parity_report_sha256 == Digest::ZERO
            || !self.verified
            || self.report_sha256 != self.compute_identity()?
        {
            return Err(error(
                "native frozen policy cold replay report envelope or seal is invalid",
            ));
        }
        Ok(())
    }

    fn compute_identity(&self) -> Result<Digest, NativeFrozenPolicyColdReplayError> {
        let mut canonical = self.clone();
        canonical.report_sha256 = Digest::ZERO;
        canonical_digest(
            b"dusklight.native-frozen-policy-cold-replay-report/v1\0",
            &canonical,
        )
    }
}

#[allow(clippy::too_many_arguments)]
pub fn verify_native_frozen_policy_cold_replay(
    batch_result_bytes: &[u8],
    reinference: &NativeFrozenPolicyReinferenceReport,
    source_tape: &InputTape,
    source_tape_bytes: &[u8],
    realized_tape: &InputTape,
    realized_tape_bytes: &[u8],
    shard: &NativeEpisodeShard,
    live_trace: &DecodedTrace,
    live_trace_bytes: &[u8],
    cold_trace: &DecodedTrace,
    cold_trace_bytes: &[u8],
    cold_milestone_result_bytes: &[u8],
    episode_id: &str,
) -> Result<NativeFrozenPolicyColdReplayReport, NativeFrozenPolicyColdReplayError> {
    reinference
        .validate()
        .map_err(|source| error(source.to_string()))?;
    if shard.metadata.shard_schema != NATIVE_EPISODE_SHARD_SCHEMA_V3
        || shard.content_sha256 != reinference.shard_content_sha256
    {
        return Err(error(
            "cold replay shard differs from the sealed reinference report",
        ));
    }
    let episode = shard
        .episodes
        .iter()
        .find(|episode| episode.id == episode_id)
        .ok_or_else(|| error("cold replay episode ID is absent from the shard"))?;
    if shard
        .episodes
        .iter()
        .filter(|episode| episode.id == episode_id)
        .count()
        != 1
        || episode.steps.is_empty()
        || episode.steps.len() != reinference.transition_count
    {
        return Err(error(
            "cold replay episode count or transition count differs from reinference",
        ));
    }

    let batch: Value = serde_json::from_slice(batch_result_bytes)
        .map_err(|source| error(format!("native batch result is invalid JSON: {source}")))?;
    let candidate = validate_batch_result(&batch, shard, reinference, episode_id)?;
    let predicate = candidate
        .get("predicate_evidence")
        .ok_or_else(|| error("native batch candidate lacks predicate evidence"))?;
    let cold_predicate: Value = serde_json::from_slice(cold_milestone_result_bytes)
        .map_err(|source| error(format!("cold milestone result is invalid JSON: {source}")))?;
    if predicate != &cold_predicate {
        return Err(error(
            "cold replay terminal predicate evidence differs from the native policy episode",
        ));
    }
    let (goal, goal_reached) =
        validate_milestone_evidence(predicate, reinference.objective_sha256)?;
    if bool_field(candidate, "success")? != goal_reached || episode.success != goal_reached {
        return Err(error(
            "native candidate, episode, and terminal predicate verdicts differ",
        ));
    }

    let expected_realized = realize_native_frozen_policy_tape(source_tape, shard, episode_id)
        .map_err(|source| error(source.to_string()))?;
    let expected_realized_bytes = expected_realized
        .encode()
        .map_err(|source| error(source.to_string()))?;
    if realized_tape != &expected_realized || realized_tape_bytes != expected_realized_bytes {
        return Err(error(
            "cold replay tape is not the exact ordinary realization of consumed policy PADs",
        ));
    }
    let expected_boundaries = realized_tape.frames.len() as u64;
    if live_trace.records.len() as u64 != expected_boundaries
        || cold_trace.records.len() as u64 != expected_boundaries
    {
        return Err(error(
            "cold replay proof does not retain the complete live and cold boot boundary sequence",
        ));
    }

    let parity = ObservationParityReport::build(
        cold_trace,
        cold_trace_bytes,
        live_trace,
        live_trace_bytes,
        shard,
    )
    .map_err(|source| error(source.to_string()))?;
    if !parity.verified
        || parity.learning_shard_sha256 != shard.content_sha256
        || parity.source_frame != shard.source_frame
        || parity.compared_boundaries != expected_boundaries
        || parity.compared_learning_steps != episode.steps.len() as u64
        || parity.observation_off_state_series_sha256 != parity.observation_on_state_series_sha256
        || parity.observation_off_raw_pad_series_sha256
            != parity.observation_on_raw_pad_series_sha256
        || parity.learning_consumed_pad_series_sha256
            != parity.observation_on_suffix_pad_series_sha256
    {
        return Err(error(
            "model-free cold replay differs in complete gameplay state or exact PAD sequence",
        ));
    }

    let mut report = NativeFrozenPolicyColdReplayReport {
        schema: NATIVE_FROZEN_POLICY_COLD_REPLAY_SCHEMA_V1.into(),
        batch_result_sha256: sha256(batch_result_bytes),
        reinference_report_sha256: reinference.report_sha256,
        learning_shard_sha256: shard.content_sha256,
        source_tape_sha256: sha256(source_tape_bytes),
        realized_tape_sha256: sha256(realized_tape_bytes),
        live_trace_sha256: sha256(live_trace_bytes),
        cold_trace_sha256: sha256(cold_trace_bytes),
        cold_milestone_result_sha256: sha256(cold_milestone_result_bytes),
        episode_id: episode_id.into(),
        source_frame: shard.source_frame,
        transition_count: episode.steps.len() as u64,
        compared_boundaries: parity.compared_boundaries,
        goal,
        goal_reached,
        realized_tape_exact: true,
        terminal_verdict_exact: true,
        complete_gameplay_state_exact: true,
        exact_pad_sequence: true,
        gameplay_state_series_sha256: parity.observation_off_state_series_sha256,
        full_raw_pad_series_sha256: parity.observation_off_raw_pad_series_sha256,
        policy_pad_series_sha256: parity.learning_consumed_pad_series_sha256,
        parity_report_sha256: parity.report_sha256,
        verified: true,
        report_sha256: Digest::ZERO,
    };
    report.report_sha256 = report.compute_identity()?;
    report.validate()?;
    Ok(report)
}

fn validate_batch_result<'a>(
    batch: &'a Value,
    shard: &NativeEpisodeShard,
    reinference: &NativeFrozenPolicyReinferenceReport,
    episode_id: &str,
) -> Result<&'a Value, NativeFrozenPolicyColdReplayError> {
    let feature_schema_sha256 = reinference.feature_schema_sha256.to_string();
    let action_schema_sha256 = reinference.action_schema_sha256.to_string();
    let objective_sha256 = reinference.objective_sha256.to_string();
    let schema = string_field(batch, "schema")?;
    if !matches!(
        schema.as_str(),
        "dusklight-suffix-batch-result/v5"
            | "dusklight-suffix-batch-result/v6"
            | "dusklight-suffix-batch-result/v7"
    ) || string_field(batch, "status")? != "passed"
        || u64_field(batch, "source_frame")? != shard.source_frame
        || string_field(batch, "restore_identity")? != reinference.checkpoint_identity
        || batch
            .pointer("/source_boundary/actual_fingerprint")
            .and_then(Value::as_str)
            != Some(reinference.source_boundary_fingerprint.as_str())
        || batch
            .pointer("/source_boundary/verified")
            .and_then(Value::as_bool)
            != Some(true)
        || batch
            .pointer("/checkpoint_validation/verified")
            .and_then(Value::as_bool)
            != Some(true)
        || batch
            .pointer("/episode_shard/schema")
            .and_then(Value::as_str)
            != Some(NATIVE_EPISODE_SHARD_SCHEMA_V3)
        || batch
            .pointer("/policy_model/schema")
            .and_then(Value::as_str)
            != Some(if schema == "dusklight-suffix-batch-result/v7" {
                "dusklight-native-frozen-policy/v2"
            } else {
                "dusklight-native-frozen-policy/v1"
            })
        || batch
            .pointer("/policy_model/model_xxh3_128")
            .and_then(Value::as_str)
            != Some(reinference.model_xxh3_128.as_str())
        || batch
            .pointer("/policy_model/feature_schema_sha256")
            .and_then(Value::as_str)
            != Some(feature_schema_sha256.as_str())
        || batch
            .pointer("/policy_model/action_schema_sha256")
            .and_then(Value::as_str)
            != Some(action_schema_sha256.as_str())
        || batch
            .pointer("/policy_model/objective_sha256")
            .and_then(Value::as_str)
            != Some(objective_sha256.as_str())
        || (schema == "dusklight-suffix-batch-result/v7"
            && !batch
                .pointer("/policy_model/rollout_exploration")
                .is_some_and(valid_rollout_exploration))
    {
        return Err(error(
            "native frozen policy batch result is not a passed identity-complete v5/v6/v7 run",
        ));
    }
    let candidates = batch
        .get("candidates")
        .and_then(Value::as_array)
        .ok_or_else(|| error("native batch result lacks candidates"))?;
    let mut matching = candidates
        .iter()
        .filter(|candidate| candidate.get("id").and_then(Value::as_str) == Some(episode_id));
    let candidate = matching
        .next()
        .ok_or_else(|| error("native batch result lacks the requested episode"))?;
    let terminal_boundary_valid = !matches!(
        schema.as_str(),
        "dusklight-suffix-batch-result/v6" | "dusklight-suffix-batch-result/v7"
    )
        || candidate
            .get("terminal_boundary_fingerprint")
            .and_then(Value::as_str)
            .is_some_and(|value| {
                value.len() == 32
                    && value
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            });
    if matching.next().is_some()
        || u64_field(candidate, "ticks_executed")? != reinference.transition_count as u64
        || !terminal_boundary_valid
    {
        return Err(error(
            "native batch candidate identity or transition count differs",
        ));
    }
    Ok(candidate)
}

fn valid_rollout_exploration(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    const KEYS: [&str; 6] = [
        "schema",
        "seed",
        "stick_axis_delta_probability_millionths",
        "maximum_stick_axis_delta",
        "button_flip_probability_millionths",
        "button_flip_mask",
    ];
    object.len() == KEYS.len()
        && KEYS.iter().all(|key| object.contains_key(*key))
        && object.get("schema").and_then(Value::as_str)
            == Some("dusklight-native-policy-rollout-exploration/v1")
        && object.get("seed").and_then(Value::as_u64).is_some_and(|value| value > 0)
        && object
            .get("stick_axis_delta_probability_millionths")
            .and_then(Value::as_u64)
            .is_some_and(|value| value <= 1_000_000)
        && object
            .get("maximum_stick_axis_delta")
            .and_then(Value::as_u64)
            .is_some_and(|value| (1..=64).contains(&value))
        && object
            .get("button_flip_probability_millionths")
            .and_then(Value::as_u64)
            .is_some_and(|value| value <= 1_000_000)
        && object
            .get("button_flip_mask")
            .and_then(Value::as_u64)
            .is_some_and(|value| (1..=u64::from(u16::MAX)).contains(&value))
}

fn validate_milestone_evidence(
    evidence: &Value,
    objective_sha256: Digest,
) -> Result<(String, bool), NativeFrozenPolicyColdReplayError> {
    let goal = string_field(evidence, "goal")?;
    let goal_reached = bool_field(evidence, "goal_reached")?;
    let program_digest = string_field(evidence, "program_digest")?;
    let objective_sha256 = objective_sha256.to_string();
    let valid_digest = |value: &str| {
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    };
    let milestones = evidence
        .get("milestones")
        .and_then(Value::as_array)
        .ok_or_else(|| error("cold replay predicate evidence lacks milestones"))?;
    let matching = milestones
        .iter()
        .filter(|milestone| milestone.get("id").and_then(Value::as_str) == Some(goal.as_str()))
        .collect::<Vec<_>>();
    if evidence.pointer("/schema/name").and_then(Value::as_str)
        != Some("dusklight.automation.milestones")
        || evidence.pointer("/schema/version").and_then(Value::as_u64) != Some(5)
        || evidence.pointer("/boot/kind").and_then(Value::as_str) != Some("process")
        || evidence
            .get("boot_origin_established")
            .and_then(Value::as_bool)
            != Some(true)
        || !valid_digest(&program_digest)
        || matching.len() != 1
        || matching[0].get("phase").and_then(Value::as_str) != Some("post_sim")
        || matching[0].get("program_digest").and_then(Value::as_str)
            != Some(program_digest.as_str())
        || matching[0].get("definition_digest").and_then(Value::as_str)
            != Some(objective_sha256.as_str())
        || matching[0].get("hit").and_then(Value::as_bool) != Some(goal_reached)
    {
        return Err(error(
            "cold replay predicate is not the exact process-boot authored milestone objective",
        ));
    }
    Ok((goal, goal_reached))
}

fn string_field(value: &Value, name: &str) -> Result<String, NativeFrozenPolicyColdReplayError> {
    value
        .get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| error(format!("cold replay evidence lacks string field {name}")))
}

fn bool_field(value: &Value, name: &str) -> Result<bool, NativeFrozenPolicyColdReplayError> {
    value
        .get(name)
        .and_then(Value::as_bool)
        .ok_or_else(|| error(format!("cold replay evidence lacks boolean field {name}")))
}

fn u64_field(value: &Value, name: &str) -> Result<u64, NativeFrozenPolicyColdReplayError> {
    value
        .get(name)
        .and_then(Value::as_u64)
        .ok_or_else(|| error(format!("cold replay evidence lacks integer field {name}")))
}

fn sha256(bytes: &[u8]) -> Digest {
    Digest(Sha256::digest(bytes).into())
}

fn canonical_digest(
    domain: &[u8],
    value: &impl Serialize,
) -> Result<Digest, NativeFrozenPolicyColdReplayError> {
    let bytes = serde_json::to_vec(value).map_err(|source| error(source.to_string()))?;
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
    Ok(Digest(hasher.finalize().into()))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeFrozenPolicyColdReplayError(String);

fn error(message: impl Into<String>) -> NativeFrozenPolicyColdReplayError {
    NativeFrozenPolicyColdReplayError(message.into())
}

impl fmt::Display for NativeFrozenPolicyColdReplayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for NativeFrozenPolicyColdReplayError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::factorized_pad_action::FactorizedPadPolicyHead;
    use crate::frozen_inference::FROZEN_INFERENCE_SCHEMA_V1;
    use crate::native_frozen_policy_reinference::verify_native_frozen_policy_reinference;
    use crate::native_frozen_policy_suffix_batch::native_frozen_policy_probe_model;
    use crate::native_policy_features::encode_native_policy_observation;
    use crate::tape::InputFrame;
    use dusklight_automation_contracts::tape::{RawPadState, TapeBoot};
    use dusklight_evidence::native_episode_shard::{
        NativeEpisodePolicyModelIdentity, NativeRawPad,
    };
    use dusklight_evidence::trace::{
        DecodedTrace, TraceAppliedPads, TraceChannel, TraceChannelStatus, TracePhase, TraceRecord,
    };
    use serde_json::json;
    use std::collections::BTreeMap;

    struct Fixture {
        batch: Vec<u8>,
        reinference: NativeFrozenPolicyReinferenceReport,
        source: InputTape,
        source_bytes: Vec<u8>,
        realized: InputTape,
        realized_bytes: Vec<u8>,
        shard: NativeEpisodeShard,
        trace: DecodedTrace,
        predicate: Vec<u8>,
        episode_id: String,
    }

    fn fixture() -> Fixture {
        let objective = Digest([0x43; 32]);
        let model = native_frozen_policy_probe_model(objective).unwrap();
        let model_bytes = model.to_bytes().unwrap();
        let model_xxh3_128 = format!("{:032x}", xxhash_rust::xxh3::xxh3_128(&model_bytes));
        let mut shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v27.dseps"
        ))
        .unwrap();
        shard.source_frame = 0;
        shard.metadata.shard_schema = NATIVE_EPISODE_SHARD_SCHEMA_V3.into();
        shard.metadata.policy_model = Some(NativeEpisodePolicyModelIdentity {
            schema: FROZEN_INFERENCE_SCHEMA_V1.into(),
            model_xxh3_128: model_xxh3_128.clone(),
            feature_schema_sha256: model.feature_schema_sha256,
            action_schema_sha256: model.action_schema_sha256,
            objective_sha256: objective,
            feature_width: model.input_width as u32,
        });
        shard.episodes.truncate(1);
        let episode = &mut shard.episodes[0];
        episode.steps.truncate(1);
        episode.ticks_executed = 1;
        episode.remaining_ticks = shard.maximum_ticks - 1;
        episode.success = false;
        episode.first_hit_tick = None;
        let row = encode_native_policy_observation(&episode.steps[0].pre_input).unwrap();
        let output = model.infer_batch(&[row.to_vec()]).unwrap().remove(0);
        let pad = FactorizedPadPolicyHead::default()
            .decode(&output)
            .unwrap()
            .realized_pad()
            .unwrap();
        episode.steps[0].chosen_pad = pad;
        episode.steps[0].consumed_pad = pad;
        episode.steps[0].post_simulation.previous_input = pad;
        let checkpoint = shard.metadata.checkpoint_identity.clone();
        let boundary = shard.metadata.source_boundary_fingerprint.clone();
        let reinference = verify_native_frozen_policy_reinference(
            &model_bytes,
            None,
            &shard,
            objective,
            &checkpoint,
            &boundary,
        )
        .unwrap();

        let source = InputTape {
            frames: vec![InputFrame::default(); shard.source_frame as usize + 2],
            ..InputTape::default()
        };
        let source_bytes = source.encode().unwrap();
        let episode_id = shard.episodes[0].id.clone();
        let realized = realize_native_frozen_policy_tape(&source, &shard, &episode_id).unwrap();
        let realized_bytes = realized.encode().unwrap();

        let program_digest = "24".repeat(32);
        let predicate_value = json!({
            "schema": {"name": "dusklight.automation.milestones", "version": 5},
            "boot": {"kind": "process"},
            "boot_origin_established": true,
            "goal": "probe_goal",
            "goal_reached": false,
            "program_digest": program_digest,
            "milestones": [{
                "id": "probe_goal",
                "hit": false,
                "phase": "post_sim",
                "program_digest": program_digest,
                "definition_digest": objective,
            }]
        });
        let predicate = serde_json::to_vec(&predicate_value).unwrap();
        let batch = serde_json::to_vec(&json!({
            "schema": "dusklight-suffix-batch-result/v5",
            "status": "passed",
            "source_frame": shard.source_frame,
            "restore_identity": checkpoint,
            "source_boundary": {"actual_fingerprint": boundary, "verified": true},
            "checkpoint_validation": {"verified": true},
            "episode_shard": {"schema": NATIVE_EPISODE_SHARD_SCHEMA_V3},
            "policy_model": {
                "schema": "dusklight-native-frozen-policy/v1",
                "model_xxh3_128": model_xxh3_128,
                "feature_schema_sha256": model.feature_schema_sha256,
                "action_schema_sha256": model.action_schema_sha256,
                "objective_sha256": objective,
            },
            "candidates": [{
                "id": episode_id,
                "ticks_executed": 1,
                "success": false,
                "predicate_evidence": predicate_value,
            }]
        }))
        .unwrap();

        let post = &shard.episodes[0].steps[0].post_simulation;
        let channel_status = TraceChannel::ALL
            .into_iter()
            .map(|channel| (channel, TraceChannelStatus::Present))
            .collect::<BTreeMap<_, _>>();
        let trace_pad = contract_pad(pad);
        let trace = DecodedTrace {
            version: 5,
            boot: TapeBoot::Process,
            tick_rate_numerator: 30,
            tick_rate_denominator: 1,
            requested_channels: TraceChannel::ALL
                .into_iter()
                .fold(0, |mask, channel| mask | channel.bit()),
            capacity_exhausted: false,
            retention: None,
            channel_formats: BTreeMap::new(),
            records: vec![TraceRecord {
                boundary_index: post.boundary_index,
                simulation_tick: post.simulation_tick,
                tape_frame: Some(post.tape_frame),
                observation_phase: TracePhase::PostSimulation,
                input_source: 1,
                channel_status,
                applied_pads: Some(TraceAppliedPads {
                    valid_ports: 1,
                    owned_ports: 1,
                    pads: [
                        trace_pad,
                        RawPadState::default(),
                        RawPadState::default(),
                        RawPadState::default(),
                    ],
                }),
                ..TraceRecord::default()
            }],
        };

        Fixture {
            batch,
            reinference,
            source,
            source_bytes,
            realized,
            realized_bytes,
            shard,
            trace,
            predicate,
            episode_id,
        }
    }

    fn contract_pad(pad: NativeRawPad) -> RawPadState {
        RawPadState {
            buttons: pad.buttons,
            stick_x: pad.stick_x,
            stick_y: pad.stick_y,
            substick_x: pad.substick_x,
            substick_y: pad.substick_y,
            trigger_left: pad.trigger_left,
            trigger_right: pad.trigger_right,
            analog_a: pad.analog_a,
            analog_b: pad.analog_b,
            connected: pad.connected,
            error: pad.error,
        }
    }

    fn verify(
        fixture: &Fixture,
        realized: &InputTape,
        realized_bytes: &[u8],
        cold_trace: &DecodedTrace,
        cold_predicate: &[u8],
    ) -> Result<NativeFrozenPolicyColdReplayReport, NativeFrozenPolicyColdReplayError> {
        verify_native_frozen_policy_cold_replay(
            &fixture.batch,
            &fixture.reinference,
            &fixture.source,
            &fixture.source_bytes,
            realized,
            realized_bytes,
            &fixture.shard,
            &fixture.trace,
            b"live-trace",
            cold_trace,
            b"cold-trace",
            cold_predicate,
            &fixture.episode_id,
        )
    }

    #[test]
    fn seals_exact_model_free_cold_replay() {
        let fixture = fixture();
        let mut report = verify(
            &fixture,
            &fixture.realized,
            &fixture.realized_bytes,
            &fixture.trace,
            &fixture.predicate,
        )
        .unwrap();
        assert!(report.verified);
        assert_eq!(report.transition_count, 1);
        assert_eq!(report.compared_boundaries, 1);
        report.validate().unwrap();
        report.transition_count += 1;
        assert!(report.validate().is_err());
    }

    #[test]
    fn rejects_detached_batch_policy_identity() {
        let mut fixture = fixture();
        let mut batch: Value = serde_json::from_slice(&fixture.batch).unwrap();
        batch["policy_model"]["objective_sha256"] = Value::String("00".repeat(32));
        fixture.batch = serde_json::to_vec(&batch).unwrap();
        assert!(
            verify(
                &fixture,
                &fixture.realized,
                &fixture.realized_bytes,
                &fixture.trace,
                &fixture.predicate,
            )
            .unwrap_err()
            .to_string()
            .contains("identity-complete")
        );
    }

    #[test]
    fn v6_cold_replay_requires_the_exact_terminal_boundary_field() {
        let mut fixture = fixture();
        let mut batch: Value = serde_json::from_slice(&fixture.batch).unwrap();
        batch["schema"] = Value::String("dusklight-suffix-batch-result/v6".into());
        fixture.batch = serde_json::to_vec(&batch).unwrap();
        assert!(
            verify(
                &fixture,
                &fixture.realized,
                &fixture.realized_bytes,
                &fixture.trace,
                &fixture.predicate,
            )
            .is_err()
        );

        batch["candidates"][0]["terminal_boundary_fingerprint"] = Value::String("a".repeat(32));
        fixture.batch = serde_json::to_vec(&batch).unwrap();
        verify(
            &fixture,
            &fixture.realized,
            &fixture.realized_bytes,
            &fixture.trace,
            &fixture.predicate,
        )
        .unwrap();
    }

    #[test]
    fn v7_cold_replay_requires_bound_rollout_exploration() {
        let mut fixture = fixture();
        let mut batch: Value = serde_json::from_slice(&fixture.batch).unwrap();
        batch["schema"] = Value::String("dusklight-suffix-batch-result/v7".into());
        batch["policy_model"]["schema"] =
            Value::String("dusklight-native-frozen-policy/v2".into());
        batch["policy_model"]["rollout_exploration"] = json!({
            "schema": "dusklight-native-policy-rollout-exploration/v1",
            "seed": 17,
            "stick_axis_delta_probability_millionths": 0,
            "maximum_stick_axis_delta": 32,
            "button_flip_probability_millionths": 0,
            "button_flip_mask": 3967,
        });
        batch["candidates"][0]["terminal_boundary_fingerprint"] =
            Value::String("a".repeat(32));
        fixture.batch = serde_json::to_vec(&batch).unwrap();
        verify(
            &fixture,
            &fixture.realized,
            &fixture.realized_bytes,
            &fixture.trace,
            &fixture.predicate,
        )
        .unwrap();

        batch["policy_model"]["rollout_exploration"]["seed"] = Value::from(0);
        fixture.batch = serde_json::to_vec(&batch).unwrap();
        assert!(
            verify(
                &fixture,
                &fixture.realized,
                &fixture.realized_bytes,
                &fixture.trace,
                &fixture.predicate,
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_tape_state_and_terminal_divergence() {
        let fixture = fixture();
        let mut changed_tape = fixture.realized.clone();
        changed_tape.frames.last_mut().unwrap().pads[0].stick_x ^= 1;
        let changed_tape_bytes = changed_tape.encode().unwrap();
        assert!(
            verify(
                &fixture,
                &changed_tape,
                &changed_tape_bytes,
                &fixture.trace,
                &fixture.predicate,
            )
            .unwrap_err()
            .to_string()
            .contains("exact ordinary realization")
        );

        let mut changed_trace = fixture.trace.clone();
        changed_trace.records[0].position[0] += 1.0;
        assert!(
            verify(
                &fixture,
                &fixture.realized,
                &fixture.realized_bytes,
                &changed_trace,
                &fixture.predicate,
            )
            .unwrap_err()
            .to_string()
            .contains("gameplay state")
        );

        let mut changed_predicate: Value = serde_json::from_slice(&fixture.predicate).unwrap();
        changed_predicate["goal_reached"] = Value::Bool(true);
        assert!(
            verify(
                &fixture,
                &fixture.realized,
                &fixture.realized_bytes,
                &fixture.trace,
                &serde_json::to_vec(&changed_predicate).unwrap(),
            )
            .unwrap_err()
            .to_string()
            .contains("predicate evidence differs")
        );
    }
}
