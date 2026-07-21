//! Corpus-level diagnostics for decoded native learning episodes.
//!
//! The native shard decoder already fails closed on malformed boundaries,
//! non-finite floats, action shifts, and incomplete v4+ actor sets. This module
//! summarizes the surviving corpus so missing/constant channels, poor action
//! coverage, duplicates, and suspicious outcome partitions are visible before
//! learner code is allowed to treat the data as useful.

use crate::native_dynamic_collider_temporal::{
    DynamicColliderTemporalCoverage, inspect_dynamic_collider_temporal_coverage,
};
use crate::native_episode_shard::{
    NativeActorObservation, NativeChannelStatus, NativeEpisode, NativeEpisodeShard,
    NativeLearningObservation, NativeRawPad,
};
use crate::native_global_temporal::{GlobalTemporalCoverage, inspect_global_temporal_coverage};
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const NATIVE_CORPUS_INSPECTION_SCHEMA_V7: &str = "dusklight-native-corpus-inspection/v7";

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
pub struct ActorTemporalProfileCoverage {
    pub profile_name: i16,
    pub actor_names: Vec<i16>,
    pub stages: Vec<String>,
    pub boundary_samples: u64,
    pub episode_local_lifetimes: u64,
    pub persistent_transition_pairs: u64,
    pub in_context_appearances: u64,
    pub in_context_disappearances: u64,
    pub context_change_appearances: u64,
    pub context_change_disappearances: u64,
    pub changed_fields: BTreeMap<String, u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ActorIdentityConflict {
    pub profile_name_before: i16,
    pub profile_name_after: i16,
    pub actor_name_before: i16,
    pub actor_name_after: i16,
    pub occurrences: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ActorTemporalCoverage {
    /// One initial pre-input boundary plus every post-simulation boundary in
    /// each episode. Shared post/pre boundaries are intentionally counted once.
    pub boundary_count: u64,
    pub compared_transition_count: u64,
    pub actor_boundary_samples: u64,
    pub episode_local_lifetimes: u64,
    pub persistent_transition_pairs: u64,
    pub in_context_appearances: u64,
    pub in_context_disappearances: u64,
    pub context_change_appearances: u64,
    pub context_change_disappearances: u64,
    /// A complete actor set must not omit a generation and later reintroduce
    /// it within one episode. Any nonzero value is observer identity drift.
    pub runtime_generation_reappearances: u64,
    pub identity_conflicts: Vec<ActorIdentityConflict>,
    pub profiles: Vec<ActorTemporalProfileCoverage>,
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
    pub dynamic_collider_set_sizes: SetSizeSummary,
    pub unique_actor_types: usize,
    pub actor_temporal_coverage: ActorTemporalCoverage,
    pub dynamic_collider_temporal_coverage: DynamicColliderTemporalCoverage,
    pub global_temporal_coverage: GlobalTemporalCoverage,
    pub channel_coverage: BTreeMap<String, ChannelCoverage>,
    pub player_relationship_role_presence: BTreeMap<String, u64>,
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

#[derive(Default)]
struct ActorTemporalProfileAccumulator {
    actor_names: BTreeSet<i16>,
    stages: BTreeSet<String>,
    boundary_samples: u64,
    episode_local_lifetimes: u64,
    persistent_transition_pairs: u64,
    in_context_appearances: u64,
    in_context_disappearances: u64,
    context_change_appearances: u64,
    context_change_disappearances: u64,
    changed_fields: BTreeMap<String, u64>,
}

#[derive(Default)]
struct ActorTemporalAccumulator {
    boundary_count: u64,
    compared_transition_count: u64,
    actor_boundary_samples: u64,
    episode_local_lifetimes: u64,
    persistent_transition_pairs: u64,
    in_context_appearances: u64,
    in_context_disappearances: u64,
    context_change_appearances: u64,
    context_change_disappearances: u64,
    runtime_generation_reappearances: u64,
    identity_conflicts: BTreeMap<(i16, i16, i16, i16), u64>,
    profiles: BTreeMap<i16, ActorTemporalProfileAccumulator>,
}

impl ActorTemporalAccumulator {
    fn finish(self) -> ActorTemporalCoverage {
        ActorTemporalCoverage {
            boundary_count: self.boundary_count,
            compared_transition_count: self.compared_transition_count,
            actor_boundary_samples: self.actor_boundary_samples,
            episode_local_lifetimes: self.episode_local_lifetimes,
            persistent_transition_pairs: self.persistent_transition_pairs,
            in_context_appearances: self.in_context_appearances,
            in_context_disappearances: self.in_context_disappearances,
            context_change_appearances: self.context_change_appearances,
            context_change_disappearances: self.context_change_disappearances,
            runtime_generation_reappearances: self.runtime_generation_reappearances,
            identity_conflicts: self
                .identity_conflicts
                .into_iter()
                .map(
                    |(
                        (
                            profile_name_before,
                            profile_name_after,
                            actor_name_before,
                            actor_name_after,
                        ),
                        occurrences,
                    )| ActorIdentityConflict {
                        profile_name_before,
                        profile_name_after,
                        actor_name_before,
                        actor_name_after,
                        occurrences,
                    },
                )
                .collect(),
            profiles: self
                .profiles
                .into_iter()
                .map(|(profile_name, profile)| ActorTemporalProfileCoverage {
                    profile_name,
                    actor_names: profile.actor_names.into_iter().collect(),
                    stages: profile.stages.into_iter().collect(),
                    boundary_samples: profile.boundary_samples,
                    episode_local_lifetimes: profile.episode_local_lifetimes,
                    persistent_transition_pairs: profile.persistent_transition_pairs,
                    in_context_appearances: profile.in_context_appearances,
                    in_context_disappearances: profile.in_context_disappearances,
                    context_change_appearances: profile.context_change_appearances,
                    context_change_disappearances: profile.context_change_disappearances,
                    changed_fields: profile.changed_fields,
                })
                .collect(),
        }
    }
}

fn record_changed_field(
    profile: &mut ActorTemporalProfileAccumulator,
    name: &'static str,
    changed: bool,
) {
    if changed {
        *profile.changed_fields.entry(name.into()).or_default() += 1;
    }
}

fn float_changed(left: f32, right: f32) -> bool {
    left.to_bits() != right.to_bits()
}

fn float_array_changed<const N: usize>(left: [f32; N], right: [f32; N]) -> bool {
    left.iter()
        .zip(right)
        .any(|(left, right)| float_changed(*left, right))
}

fn record_persistent_actor_changes(
    profile: &mut ActorTemporalProfileAccumulator,
    before: &NativeActorObservation,
    after: &NativeActorObservation,
) {
    record_changed_field(profile, "actor_name", before.actor_name != after.actor_name);
    record_changed_field(
        profile,
        "profile_name",
        before.profile_name != after.profile_name,
    );
    record_changed_field(profile, "set_id", before.set_id != after.set_id);
    record_changed_field(profile, "home_room", before.home_room != after.home_room);
    record_changed_field(
        profile,
        "current_room",
        before.current_room != after.current_room,
    );
    record_changed_field(profile, "health", before.health != after.health);
    record_changed_field(profile, "status", before.status != after.status);
    record_changed_field(
        profile,
        "position",
        float_array_changed(before.position, after.position),
    );
    record_changed_field(
        profile,
        "current_angle",
        before.current_angle != after.current_angle,
    );
    record_changed_field(
        profile,
        "shape_angle",
        before.shape_angle != after.shape_angle,
    );
    record_changed_field(
        profile,
        "base_state_available",
        before.base_state_available != after.base_state_available,
    );

    if before.base_state_available && after.base_state_available {
        record_changed_field(profile, "actor_type", before.actor_type != after.actor_type);
        record_changed_field(
            profile,
            "process_subtype",
            before.process_subtype != after.process_subtype,
        );
        record_changed_field(
            profile,
            "parent_runtime_generation",
            before.parent_runtime_generation != after.parent_runtime_generation,
        );
        record_changed_field(profile, "parameters", before.parameters != after.parameters);
        record_changed_field(profile, "condition", before.condition != after.condition);
        record_changed_field(profile, "old_room", before.old_room != after.old_room);
        record_changed_field(profile, "group", before.group != after.group);
        record_changed_field(profile, "argument", before.argument != after.argument);
        record_changed_field(profile, "pause_flag", before.pause_flag != after.pause_flag);
        record_changed_field(
            profile,
            "process_init_state",
            before.process_init_state != after.process_init_state,
        );
        record_changed_field(
            profile,
            "process_create_phase",
            before.process_create_phase != after.process_create_phase,
        );
        record_changed_field(profile, "cull_type", before.cull_type != after.cull_type);
        record_changed_field(
            profile,
            "demo_actor_id",
            before.demo_actor_id != after.demo_actor_id,
        );
        record_changed_field(profile, "carry_type", before.carry_type != after.carry_type);
        record_changed_field(
            profile,
            "heap_present",
            before.heap_present != after.heap_present,
        );
        record_changed_field(
            profile,
            "model_present",
            before.model_present != after.model_present,
        );
        record_changed_field(
            profile,
            "joint_collision_present",
            before.joint_collision_present != after.joint_collision_present,
        );
        record_changed_field(
            profile,
            "home_position",
            float_array_changed(before.home_position, after.home_position),
        );
        record_changed_field(
            profile,
            "old_position",
            float_array_changed(before.old_position, after.old_position),
        );
        record_changed_field(
            profile,
            "velocity",
            float_array_changed(before.velocity, after.velocity),
        );
        record_changed_field(
            profile,
            "forward_speed",
            float_changed(before.forward_speed, after.forward_speed),
        );
        record_changed_field(
            profile,
            "scale",
            float_array_changed(before.scale, after.scale),
        );
        record_changed_field(
            profile,
            "gravity",
            float_changed(before.gravity, after.gravity),
        );
        record_changed_field(
            profile,
            "max_fall_speed",
            float_changed(before.max_fall_speed, after.max_fall_speed),
        );
        record_changed_field(
            profile,
            "eye_position",
            float_array_changed(before.eye_position, after.eye_position),
        );
        record_changed_field(profile, "home_angle", before.home_angle != after.home_angle);
        record_changed_field(profile, "old_angle", before.old_angle != after.old_angle);
    }

    record_changed_field(
        profile,
        "attention.present",
        before.attention.is_some() != after.attention.is_some(),
    );
    if let (Some(before), Some(after)) = (&before.attention, &after.attention) {
        record_changed_field(profile, "attention.flags", before.flags != after.flags);
        record_changed_field(
            profile,
            "attention.position",
            float_array_changed(before.position, after.position),
        );
        record_changed_field(
            profile,
            "attention.distance_indices",
            before.distance_indices != after.distance_indices,
        );
        record_changed_field(
            profile,
            "attention.auxiliary",
            before.auxiliary != after.auxiliary,
        );
    }
    record_changed_field(
        profile,
        "event_participation.present",
        before.event_participation.is_some() != after.event_participation.is_some(),
    );
    if let (Some(before), Some(after)) = (&before.event_participation, &after.event_participation) {
        record_changed_field(
            profile,
            "event_participation.command",
            before.command != after.command,
        );
        record_changed_field(
            profile,
            "event_participation.condition",
            before.condition != after.condition,
        );
        record_changed_field(
            profile,
            "event_participation.event_id",
            before.event_id != after.event_id,
        );
        record_changed_field(
            profile,
            "event_participation.map_tool_id",
            before.map_tool_id != after.map_tool_id,
        );
        record_changed_field(
            profile,
            "event_participation.index",
            before.index != after.index,
        );
    }
    record_changed_field(
        profile,
        "return_place_writer",
        before.return_place_writer != after.return_place_writer,
    );
    record_changed_field(
        profile,
        "enemy_base.present",
        before.enemy_base.is_some() != after.enemy_base.is_some(),
    );
    if let (Some(before), Some(after)) = (&before.enemy_base, &after.enemy_base) {
        record_changed_field(profile, "enemy_base.flags", before.flags != after.flags);
        record_changed_field(
            profile,
            "enemy_base.throw_mode",
            before.throw_mode != after.throw_mode,
        );
        record_changed_field(
            profile,
            "enemy_base.down_position",
            float_array_changed(before.down_position, after.down_position),
        );
        record_changed_field(
            profile,
            "enemy_base.head_lock_position",
            float_array_changed(before.head_lock_position, after.head_lock_position),
        );
    }
}

fn record_actor_temporal_episode(
    accumulator: &mut ActorTemporalAccumulator,
    episode: &NativeEpisode,
) {
    let mut boundaries = Vec::with_capacity(episode.steps.len() + 1);
    boundaries.push(&episode.steps[0].pre_input);
    boundaries.extend(episode.steps.iter().map(|step| &step.post_simulation));
    accumulator.boundary_count += boundaries.len() as u64;

    let mut episode_lifetimes = BTreeSet::new();
    let mut seen_generations = BTreeSet::new();
    let mut previous_generations = BTreeSet::new();
    for (boundary_index, observation) in boundaries.iter().enumerate() {
        accumulator.actor_boundary_samples += observation.actors.len() as u64;
        let current_generations = observation
            .actors
            .iter()
            .map(|actor| actor.runtime_generation)
            .collect::<BTreeSet<_>>();
        if boundary_index != 0 {
            accumulator.runtime_generation_reappearances += current_generations
                .iter()
                .filter(|generation| {
                    !previous_generations.contains(*generation)
                        && seen_generations.contains(*generation)
                })
                .count() as u64;
        }
        seen_generations.extend(current_generations.iter().copied());
        previous_generations = current_generations;
        for actor in &observation.actors {
            episode_lifetimes.insert((actor.profile_name, actor.runtime_generation));
            let profile = accumulator.profiles.entry(actor.profile_name).or_default();
            profile.actor_names.insert(actor.actor_name);
            profile.stages.insert(observation.stage.clone());
            profile.boundary_samples += 1;
        }
    }
    accumulator.episode_local_lifetimes += episode_lifetimes.len() as u64;
    for (profile_name, _) in episode_lifetimes {
        accumulator
            .profiles
            .entry(profile_name)
            .or_default()
            .episode_local_lifetimes += 1;
    }

    for pair in boundaries.windows(2) {
        let before = pair[0];
        let after = pair[1];
        accumulator.compared_transition_count += 1;
        let same_context =
            before.stage == after.stage && before.room == after.room && before.layer == after.layer;
        let before_by_id = before
            .actors
            .iter()
            .map(|actor| (actor.runtime_generation, actor))
            .collect::<BTreeMap<_, _>>();
        let after_by_id = after
            .actors
            .iter()
            .map(|actor| (actor.runtime_generation, actor))
            .collect::<BTreeMap<_, _>>();

        for (runtime_generation, actor) in &after_by_id {
            if let Some(previous) = before_by_id.get(runtime_generation) {
                accumulator.persistent_transition_pairs += 1;
                let profile = accumulator
                    .profiles
                    .entry(previous.profile_name)
                    .or_default();
                profile.persistent_transition_pairs += 1;
                record_persistent_actor_changes(profile, previous, actor);
                if previous.profile_name != actor.profile_name
                    || previous.actor_name != actor.actor_name
                {
                    *accumulator
                        .identity_conflicts
                        .entry((
                            previous.profile_name,
                            actor.profile_name,
                            previous.actor_name,
                            actor.actor_name,
                        ))
                        .or_default() += 1;
                }
            } else {
                let profile = accumulator.profiles.entry(actor.profile_name).or_default();
                if same_context {
                    accumulator.in_context_appearances += 1;
                    profile.in_context_appearances += 1;
                } else {
                    accumulator.context_change_appearances += 1;
                    profile.context_change_appearances += 1;
                }
            }
        }
        for (runtime_generation, actor) in &before_by_id {
            if !after_by_id.contains_key(runtime_generation) {
                let profile = accumulator.profiles.entry(actor.profile_name).or_default();
                if same_context {
                    accumulator.in_context_disappearances += 1;
                    profile.in_context_disappearances += 1;
                } else {
                    accumulator.context_change_disappearances += 1;
                    profile.context_change_disappearances += 1;
                }
            }
        }
    }
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
            .game_data_sha256
            .map(|digest| digest.to_string())
            .unwrap_or_default()
            .as_bytes(),
        shard
            .metadata
            .card_fixture_identity
            .as_deref()
            .unwrap_or_default()
            .as_bytes(),
        shard
            .metadata
            .actor_profile_catalog_identity
            .as_deref()
            .unwrap_or_default()
            .as_bytes(),
        shard
            .metadata
            .world_context_sha256
            .map(|digest| digest.to_string())
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
        (
            "dynamic_collider_count",
            observation.dynamic_colliders.len().to_string(),
        ),
        ("goal_configured", observation.goal.configured.to_string()),
        ("goal_reached", observation.goal.reached.to_string()),
        ("goal_hit_count", observation.goal.hit_count.to_string()),
        ("switch_flag_room", observation.switch_flag_room.to_string()),
    ]
}

pub fn inspect_native_episode_corpus(shards: &[NativeEpisodeShard]) -> NativeCorpusInspection {
    let dynamic_collider_temporal_coverage = inspect_dynamic_collider_temporal_coverage(shards);
    let global_temporal_coverage = inspect_global_temporal_coverage(shards);
    let mut observation_schemas = BTreeMap::new();
    let mut action_schemas = BTreeMap::new();
    let mut channel_coverage = BTreeMap::<String, ChannelCoverage>::new();
    let mut player_relationship_role_presence = BTreeMap::<String, u64>::new();
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
    let mut dynamic_collider_sizes = SetAccumulator::default();
    let mut actor_temporal = ActorTemporalAccumulator::default();
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
            record_actor_temporal_episode(&mut actor_temporal, episode);
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
                    "game_data_sha256",
                    shard
                        .metadata
                        .game_data_sha256
                        .map(|digest| digest.to_string())
                        .unwrap_or_default(),
                ),
                (
                    "card_fixture_identity",
                    shard
                        .metadata
                        .card_fixture_identity
                        .clone()
                        .unwrap_or_default(),
                ),
                (
                    "actor_profile_catalog_identity",
                    shard
                        .metadata
                        .actor_profile_catalog_identity
                        .clone()
                        .unwrap_or_default(),
                ),
                (
                    "world_context_sha256",
                    shard
                        .metadata
                        .world_context_sha256
                        .map(|digest| digest.to_string())
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
                        ("dynamic_colliders", observation.dynamic_colliders_status),
                        ("player_resources", observation.player_resources_status),
                        (
                            "player_relationships",
                            observation.player_relationships_status,
                        ),
                        (
                            "player_collision_solver",
                            observation.player_collision_solver_status,
                        ),
                        ("message_session", observation.message_session_status),
                    ] {
                        record_status(channel_coverage.entry(name.into()).or_default(), status);
                    }
                    let surface_count = observation
                        .player_collision_surfaces
                        .as_ref()
                        .map_or(0, |surfaces| surfaces.surfaces.len());
                    surface_sizes.push(surface_count);
                    dynamic_collider_sizes.push(observation.dynamic_colliders.len());
                    let resources = observation.player_resources.as_ref();
                    let solver_flags = observation
                        .player_collision_solver
                        .as_ref()
                        .map(|solver| solver.flags.to_le_bytes());
                    let solver_wall_flags =
                        observation.player_collision_solver.as_ref().map(|solver| {
                            solver
                                .wall_circles
                                .iter()
                                .flat_map(|wall| wall.flags.to_le_bytes())
                                .collect::<Vec<_>>()
                        });
                    if let Some(relationships) = observation.player_relationships.as_ref() {
                        for (role, identity) in [
                            ("targeted_actor", &relationships.targeted_actor),
                            ("ride_actor", &relationships.ride_actor),
                            ("held_item_actor", &relationships.held_item_actor),
                            ("grabbed_actor", &relationships.grabbed_actor),
                            (
                                "thrown_boomerang_actor",
                                &relationships.thrown_boomerang_actor,
                            ),
                            ("copy_rod_actor", &relationships.copy_rod_actor),
                            (
                                "hookshot_roof_wait_actor",
                                &relationships.hookshot_roof_wait_actor,
                            ),
                            ("chain_grab_actor", &relationships.chain_grab_actor),
                            ("attention_hint_actor", &relationships.attention_hint_actor),
                            (
                                "attention_catch_actor",
                                &relationships.attention_catch_actor,
                            ),
                            ("attention_look_actor", &relationships.attention_look_actor),
                        ] {
                            *player_relationship_role_presence
                                .entry(role.into())
                                .or_default() += u64::from(identity.is_some());
                        }
                    }
                    for (name, values) in [
                        ("event_flags", observation.event_flags.as_deref()),
                        ("temporary_flags", observation.temporary_flags.as_deref()),
                        (
                            "temporary_event_bytes",
                            observation.temporary_event_bytes.as_deref(),
                        ),
                        ("dungeon_flags", observation.dungeon_flags.as_deref()),
                        ("switch_flags", observation.switch_flags.as_deref()),
                        (
                            "acquired_item_bits",
                            resources.map(|value| value.acquired_item_bits.as_slice()),
                        ),
                        (
                            "collect_item_bits",
                            resources.map(|value| value.collect_item_bits.as_slice()),
                        ),
                        (
                            "player_collision_solver_flags",
                            solver_flags.as_ref().map(|value| value.as_slice()),
                        ),
                        (
                            "player_collision_solver_wall_flags",
                            solver_wall_flags.as_deref(),
                        ),
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

    let actor_temporal_coverage = actor_temporal.finish();
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
    if actor_temporal_coverage.runtime_generation_reappearances != 0
        || !actor_temporal_coverage.identity_conflicts.is_empty()
    {
        warnings.push("actor runtime-generation identity is inconsistent within an episode".into());
    }
    if dynamic_collider_temporal_coverage.duplicate_identity_boundaries != 0 {
        warnings.push("dynamic collider identities are duplicated within a boundary".into());
    }
    if dynamic_collider_temporal_coverage.unresolved_owner_samples != 0 {
        warnings.push("dynamic collider owners do not join the complete actor set".into());
    }

    NativeCorpusInspection {
        schema: NATIVE_CORPUS_INSPECTION_SCHEMA_V7.into(),
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
        dynamic_collider_set_sizes: dynamic_collider_sizes.finish(),
        unique_actor_types: actor_types.len(),
        actor_temporal_coverage,
        dynamic_collider_temporal_coverage,
        global_temporal_coverage,
        channel_coverage,
        player_relationship_role_presence,
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

    #[test]
    fn audits_v9_player_resource_and_inventory_coverage() {
        let bytes =
            include_bytes!("../../../../../tests/fixtures/automation/native_episode_v9.dseps");
        let shard = NativeEpisodeShard::decode(bytes).unwrap();
        let report = inspect_native_episode_corpus(&[shard]);
        assert_eq!(report.schema, NATIVE_CORPUS_INSPECTION_SCHEMA_V7);
        assert_eq!(
            report.channel_coverage["player_resources"].present,
            report.observation_count
        );
        assert_eq!(report.missing_mask_counts["acquired_item_bits"], 0);
        assert_eq!(report.missing_mask_counts["collect_item_bits"], 0);
        assert_eq!(
            report.flag_mask_coverage["acquired_item_bits"]
                .widths
                .minimum,
            32
        );
        assert_eq!(
            report.flag_mask_coverage["collect_item_bits"]
                .widths
                .minimum,
            8
        );
    }

    #[test]
    fn audits_v10_player_relationship_role_coverage() {
        let bytes =
            include_bytes!("../../../../../tests/fixtures/automation/native_episode_v10.dseps");
        let shard = NativeEpisodeShard::decode(bytes).unwrap();
        let report = inspect_native_episode_corpus(&[shard]);
        assert_eq!(report.schema, NATIVE_CORPUS_INSPECTION_SCHEMA_V7);
        assert_eq!(
            report.channel_coverage["player_relationships"].present,
            report.observation_count
        );
        assert_eq!(
            report.player_relationship_role_presence["targeted_actor"],
            report.observation_count
        );
        assert_eq!(report.player_relationship_role_presence["ride_actor"], 0);
    }

    #[test]
    fn audits_v11_player_collision_solver_coverage() {
        let bytes =
            include_bytes!("../../../../../tests/fixtures/automation/native_episode_v11.dseps");
        let shard = NativeEpisodeShard::decode(bytes).unwrap();
        let report = inspect_native_episode_corpus(&[shard]);
        assert_eq!(
            report.channel_coverage["player_collision_solver"].present,
            report.observation_count
        );
        assert_eq!(
            report.missing_mask_counts["player_collision_solver_flags"],
            0
        );
        assert_eq!(
            report.flag_mask_coverage["player_collision_solver_flags"]
                .widths
                .minimum,
            4
        );
        assert_eq!(
            report.flag_mask_coverage["player_collision_solver_wall_flags"]
                .widths
                .minimum,
            12
        );
    }

    #[test]
    fn audits_actor_lifecycles_and_typed_changes_without_raw_values() {
        let bytes =
            include_bytes!("../../../../../tests/fixtures/automation/native_episode_v14.dseps");
        let mut shard = NativeEpisodeShard::decode(bytes).unwrap();
        shard.episodes.truncate(1);
        shard.episodes[0].steps.truncate(1);
        let step = &mut shard.episodes[0].steps[0];
        let persistent = step.pre_input.actors[0].clone();

        let mut disappeared = persistent.clone();
        disappeared.runtime_generation += 10_000;
        step.pre_input.actors.push(disappeared);
        step.pre_input
            .actors
            .sort_by_key(|actor| actor.runtime_generation);

        let after_persistent = step
            .post_simulation
            .actors
            .iter_mut()
            .find(|actor| actor.runtime_generation == persistent.runtime_generation)
            .unwrap();
        after_persistent.position[0] += 1.0;
        after_persistent.velocity[2] += 2.0;

        let mut appeared = persistent.clone();
        appeared.runtime_generation += 20_000;
        appeared.profile_name += 1;
        appeared.actor_name += 1;
        step.post_simulation.actors.push(appeared.clone());
        step.post_simulation
            .actors
            .sort_by_key(|actor| actor.runtime_generation);

        let report = inspect_native_episode_corpus(&[shard]);
        let temporal = &report.actor_temporal_coverage;
        assert_eq!(temporal.boundary_count, 2);
        assert_eq!(temporal.compared_transition_count, 1);
        assert_eq!(temporal.in_context_appearances, 1);
        assert_eq!(temporal.in_context_disappearances, 1);
        assert_eq!(temporal.context_change_appearances, 0);
        assert_eq!(temporal.context_change_disappearances, 0);
        assert!(temporal.identity_conflicts.is_empty());

        let persistent_profile = temporal
            .profiles
            .iter()
            .find(|profile| profile.profile_name == persistent.profile_name)
            .unwrap();
        assert_eq!(persistent_profile.changed_fields["position"], 1);
        assert_eq!(persistent_profile.changed_fields["velocity"], 1);
        assert_eq!(persistent_profile.in_context_disappearances, 1);
        assert!(!persistent_profile.changed_fields.contains_key("health"));

        let appeared_profile = temporal
            .profiles
            .iter()
            .find(|profile| profile.profile_name == appeared.profile_name)
            .unwrap();
        assert_eq!(appeared_profile.in_context_appearances, 1);
        assert_eq!(appeared_profile.boundary_samples, 1);
    }

    #[test]
    fn separates_context_teardown_from_in_context_actor_lifecycle() {
        let bytes =
            include_bytes!("../../../../../tests/fixtures/automation/native_episode_v14.dseps");
        let mut shard = NativeEpisodeShard::decode(bytes).unwrap();
        shard.episodes.truncate(1);
        shard.episodes[0].steps.truncate(1);
        let step = &mut shard.episodes[0].steps[0];
        let removed = step.pre_input.actors[0].runtime_generation;
        step.post_simulation.room = step.pre_input.room.wrapping_add(1);
        step.post_simulation
            .actors
            .retain(|actor| actor.runtime_generation != removed);
        let mut appeared = step.pre_input.actors[0].clone();
        appeared.runtime_generation += 30_000;
        step.post_simulation.actors.push(appeared);
        step.post_simulation
            .actors
            .sort_by_key(|actor| actor.runtime_generation);

        let report = inspect_native_episode_corpus(&[shard]);
        let temporal = report.actor_temporal_coverage;
        assert_eq!(temporal.in_context_appearances, 0);
        assert_eq!(temporal.in_context_disappearances, 0);
        assert_eq!(temporal.context_change_appearances, 1);
        assert_eq!(temporal.context_change_disappearances, 1);
    }

    #[test]
    fn flags_a_runtime_generation_that_reappears_after_omission() {
        let bytes =
            include_bytes!("../../../../../tests/fixtures/automation/native_episode_v14.dseps");
        let mut shard = NativeEpisodeShard::decode(bytes).unwrap();
        shard.episodes.truncate(1);
        let second_step = shard.episodes[0].steps[0].clone();
        shard.episodes[0].steps.push(second_step);
        let generation = shard.episodes[0].steps[0].pre_input.actors[0].runtime_generation;
        let actor = shard.episodes[0].steps[0].pre_input.actors[0].clone();
        shard.episodes[0].steps[0]
            .post_simulation
            .actors
            .retain(|candidate| candidate.runtime_generation != generation);
        if !shard.episodes[0].steps[1]
            .post_simulation
            .actors
            .iter()
            .any(|candidate| candidate.runtime_generation == generation)
        {
            shard.episodes[0].steps[1]
                .post_simulation
                .actors
                .push(actor);
            shard.episodes[0].steps[1]
                .post_simulation
                .actors
                .sort_by_key(|candidate| candidate.runtime_generation);
        }

        let report = inspect_native_episode_corpus(&[shard]);
        assert_eq!(
            report
                .actor_temporal_coverage
                .runtime_generation_reappearances,
            1
        );
        assert!(report.warnings.iter().any(|warning| {
            warning.contains("actor runtime-generation identity is inconsistent within an episode")
        }));
    }
}
