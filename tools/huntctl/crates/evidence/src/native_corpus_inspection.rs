//! Corpus-level diagnostics for decoded native learning episodes.
//!
//! The native shard decoder already fails closed on malformed boundaries,
//! non-finite floats, action shifts, and incomplete v4+ actor sets. This module
//! summarizes the surviving corpus so missing/constant channels, poor action
//! coverage, duplicates, and suspicious outcome partitions are visible before
//! learner code is allowed to treat the data as useful.

use crate::native_episode_shard::{
    NativeChannelStatus, NativeEpisodeShard, NativeLearningObservation, NativeRawPad,
};
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const NATIVE_CORPUS_INSPECTION_SCHEMA_V1: &str = "dusklight-native-corpus-inspection/v1";

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct ChannelCoverage {
    pub present: u64,
    pub absent: u64,
    pub unavailable: u64,
    pub not_sampled: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SetSizeSummary {
    pub minimum: usize,
    pub maximum: usize,
    pub total: u64,
    pub mean_microunits: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct FlagMaskCoverage {
    pub present: u64,
    pub missing: u64,
    pub widths: SetSizeSummary,
    pub ever_set_indices: Vec<usize>,
    pub varying_indices: Vec<usize>,
    pub constant_set_indices: Vec<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DuplicateTrajectoryGroup {
    pub payload_xxh3_128: String,
    pub copies: u64,
    pub successes: u64,
    pub failures: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DeterminismConflictGroup {
    pub execution_and_consumed_pad_sha256: String,
    pub copies: usize,
    pub distinct_payloads: usize,
    pub episode_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct NativeActionCoverage {
    pub unique_chosen_pad_states: usize,
    pub unique_consumed_pad_states: usize,
    pub unique_consumed_trajectories: usize,
    pub chosen_consumed_mismatches: u64,
    pub button_mask_counts: BTreeMap<String, u64>,
    pub stick_sample_counts: BTreeMap<String, u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct NativeIdentityInspection {
    pub unique_values: BTreeMap<String, usize>,
    /// Heuristic only: each distinct value of this field appears in exactly
    /// one outcome class while the corpus contains both outcomes. Such fields
    /// require an ablation; they are not proof of leakage by themselves.
    pub outcome_separating_candidates: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct NativeCorpusInspection {
    pub schema: String,
    pub shard_count: usize,
    pub shard_content_sha256: Vec<String>,
    pub observation_schemas: BTreeMap<String, u64>,
    pub action_schemas: BTreeMap<String, u64>,
    pub episode_count: u64,
    pub success_count: u64,
    pub failure_count: u64,
    pub transition_count: u64,
    pub observation_count: u64,
    pub terminal_observation_count: u64,
    pub validated_non_finite_values: u64,
    pub validated_phase_discontinuities: u64,
    pub truncated_actor_observations: u64,
    pub actor_set_sizes: SetSizeSummary,
    pub rng_stream_set_sizes: SetSizeSummary,
    pub collision_surface_set_sizes: SetSizeSummary,
    pub unique_actor_types: usize,
    pub channel_coverage: BTreeMap<String, ChannelCoverage>,
    pub missing_mask_counts: BTreeMap<String, u64>,
    pub flag_mask_coverage: BTreeMap<String, FlagMaskCoverage>,
    pub constant_pre_input_channels: BTreeMap<String, String>,
    pub action_coverage: NativeActionCoverage,
    pub duplicate_trajectory_groups: Vec<DuplicateTrajectoryGroup>,
    pub determinism_conflicts: Vec<DeterminismConflictGroup>,
    pub identities: NativeIdentityInspection,
    pub warnings: Vec<String>,
}

#[derive(Default)]
struct FlagAccumulator {
    present: u64,
    missing: u64,
    widths: SetAccumulator,
    first: Option<Vec<u8>>,
    ever_set: BTreeSet<usize>,
    varying: BTreeSet<usize>,
}

impl FlagAccumulator {
    fn push(&mut self, values: Option<&[u8]>) {
        let Some(values) = values else {
            self.missing += 1;
            return;
        };
        self.present += 1;
        self.widths.push(values.len());
        for (index, value) in values.iter().copied().enumerate() {
            if value != 0 {
                self.ever_set.insert(index);
            }
            if self
                .first
                .as_ref()
                .and_then(|first| first.get(index))
                .is_some_and(|first| *first != value)
            {
                self.varying.insert(index);
            }
        }
        if let Some(first) = &self.first
            && first.len() != values.len()
        {
            for index in first.len().min(values.len())..first.len().max(values.len()) {
                self.varying.insert(index);
            }
        }
        self.first.get_or_insert_with(|| values.to_vec());
    }

    fn finish(self) -> FlagMaskCoverage {
        let constant_set_indices = self.ever_set.difference(&self.varying).copied().collect();
        FlagMaskCoverage {
            present: self.present,
            missing: self.missing,
            widths: self.widths.finish(),
            ever_set_indices: self.ever_set.into_iter().collect(),
            varying_indices: self.varying.into_iter().collect(),
            constant_set_indices,
        }
    }
}

#[derive(Default)]
struct ReplayGroup {
    payloads: BTreeSet<String>,
    episode_ids: Vec<String>,
}

#[derive(Default)]
struct SetAccumulator {
    count: u64,
    minimum: usize,
    maximum: usize,
    total: u64,
}

impl SetAccumulator {
    fn push(&mut self, value: usize) {
        if self.count == 0 {
            self.minimum = value;
        } else {
            self.minimum = self.minimum.min(value);
        }
        self.maximum = self.maximum.max(value);
        self.total = self.total.saturating_add(value as u64);
        self.count += 1;
    }

    fn finish(self) -> SetSizeSummary {
        SetSizeSummary {
            minimum: self.minimum,
            maximum: self.maximum,
            total: self.total,
            mean_microunits: self
                .total
                .saturating_mul(1_000_000)
                .checked_div(self.count)
                .unwrap_or(0),
        }
    }
}

#[derive(Default)]
struct IdentityValues {
    values: BTreeMap<String, u8>,
}

impl IdentityValues {
    fn insert(&mut self, value: impl Into<String>, success: bool) {
        *self.values.entry(value.into()).or_default() |= if success { 2 } else { 1 };
    }

    fn separates_outcomes(&self, both_outcomes_present: bool) -> bool {
        both_outcomes_present
            && self.values.len() > 1
            && self
                .values
                .values()
                .all(|outcomes| matches!(outcomes, 1 | 2))
    }
}

fn record_status(coverage: &mut ChannelCoverage, status: NativeChannelStatus) {
    match status {
        NativeChannelStatus::Present => coverage.present += 1,
        NativeChannelStatus::Absent => coverage.absent += 1,
        NativeChannelStatus::Unavailable => coverage.unavailable += 1,
        NativeChannelStatus::NotSampled => coverage.not_sampled += 1,
    }
}

fn encode_pad(pad: NativeRawPad, output: &mut Vec<u8>) {
    output.extend_from_slice(&pad.buttons.to_le_bytes());
    output.extend_from_slice(&[
        pad.stick_x as u8,
        pad.stick_y as u8,
        pad.substick_x as u8,
        pad.substick_y as u8,
        pad.trigger_left,
        pad.trigger_right,
        pad.analog_a,
        pad.analog_b,
        u8::from(pad.connected),
        pad.error as u8,
    ]);
}

fn hash_field(hasher: &mut Sha256, value: &[u8]) {
    hasher.update((value.len() as u64).to_le_bytes());
    hasher.update(value);
}

fn replay_key(shard: &NativeEpisodeShard, source_state: [u8; 16], trajectory: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight.native-corpus-determinism-key/v1\0");
    for value in [
        shard.metadata.build_revision.as_bytes(),
        shard.metadata.aurora_revision.as_bytes(),
        shard.metadata.feature_digest.as_bytes(),
        shard.metadata.fidelity_profile.as_bytes(),
        shard
            .metadata
            .game_data_identity
            .as_deref()
            .unwrap_or_default()
            .as_bytes(),
        shard.metadata.checkpoint_identity.as_bytes(),
    ] {
        hash_field(&mut hasher, value);
    }
    hash_field(&mut hasher, &source_state);
    hash_field(&mut hasher, trajectory);
    format!("{:x}", hasher.finalize())
}

fn pad_key(pad: NativeRawPad) -> String {
    let mut bytes = Vec::with_capacity(12);
    encode_pad(pad, &mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn float_bits(value: f32) -> String {
    format!("0x{:08x}", value.to_bits())
}

fn observation_constants(observation: &NativeLearningObservation) -> Vec<(&'static str, String)> {
    vec![
        ("stage", observation.stage.clone()),
        ("room", observation.room.to_string()),
        ("layer", observation.layer.to_string()),
        ("point", observation.point.to_string()),
        ("player_present", observation.player_present.to_string()),
        ("player_is_link", observation.player_is_link.to_string()),
        (
            "player_actor_name",
            observation.player_actor_name.to_string(),
        ),
        ("player_procedure", observation.player_procedure.to_string()),
        (
            "player_mode_flags",
            observation.player_mode_flags.to_string(),
        ),
        ("player_contacts", observation.player_contacts.to_string()),
        (
            "player_position_x",
            float_bits(observation.player_position[0]),
        ),
        (
            "player_position_y",
            float_bits(observation.player_position[1]),
        ),
        (
            "player_position_z",
            float_bits(observation.player_position[2]),
        ),
        (
            "player_velocity_x",
            float_bits(observation.player_velocity[0]),
        ),
        (
            "player_velocity_y",
            float_bits(observation.player_velocity[1]),
        ),
        (
            "player_velocity_z",
            float_bits(observation.player_velocity[2]),
        ),
        (
            "player_forward_speed",
            float_bits(observation.player_forward_speed),
        ),
        (
            "player_current_angle_x",
            observation.player_current_angle[0].to_string(),
        ),
        (
            "player_current_angle_y",
            observation.player_current_angle[1].to_string(),
        ),
        (
            "player_current_angle_z",
            observation.player_current_angle[2].to_string(),
        ),
        ("event_running", observation.event_running.to_string()),
        ("event_id", observation.event_id.to_string()),
        ("event_mode", observation.event_mode.to_string()),
        ("event_status", observation.event_status.to_string()),
        (
            "event_map_tool_id",
            observation.event_map_tool_id.to_string(),
        ),
        ("menu_flags", observation.menu_flags.to_string()),
        ("actor_count", observation.actors.len().to_string()),
        (
            "rng_stream_count",
            observation.rng_streams.len().to_string(),
        ),
        (
            "collision_surface_count",
            observation
                .player_collision_surfaces
                .as_ref()
                .map_or(0, |surfaces| surfaces.surfaces.len())
                .to_string(),
        ),
        ("goal_configured", observation.goal.configured.to_string()),
        ("goal_reached", observation.goal.reached.to_string()),
        ("goal_hit_count", observation.goal.hit_count.to_string()),
        ("switch_flag_room", observation.switch_flag_room.to_string()),
    ]
}

pub fn inspect_native_episode_corpus(shards: &[NativeEpisodeShard]) -> NativeCorpusInspection {
    let mut observation_schemas = BTreeMap::new();
    let mut action_schemas = BTreeMap::new();
    let mut channel_coverage = BTreeMap::<String, ChannelCoverage>::new();
    let mut missing_mask_counts = BTreeMap::<String, u64>::new();
    let mut flag_masks = BTreeMap::<String, FlagAccumulator>::new();
    let mut constant_values = BTreeMap::<String, BTreeSet<String>>::new();
    let mut chosen_pads = BTreeSet::new();
    let mut consumed_pads = BTreeSet::new();
    let mut button_mask_counts = BTreeMap::new();
    let mut stick_sample_counts = BTreeMap::new();
    let mut input_trajectories = BTreeSet::new();
    let mut payload_counts = BTreeMap::<String, (u64, u64, u64)>::new();
    let mut replay_groups = BTreeMap::<String, ReplayGroup>::new();
    let mut actor_types = BTreeSet::new();
    let mut actor_sizes = SetAccumulator::default();
    let mut rng_sizes = SetAccumulator::default();
    let mut surface_sizes = SetAccumulator::default();
    let mut identities = BTreeMap::<String, IdentityValues>::new();
    let mut episode_count = 0_u64;
    let mut success_count = 0_u64;
    let mut failure_count = 0_u64;
    let mut transition_count = 0_u64;
    let mut observation_count = 0_u64;
    let mut terminal_observation_count = 0_u64;
    let mut chosen_consumed_mismatches = 0_u64;
    let mut truncated_actor_observations = 0_u64;

    for shard in shards {
        *observation_schemas
            .entry(shard.metadata.observation_schema.clone())
            .or_default() += 1;
        *action_schemas
            .entry(shard.metadata.action_schema.clone())
            .or_default() += 1;
        for episode in &shard.episodes {
            episode_count += 1;
            success_count += u64::from(episode.success);
            failure_count += u64::from(!episode.success);
            let outcome = episode.success;
            let first = &episode.steps[0].pre_input;
            for (name, value) in [
                (
                    "source_boundary_fingerprint",
                    shard.metadata.source_boundary_fingerprint.clone(),
                ),
                (
                    "checkpoint_identity",
                    shard.metadata.checkpoint_identity.clone(),
                ),
                (
                    "objective_identity",
                    shard.metadata.objective_identity.clone(),
                ),
                ("build_revision", shard.metadata.build_revision.clone()),
                (
                    "game_data_identity",
                    shard
                        .metadata
                        .game_data_identity
                        .clone()
                        .unwrap_or_default(),
                ),
                ("source_frame", shard.source_frame.to_string()),
                ("source_state_identity", hex::encode(first.state_identity)),
                ("source_tape_frame", first.tape_frame.to_string()),
                ("source_simulation_tick", first.simulation_tick.to_string()),
            ] {
                identities
                    .entry(name.into())
                    .or_default()
                    .insert(value, outcome);
            }

            let mut trajectory = Vec::with_capacity(episode.steps.len() * 12);
            for step in &episode.steps {
                transition_count += 1;
                chosen_consumed_mismatches += u64::from(step.chosen_pad != step.consumed_pad);
                chosen_pads.insert(pad_key(step.chosen_pad));
                consumed_pads.insert(pad_key(step.consumed_pad));
                *button_mask_counts
                    .entry(format!("0x{:04x}", step.consumed_pad.buttons))
                    .or_default() += 1;
                *stick_sample_counts
                    .entry(format!(
                        "{},{}",
                        step.consumed_pad.stick_x, step.consumed_pad.stick_y
                    ))
                    .or_default() += 1;
                encode_pad(step.consumed_pad, &mut trajectory);

                for observation in [&step.pre_input, &step.post_simulation] {
                    observation_count += 1;
                    terminal_observation_count += u64::from(
                        observation.terminal_reason
                            != crate::native_episode_shard::NativeTerminalReason::None,
                    );
                    truncated_actor_observations += u64::from(observation.actors_truncated);
                    actor_sizes.push(observation.actors.len());
                    for actor in &observation.actors {
                        actor_types.insert((actor.actor_name, actor.profile_name));
                    }
                    rng_sizes.push(observation.rng_streams.len());
                    for (name, status) in [
                        ("camera", observation.camera_status),
                        ("player_action", observation.player_action_status),
                        (
                            "player_background_collision",
                            observation.player_background_collision_status,
                        ),
                        (
                            "player_collision_surfaces",
                            observation.player_collision_surfaces_status,
                        ),
                        ("scene_exit", observation.scene_exit_status),
                    ] {
                        record_status(channel_coverage.entry(name.into()).or_default(), status);
                    }
                    let surface_count = observation
                        .player_collision_surfaces
                        .as_ref()
                        .map_or(0, |surfaces| surfaces.surfaces.len());
                    surface_sizes.push(surface_count);
                    for (name, values) in [
                        ("event_flags", observation.event_flags.as_deref()),
                        ("temporary_flags", observation.temporary_flags.as_deref()),
                        (
                            "temporary_event_bytes",
                            observation.temporary_event_bytes.as_deref(),
                        ),
                        ("dungeon_flags", observation.dungeon_flags.as_deref()),
                        ("switch_flags", observation.switch_flags.as_deref()),
                    ] {
                        *missing_mask_counts.entry(name.into()).or_default() +=
                            u64::from(values.is_none());
                        flag_masks.entry(name.into()).or_default().push(values);
                    }
                }
                for (name, value) in observation_constants(&step.pre_input) {
                    constant_values
                        .entry(name.into())
                        .or_default()
                        .insert(value);
                }
            }
            let input_digest = format!("{:x}", Sha256::digest(&trajectory));
            input_trajectories.insert(input_digest);
            let payload_digest = hex::encode(episode.payload_xxh3_128);
            let entry = payload_counts.entry(payload_digest.clone()).or_default();
            entry.0 += 1;
            entry.1 += u64::from(episode.success);
            entry.2 += u64::from(!episode.success);
            let replay_digest = replay_key(shard, first.state_identity, &trajectory);
            let replay = replay_groups.entry(replay_digest).or_default();
            replay.payloads.insert(payload_digest);
            replay
                .episode_ids
                .push(format!("{}:{}", shard.content_sha256, episode.id));
        }
    }

    let both_outcomes_present = success_count != 0 && failure_count != 0;
    let identity_unique_values = identities
        .iter()
        .map(|(name, values)| (name.clone(), values.values.len()))
        .collect();
    let outcome_separating_candidates = identities
        .iter()
        .filter(|(_, values)| values.separates_outcomes(both_outcomes_present))
        .map(|(name, _)| name.clone())
        .collect::<Vec<_>>();
    let constant_pre_input_channels = constant_values
        .into_iter()
        .filter_map(|(name, values)| {
            (values.len() == 1).then(|| (name, values.into_iter().next().unwrap()))
        })
        .collect();
    let duplicate_trajectory_groups = payload_counts
        .iter()
        .filter(|(_, (copies, _, _))| *copies > 1)
        .map(
            |(digest, (copies, successes, failures))| DuplicateTrajectoryGroup {
                payload_xxh3_128: digest.clone(),
                copies: *copies,
                successes: *successes,
                failures: *failures,
            },
        )
        .collect::<Vec<_>>();
    let determinism_conflicts = replay_groups
        .into_iter()
        .filter_map(|(digest, group)| {
            (group.payloads.len() > 1).then_some(DeterminismConflictGroup {
                execution_and_consumed_pad_sha256: digest,
                copies: group.episode_ids.len(),
                distinct_payloads: group.payloads.len(),
                episode_ids: group.episode_ids,
            })
        })
        .collect::<Vec<_>>();

    let mut warnings = Vec::new();
    if shards.is_empty() {
        warnings.push("corpus contains no shards".into());
    }
    if success_count == 0 || failure_count == 0 {
        warnings.push("corpus does not contain both success and failure outcomes".into());
    }
    if chosen_pads.len() <= 1 {
        warnings.push("corpus contains at most one chosen PAD state".into());
    }
    if !determinism_conflicts.is_empty() {
        warnings.push(
            "determinism failure: identical execution identity, initial state, and consumed PAD produced different episode payloads"
                .into(),
        );
    }
    if !outcome_separating_candidates.is_empty() {
        warnings.push("identity fields partition outcomes; run leakage ablations".into());
    }
    if channel_coverage
        .values()
        .any(|coverage| coverage.not_sampled == observation_count)
    {
        warnings.push("one or more structured channels are never sampled".into());
    }
    if observation_schemas.len() > 1 || action_schemas.len() > 1 {
        warnings.push("corpus mixes incompatible observation or action schemas".into());
    }

    NativeCorpusInspection {
        schema: NATIVE_CORPUS_INSPECTION_SCHEMA_V1.into(),
        shard_count: shards.len(),
        shard_content_sha256: shards
            .iter()
            .map(|shard| shard.content_sha256.to_string())
            .collect(),
        observation_schemas,
        action_schemas,
        episode_count,
        success_count,
        failure_count,
        transition_count,
        observation_count,
        terminal_observation_count,
        // Non-finite values and phase discontinuities are rejected by the
        // authenticated shard decoder before this report can be constructed.
        validated_non_finite_values: 0,
        validated_phase_discontinuities: 0,
        truncated_actor_observations,
        actor_set_sizes: actor_sizes.finish(),
        rng_stream_set_sizes: rng_sizes.finish(),
        collision_surface_set_sizes: surface_sizes.finish(),
        unique_actor_types: actor_types.len(),
        channel_coverage,
        missing_mask_counts,
        flag_mask_coverage: flag_masks
            .into_iter()
            .map(|(name, accumulator)| (name, accumulator.finish()))
            .collect(),
        constant_pre_input_channels,
        action_coverage: NativeActionCoverage {
            unique_chosen_pad_states: chosen_pads.len(),
            unique_consumed_pad_states: consumed_pads.len(),
            unique_consumed_trajectories: input_trajectories.len(),
            chosen_consumed_mismatches,
            button_mask_counts,
            stick_sample_counts,
        },
        duplicate_trajectory_groups,
        determinism_conflicts,
        identities: NativeIdentityInspection {
            unique_values: identity_unique_values,
            outcome_separating_candidates,
        },
        warnings,
    }
}

// Avoid another public dependency just to render fixed-size identities.
mod hex {
    pub fn encode(bytes: [u8; 16]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_outcomes_duplicates_sets_and_channel_coverage() {
        let bytes =
            include_bytes!("../../../../../tests/fixtures/automation/native_episode_v4.dseps");
        let shard = NativeEpisodeShard::decode(bytes).unwrap();
        let report = inspect_native_episode_corpus(&[shard.clone(), shard]);
        assert_eq!(report.shard_count, 2);
        assert_eq!(report.episode_count, 4);
        assert_eq!(report.success_count, 2);
        assert_eq!(report.failure_count, 2);
        assert_eq!(report.transition_count, 4);
        assert_eq!(report.observation_count, 8);
        assert_eq!(report.truncated_actor_observations, 0);
        assert_eq!(report.actor_set_sizes.minimum, 257);
        assert_eq!(report.actor_set_sizes.maximum, 257);
        assert_eq!(report.unique_actor_types, 1);
        assert_eq!(report.duplicate_trajectory_groups.len(), 2);
        assert!(
            report
                .duplicate_trajectory_groups
                .iter()
                .all(|group| group.copies == 2)
        );
        assert_eq!(report.determinism_conflicts.len(), 1);
        assert_eq!(report.determinism_conflicts[0].copies, 4);
        assert_eq!(report.determinism_conflicts[0].distinct_payloads, 2);
        assert_eq!(report.channel_coverage["camera"].present, 8);
        assert_eq!(report.missing_mask_counts["event_flags"], 0);
        assert_eq!(report.flag_mask_coverage["event_flags"].widths.minimum, 822);
        assert_eq!(report.rng_stream_set_sizes.minimum, 2);
        assert_eq!(report.validated_non_finite_values, 0);
        assert_eq!(report.validated_phase_discontinuities, 0);
    }

    #[test]
    fn flags_identity_fields_that_separate_success_from_failure() {
        let bytes =
            include_bytes!("../../../../../tests/fixtures/automation/native_episode_v4.dseps");
        let shard = NativeEpisodeShard::decode(bytes).unwrap();
        let mut success = shard.clone();
        success.episodes.retain(|episode| episode.success);
        success.metadata.checkpoint_identity = "11111111111111111111111111111111".into();
        let mut failure = shard;
        failure.episodes.retain(|episode| !episode.success);
        failure.metadata.checkpoint_identity = "22222222222222222222222222222222".into();

        let report = inspect_native_episode_corpus(&[success, failure]);
        assert!(
            report
                .identities
                .outcome_separating_candidates
                .iter()
                .any(|field| field == "checkpoint_identity")
        );
        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.contains("leakage ablations"))
        );
        assert!(report.determinism_conflicts.is_empty());
    }

    #[test]
    fn audits_v5_temporary_event_register_coverage() {
        let bytes =
            include_bytes!("../../../../../tests/fixtures/automation/native_episode_v5.dseps");
        let shard = NativeEpisodeShard::decode(bytes).unwrap();
        let report = inspect_native_episode_corpus(&[shard]);
        assert_eq!(report.missing_mask_counts["temporary_event_bytes"], 0);
        let coverage = &report.flag_mask_coverage["temporary_event_bytes"];
        assert_eq!(coverage.present, report.observation_count);
        assert_eq!(coverage.widths.minimum, 256);
        assert_eq!(coverage.widths.maximum, 256);
        assert!(coverage.ever_set_indices.contains(&0));
        assert!(coverage.ever_set_indices.contains(&1));
        assert!(coverage.ever_set_indices.contains(&5));
    }
}
