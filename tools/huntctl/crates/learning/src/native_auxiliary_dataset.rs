//! Phase-correct auxiliary-training rows over complete native observations.
//!
//! Rows reference authenticated `.dseps` pre-input and post-simulation
//! observations instead of flattening them. Generic targets are materialized
//! separately so representation learners can train on broad state without
//! accidentally admitting post-simulation evidence into policy inputs.

use crate::artifact::Digest;
use crate::native_replay_corpus::{NativeReplayCorpus, NativeReplayEntry, ReplayExperienceRole};
use dusklight_evidence::native_episode_shard::{
    NativeEpisode, NativeEpisodeShard, NativeEpisodeStep, NativeLearningObservation, NativeRawPad,
    NativeTerminalReason,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

pub const NATIVE_AUXILIARY_DATASET_SCHEMA_V2: &str = "dusklight-native-auxiliary-dataset/v2";
pub const NATIVE_AUXILIARY_EXAMPLE_SCHEMA_V2: &str = "dusklight-native-auxiliary-example/v2";
const MAX_AUXILIARY_EXAMPLES: usize = 16_000_000;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuxiliarySplit {
    Training,
    Validation,
    Test,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuxiliarySplitConfig {
    pub training_basis_points: u16,
    pub validation_basis_points: u16,
    pub seed: u64,
}

impl Default for AuxiliarySplitConfig {
    fn default() -> Self {
        Self {
            training_basis_points: 8_000,
            validation_basis_points: 1_000,
            seed: 0x4155_5844_4154_4101,
        }
    }
}

impl AuxiliarySplitConfig {
    fn validate(self) -> Result<(), NativeAuxiliaryDatasetError> {
        if self.training_basis_points == 0
            || self.validation_basis_points == 0
            || u32::from(self.training_basis_points) + u32::from(self.validation_basis_points)
                >= 10_000
        {
            return Err(NativeAuxiliaryDatasetError::new(
                "auxiliary split requires nonzero train, validation, and test ranges",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuxiliaryPadTarget {
    pub buttons: u16,
    pub stick_x: i8,
    pub stick_y: i8,
    pub substick_x: i8,
    pub substick_y: i8,
    pub trigger_left: u8,
    pub trigger_right: u8,
    pub analog_a: u8,
    pub analog_b: u8,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerDynamicsTarget {
    pub position_delta: [f32; 3],
    pub velocity_delta: [f32; 3],
    pub forward_speed_delta: f32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContactTransitionTarget {
    pub activated: u8,
    pub cleared: u8,
    pub before: u8,
    pub after: u8,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActorLifecycleTarget {
    pub appeared_runtime_generations: Vec<u64>,
    pub disappeared_runtime_generations: Vec<u64>,
    pub retained_count: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActionPhaseTarget {
    pub procedure_before: u16,
    pub procedure_after: u16,
    pub mode_flags_activated: u32,
    pub mode_flags_cleared: u32,
    pub do_status_before: u8,
    pub do_status_after: u8,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EventLoadingTarget {
    pub event_running_before: bool,
    pub event_running_after: bool,
    pub event_id_before: i16,
    pub event_id_after: i16,
    pub stage_before: String,
    pub stage_after: String,
    pub room_before: i8,
    pub room_after: i8,
    pub layer_before: i8,
    pub layer_after: i8,
    pub point_before: i16,
    pub point_after: i16,
    pub pending_stage_before: Option<String>,
    pub pending_stage_after: Option<String>,
    pub goal_reached_after: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ShortHorizonReachabilityTarget {
    pub ticks_to_terminal: Option<u32>,
    pub within_1_tick: bool,
    pub within_2_ticks: bool,
    pub within_4_ticks: bool,
    pub within_8_ticks: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuxiliaryComparability {
    pub same_context: bool,
    pub same_player: bool,
    pub link_contacts: bool,
    pub actor_lifecycle: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeAuxiliaryTargets {
    pub comparability: AuxiliaryComparability,
    pub inverse_action: AuxiliaryPadTarget,
    pub player_dynamics: Option<PlayerDynamicsTarget>,
    pub contacts: Option<ContactTransitionTarget>,
    pub actor_lifecycle: Option<ActorLifecycleTarget>,
    pub action_phase: Option<ActionPhaseTarget>,
    pub event_loading: EventLoadingTarget,
    pub reachability: ShortHorizonReachabilityTarget,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeAuxiliaryExample {
    pub schema: String,
    pub example_sha256: Digest,
    pub replay_entry_sha256: Digest,
    pub shard_sha256: Digest,
    pub episode_id: String,
    pub episode_payload_xxh3_128: String,
    pub role: ReplayExperienceRole,
    pub step_index: u32,
    pub split: AuxiliarySplit,
    pub pre_input_state_xxh3_128: String,
    pub post_simulation_state_xxh3_128: String,
    pub targets: NativeAuxiliaryTargets,
}

impl NativeAuxiliaryExample {
    fn build(
        entry: &NativeReplayEntry,
        episode: &NativeEpisode,
        step_index: usize,
        split: AuxiliarySplit,
    ) -> Result<Self, NativeAuxiliaryDatasetError> {
        let step = episode
            .steps
            .get(step_index)
            .ok_or_else(|| NativeAuxiliaryDatasetError::new("auxiliary step index is invalid"))?;
        let step_index_u32 = u32::try_from(step_index)
            .map_err(|_| NativeAuxiliaryDatasetError::new("auxiliary step index overflowed"))?;
        let mut example = Self {
            schema: NATIVE_AUXILIARY_EXAMPLE_SCHEMA_V2.into(),
            example_sha256: Digest::ZERO,
            replay_entry_sha256: entry.entry_sha256,
            shard_sha256: entry.shard_sha256,
            episode_id: episode.id.clone(),
            episode_payload_xxh3_128: entry.episode_payload_xxh3_128.clone(),
            role: entry.role,
            step_index: step_index_u32,
            split,
            pre_input_state_xxh3_128: hex_128(step.pre_input.state_identity),
            post_simulation_state_xxh3_128: hex_128(step.post_simulation.state_identity),
            targets: targets(episode, step_index, step)?,
        };
        example.example_sha256 = example.digest()?;
        example.validate()?;
        Ok(example)
    }

    pub fn validate(&self) -> Result<(), NativeAuxiliaryDatasetError> {
        if self.schema != NATIVE_AUXILIARY_EXAMPLE_SCHEMA_V2
            || self.example_sha256 == Digest::ZERO
            || self.replay_entry_sha256 == Digest::ZERO
            || self.shard_sha256 == Digest::ZERO
            || self.episode_id.is_empty()
            || self.episode_payload_xxh3_128.len() != 32
            || !is_lower_hex(&self.episode_payload_xxh3_128)
            || self.pre_input_state_xxh3_128.len() != 32
            || !is_lower_hex(&self.pre_input_state_xxh3_128)
            || self.post_simulation_state_xxh3_128.len() != 32
            || !is_lower_hex(&self.post_simulation_state_xxh3_128)
            || !targets_are_valid(&self.targets)
            || self.example_sha256 != self.digest()?
        {
            return Err(NativeAuxiliaryDatasetError::new(
                "native auxiliary example is invalid or detached",
            ));
        }
        Ok(())
    }

    fn digest(&self) -> Result<Digest, NativeAuxiliaryDatasetError> {
        canonical_digest(
            b"dusklight.native-auxiliary-example/v2\0",
            &(
                &self.schema,
                self.replay_entry_sha256,
                self.shard_sha256,
                &self.episode_id,
                &self.episode_payload_xxh3_128,
                self.role,
                self.step_index,
                self.split,
                &self.pre_input_state_xxh3_128,
                &self.post_simulation_state_xxh3_128,
                &self.targets,
            ),
        )
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeAuxiliaryDatasetReport {
    pub examples: usize,
    pub episodes: usize,
    pub splits: BTreeMap<AuxiliarySplit, usize>,
    pub roles: BTreeMap<ReplayExperienceRole, usize>,
    pub player_dynamics_targets: usize,
    pub contact_targets: usize,
    pub action_phase_targets: usize,
    pub actor_appearances: u64,
    pub actor_disappearances: u64,
    pub terminal_reachable_examples: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AuxiliarySplitDiagnostics {
    pub examples: usize,
    pub episodes: usize,
    pub distinct_consumed_pad_states: usize,
    pub player_motion_changes: usize,
    pub contact_changes: usize,
    pub procedure_changes: usize,
    pub mode_flag_changes: usize,
    pub event_or_loading_changes: usize,
    pub actor_appearances: u64,
    pub actor_disappearances: u64,
    pub goal_reachable_examples: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeAuxiliaryDataset {
    pub schema: String,
    pub replay_corpus_sha256: Digest,
    pub observation_schema: String,
    pub action_schema: String,
    pub split_config: AuxiliarySplitConfig,
    pub examples: Vec<NativeAuxiliaryExample>,
    pub report: NativeAuxiliaryDatasetReport,
    pub dataset_sha256: Digest,
}

impl NativeAuxiliaryDataset {
    pub fn build(
        corpus: &NativeReplayCorpus,
        shards: &[NativeEpisodeShard],
        split_config: AuxiliarySplitConfig,
    ) -> Result<Self, NativeAuxiliaryDatasetError> {
        corpus
            .validate()
            .map_err(|error| NativeAuxiliaryDatasetError::new(error.to_string()))?;
        split_config.validate()?;
        let mut shard_by_digest = BTreeMap::new();
        for shard in shards {
            if shard_by_digest
                .insert(shard.content_sha256, shard)
                .is_some()
            {
                return Err(NativeAuxiliaryDatasetError::new(
                    "auxiliary dataset received a duplicate shard",
                ));
            }
        }
        let mut examples = Vec::new();
        for entry in &corpus.entries {
            let shard = shard_by_digest.get(&entry.shard_sha256).ok_or_else(|| {
                NativeAuxiliaryDatasetError::new(format!(
                    "auxiliary dataset is missing shard {}",
                    entry.shard_sha256
                ))
            })?;
            if shard.metadata.observation_schema != corpus.observation_schema
                || shard.metadata.action_schema != corpus.action_schema
            {
                return Err(NativeAuxiliaryDatasetError::new(
                    "auxiliary shard schema disagrees with replay corpus",
                ));
            }
            let episode = shard
                .episodes
                .iter()
                .find(|episode| episode.id == entry.episode_id)
                .ok_or_else(|| {
                    NativeAuxiliaryDatasetError::new("auxiliary replay episode is absent")
                })?;
            if hex_128(episode.payload_xxh3_128) != entry.episode_payload_xxh3_128
                || episode.steps.len() != entry.ticks_executed as usize
                || episode.success != entry.success
            {
                return Err(NativeAuxiliaryDatasetError::new(
                    "auxiliary replay episode identity or outcome is detached",
                ));
            }
            let split = split_for(entry, split_config);
            for step_index in 0..episode.steps.len() {
                examples.push(NativeAuxiliaryExample::build(
                    entry, episode, step_index, split,
                )?);
                if examples.len() > MAX_AUXILIARY_EXAMPLES {
                    return Err(NativeAuxiliaryDatasetError::new(
                        "auxiliary example limit exceeded",
                    ));
                }
            }
        }
        examples.sort_by_key(|example| example.example_sha256);
        let report = report(&examples)?;
        let mut dataset = Self {
            schema: NATIVE_AUXILIARY_DATASET_SCHEMA_V2.into(),
            replay_corpus_sha256: corpus.corpus_sha256,
            observation_schema: corpus.observation_schema.clone(),
            action_schema: corpus.action_schema.clone(),
            split_config,
            examples,
            report,
            dataset_sha256: Digest::ZERO,
        };
        dataset.dataset_sha256 = dataset.digest()?;
        dataset.validate()?;
        Ok(dataset)
    }

    pub fn validate(&self) -> Result<(), NativeAuxiliaryDatasetError> {
        self.split_config.validate()?;
        if self.schema != NATIVE_AUXILIARY_DATASET_SCHEMA_V2
            || self.replay_corpus_sha256 == Digest::ZERO
            || self.observation_schema.is_empty()
            || self.action_schema.is_empty()
            || self.examples.is_empty()
            || self.examples.len() > MAX_AUXILIARY_EXAMPLES
            || !self
                .examples
                .windows(2)
                .all(|pair| pair[0].example_sha256 < pair[1].example_sha256)
            || self
                .examples
                .iter()
                .any(|example| example.validate().is_err())
            || self.report != report(&self.examples)?
            || self.dataset_sha256 != self.digest()?
        {
            return Err(NativeAuxiliaryDatasetError::new(
                "native auxiliary dataset is invalid or detached",
            ));
        }
        let mut episode_splits = BTreeMap::<&str, AuxiliarySplit>::new();
        for example in &self.examples {
            if episode_splits
                .insert(&example.episode_payload_xxh3_128, example.split)
                .is_some_and(|prior| prior != example.split)
            {
                return Err(NativeAuxiliaryDatasetError::new(
                    "auxiliary dataset splits one episode payload across partitions",
                ));
            }
        }
        Ok(())
    }

    pub fn split_diagnostics(
        &self,
    ) -> Result<BTreeMap<AuxiliarySplit, AuxiliarySplitDiagnostics>, NativeAuxiliaryDatasetError>
    {
        self.validate()?;
        let mut output = BTreeMap::new();
        for split in [
            AuxiliarySplit::Training,
            AuxiliarySplit::Validation,
            AuxiliarySplit::Test,
        ] {
            let examples = self
                .examples
                .iter()
                .filter(|example| example.split == split)
                .collect::<Vec<_>>();
            let actor_appearances = checked_lifecycle_count(&examples, true)?;
            let actor_disappearances = checked_lifecycle_count(&examples, false)?;
            output.insert(
                split,
                AuxiliarySplitDiagnostics {
                    examples: examples.len(),
                    episodes: examples
                        .iter()
                        .map(|example| example.episode_payload_xxh3_128.as_str())
                        .collect::<BTreeSet<_>>()
                        .len(),
                    distinct_consumed_pad_states: examples
                        .iter()
                        .map(|example| example.targets.inverse_action)
                        .collect::<BTreeSet<_>>()
                        .len(),
                    player_motion_changes: examples
                        .iter()
                        .filter(|example| {
                            example
                                .targets
                                .player_dynamics
                                .as_ref()
                                .is_some_and(|target| {
                                    target
                                        .position_delta
                                        .iter()
                                        .chain(&target.velocity_delta)
                                        .chain(std::iter::once(&target.forward_speed_delta))
                                        .any(|value| *value != 0.0)
                                })
                        })
                        .count(),
                    contact_changes: examples
                        .iter()
                        .filter(|example| {
                            example
                                .targets
                                .contacts
                                .as_ref()
                                .is_some_and(|target| target.activated != 0 || target.cleared != 0)
                        })
                        .count(),
                    procedure_changes: examples
                        .iter()
                        .filter(|example| {
                            example.targets.action_phase.as_ref().is_some_and(|target| {
                                target.procedure_before != target.procedure_after
                            })
                        })
                        .count(),
                    mode_flag_changes: examples
                        .iter()
                        .filter(|example| {
                            example.targets.action_phase.as_ref().is_some_and(|target| {
                                target.mode_flags_activated != 0 || target.mode_flags_cleared != 0
                            })
                        })
                        .count(),
                    event_or_loading_changes: examples
                        .iter()
                        .filter(|example| event_or_loading_changed(&example.targets.event_loading))
                        .count(),
                    actor_appearances,
                    actor_disappearances,
                    goal_reachable_examples: examples
                        .iter()
                        .filter(|example| example.targets.reachability.ticks_to_terminal.is_some())
                        .count(),
                },
            );
        }
        Ok(output)
    }

    fn digest(&self) -> Result<Digest, NativeAuxiliaryDatasetError> {
        canonical_digest(
            b"dusklight.native-auxiliary-dataset/v2\0",
            &(
                &self.schema,
                self.replay_corpus_sha256,
                &self.observation_schema,
                &self.action_schema,
                self.split_config,
                &self.examples,
                &self.report,
            ),
        )
    }
}

fn checked_lifecycle_count(
    examples: &[&NativeAuxiliaryExample],
    appearances: bool,
) -> Result<u64, NativeAuxiliaryDatasetError> {
    examples.iter().try_fold(0_u64, |total, example| {
        let count = if appearances {
            example
                .targets
                .actor_lifecycle
                .as_ref()
                .map_or(0, |target| target.appeared_runtime_generations.len())
        } else {
            example
                .targets
                .actor_lifecycle
                .as_ref()
                .map_or(0, |target| target.disappeared_runtime_generations.len())
        };
        total
            .checked_add(count as u64)
            .ok_or_else(|| NativeAuxiliaryDatasetError::new("lifecycle count overflowed"))
    })
}

fn event_or_loading_changed(target: &EventLoadingTarget) -> bool {
    target.event_running_before != target.event_running_after
        || target.event_id_before != target.event_id_after
        || target.stage_before != target.stage_after
        || target.room_before != target.room_after
        || target.layer_before != target.layer_after
        || target.point_before != target.point_after
        || target.pending_stage_before != target.pending_stage_after
        || target.goal_reached_after
}

fn targets(
    episode: &NativeEpisode,
    step_index: usize,
    step: &NativeEpisodeStep,
) -> Result<NativeAuxiliaryTargets, NativeAuxiliaryDatasetError> {
    let before = &step.pre_input;
    let after = &step.post_simulation;
    let same_context =
        before.stage == after.stage && before.room == after.room && before.layer == after.layer;
    let same_player = same_context
        && before.player_present
        && after.player_present
        && before.player_process_id == after.player_process_id
        && before.player_actor_name == after.player_actor_name;
    let link_contacts = same_player && before.player_is_link && after.player_is_link;
    let player_dynamics = same_player.then(|| PlayerDynamicsTarget {
        position_delta: subtract3(after.player_position, before.player_position),
        velocity_delta: subtract3(after.player_velocity, before.player_velocity),
        forward_speed_delta: after.player_forward_speed - before.player_forward_speed,
    });
    let contacts = link_contacts.then_some(ContactTransitionTarget {
        activated: after.player_contacts & !before.player_contacts,
        cleared: before.player_contacts & !after.player_contacts,
        before: before.player_contacts,
        after: after.player_contacts,
    });
    let action_phase = same_player.then_some(ActionPhaseTarget {
        procedure_before: before.player_procedure,
        procedure_after: after.player_procedure,
        mode_flags_activated: after.player_mode_flags & !before.player_mode_flags,
        mode_flags_cleared: before.player_mode_flags & !after.player_mode_flags,
        do_status_before: before.player_do_status,
        do_status_after: after.player_do_status,
    });
    Ok(NativeAuxiliaryTargets {
        comparability: AuxiliaryComparability {
            same_context,
            same_player,
            link_contacts,
            actor_lifecycle: same_context,
        },
        inverse_action: pad_target(step.consumed_pad)?,
        player_dynamics,
        contacts,
        actor_lifecycle: same_context
            .then(|| actor_lifecycle(before, after))
            .transpose()?,
        action_phase,
        event_loading: EventLoadingTarget {
            event_running_before: before.event_running,
            event_running_after: after.event_running,
            event_id_before: before.event_id,
            event_id_after: after.event_id,
            stage_before: before.stage.clone(),
            stage_after: after.stage.clone(),
            room_before: before.room,
            room_after: after.room,
            layer_before: before.layer,
            layer_after: after.layer,
            point_before: before.point,
            point_after: after.point,
            pending_stage_before: before.next_stage.clone(),
            pending_stage_after: after.next_stage.clone(),
            goal_reached_after: after.terminal_reason == NativeTerminalReason::GoalReached,
        },
        reachability: reachability(episode, step_index)?,
    })
}

fn pad_target(pad: NativeRawPad) -> Result<AuxiliaryPadTarget, NativeAuxiliaryDatasetError> {
    if !pad.connected || pad.error != 0 {
        return Err(NativeAuxiliaryDatasetError::new(
            "auxiliary inverse-action target requires a connected error-free PAD",
        ));
    }
    Ok(AuxiliaryPadTarget {
        buttons: pad.buttons,
        stick_x: pad.stick_x,
        stick_y: pad.stick_y,
        substick_x: pad.substick_x,
        substick_y: pad.substick_y,
        trigger_left: pad.trigger_left,
        trigger_right: pad.trigger_right,
        analog_a: pad.analog_a,
        analog_b: pad.analog_b,
    })
}

fn actor_lifecycle(
    before: &NativeLearningObservation,
    after: &NativeLearningObservation,
) -> Result<ActorLifecycleTarget, NativeAuxiliaryDatasetError> {
    if before.actors_truncated || after.actors_truncated {
        return Err(NativeAuxiliaryDatasetError::new(
            "auxiliary actor lifecycle requires complete actor populations",
        ));
    }
    let before_ids = before
        .actors
        .iter()
        .map(|actor| actor.runtime_generation)
        .collect::<BTreeSet<_>>();
    let after_ids = after
        .actors
        .iter()
        .map(|actor| actor.runtime_generation)
        .collect::<BTreeSet<_>>();
    Ok(ActorLifecycleTarget {
        appeared_runtime_generations: after_ids.difference(&before_ids).copied().collect(),
        disappeared_runtime_generations: before_ids.difference(&after_ids).copied().collect(),
        retained_count: u32::try_from(before_ids.intersection(&after_ids).count())
            .map_err(|_| NativeAuxiliaryDatasetError::new("retained actor count overflowed"))?,
    })
}

fn reachability(
    episode: &NativeEpisode,
    step_index: usize,
) -> Result<ShortHorizonReachabilityTarget, NativeAuxiliaryDatasetError> {
    let ticks_to_terminal = episode
        .first_hit_tick
        .map(|hit| {
            let step = u32::try_from(step_index)
                .map_err(|_| NativeAuxiliaryDatasetError::new("reachability step overflowed"))?;
            hit.checked_sub(step)
                .and_then(|remaining| remaining.checked_add(1))
                .ok_or_else(|| {
                    NativeAuxiliaryDatasetError::new("terminal tick precedes auxiliary decision")
                })
        })
        .transpose()?;
    Ok(ShortHorizonReachabilityTarget {
        ticks_to_terminal,
        within_1_tick: ticks_to_terminal.is_some_and(|ticks| ticks <= 1),
        within_2_ticks: ticks_to_terminal.is_some_and(|ticks| ticks <= 2),
        within_4_ticks: ticks_to_terminal.is_some_and(|ticks| ticks <= 4),
        within_8_ticks: ticks_to_terminal.is_some_and(|ticks| ticks <= 8),
    })
}

fn split_for(entry: &NativeReplayEntry, config: AuxiliarySplitConfig) -> AuxiliarySplit {
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight.native-auxiliary-episode-split/v1\0");
    hasher.update(config.seed.to_le_bytes());
    hasher.update(entry.episode_payload_xxh3_128.as_bytes());
    let digest: [u8; 32] = hasher.finalize().into();
    let bucket = u64::from_le_bytes(digest[..8].try_into().expect("fixed digest width")) % 10_000;
    if bucket < u64::from(config.training_basis_points) {
        AuxiliarySplit::Training
    } else if bucket
        < u64::from(config.training_basis_points) + u64::from(config.validation_basis_points)
    {
        AuxiliarySplit::Validation
    } else {
        AuxiliarySplit::Test
    }
}

fn report(
    examples: &[NativeAuxiliaryExample],
) -> Result<NativeAuxiliaryDatasetReport, NativeAuxiliaryDatasetError> {
    let mut splits = BTreeMap::new();
    let mut roles = BTreeMap::new();
    let mut actor_appearances = 0_u64;
    let mut actor_disappearances = 0_u64;
    for example in examples {
        *splits.entry(example.split).or_default() += 1;
        *roles.entry(example.role).or_default() += 1;
        actor_appearances = actor_appearances
            .checked_add(
                example
                    .targets
                    .actor_lifecycle
                    .as_ref()
                    .map_or(0, |target| target.appeared_runtime_generations.len())
                    as u64,
            )
            .ok_or_else(|| NativeAuxiliaryDatasetError::new("actor appearance count overflowed"))?;
        actor_disappearances = actor_disappearances
            .checked_add(
                example
                    .targets
                    .actor_lifecycle
                    .as_ref()
                    .map_or(0, |target| target.disappeared_runtime_generations.len())
                    as u64,
            )
            .ok_or_else(|| {
                NativeAuxiliaryDatasetError::new("actor disappearance count overflowed")
            })?;
    }
    Ok(NativeAuxiliaryDatasetReport {
        examples: examples.len(),
        episodes: examples
            .iter()
            .map(|example| {
                (
                    example.shard_sha256,
                    example.episode_payload_xxh3_128.as_str(),
                )
            })
            .collect::<BTreeSet<_>>()
            .len(),
        splits,
        roles,
        player_dynamics_targets: examples
            .iter()
            .filter(|example| example.targets.player_dynamics.is_some())
            .count(),
        contact_targets: examples
            .iter()
            .filter(|example| example.targets.contacts.is_some())
            .count(),
        action_phase_targets: examples
            .iter()
            .filter(|example| example.targets.action_phase.is_some())
            .count(),
        actor_appearances,
        actor_disappearances,
        terminal_reachable_examples: examples
            .iter()
            .filter(|example| example.targets.reachability.ticks_to_terminal.is_some())
            .count(),
    })
}

fn targets_are_valid(targets: &NativeAuxiliaryTargets) -> bool {
    let lifecycle_valid = targets.actor_lifecycle.as_ref().is_none_or(|target| {
        is_sorted_unique(&target.appeared_runtime_generations)
            && is_sorted_unique(&target.disappeared_runtime_generations)
            && target.appeared_runtime_generations.iter().all(|id| {
                target
                    .disappeared_runtime_generations
                    .binary_search(id)
                    .is_err()
            })
    });
    (!targets.comparability.same_player || targets.comparability.same_context)
        && (!targets.comparability.link_contacts || targets.comparability.same_player)
        && (targets.player_dynamics.is_some() == targets.comparability.same_player)
        && (targets.action_phase.is_some() == targets.comparability.same_player)
        && (targets.contacts.is_some() == targets.comparability.link_contacts)
        && (targets.actor_lifecycle.is_some() == targets.comparability.actor_lifecycle)
        && (targets.comparability.actor_lifecycle == targets.comparability.same_context)
        && targets.player_dynamics.as_ref().is_none_or(|target| {
            target
                .position_delta
                .iter()
                .chain(&target.velocity_delta)
                .chain(std::iter::once(&target.forward_speed_delta))
                .all(|value| value.is_finite())
        })
        && lifecycle_valid
        && targets.event_loading.goal_reached_after == targets.reachability.within_1_tick
}

fn is_sorted_unique(values: &[u64]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
}

fn subtract3(after: [f32; 3], before: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|index| after[index] - before[index])
}

fn hex_128(value: [u8; 16]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn is_lower_hex(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn canonical_digest<T: Serialize>(
    domain: &[u8],
    value: &T,
) -> Result<Digest, NativeAuxiliaryDatasetError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| NativeAuxiliaryDatasetError::new(error.to_string()))?;
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
    Ok(Digest(hasher.finalize().into()))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeAuxiliaryDatasetError(String);

impl NativeAuxiliaryDatasetError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for NativeAuxiliaryDatasetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for NativeAuxiliaryDatasetError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_replay_corpus::{ReplayEpisodeSource, ReplayExperienceRole};

    fn shard() -> NativeEpisodeShard {
        NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v14.dseps"
        ))
        .unwrap()
    }

    fn corpus(shard: &NativeEpisodeShard) -> NativeReplayCorpus {
        let sources = shard
            .episodes
            .iter()
            .enumerate()
            .map(|(episode_index, episode)| ReplayEpisodeSource {
                shard,
                episode_index,
                role: if episode.success {
                    ReplayExperienceRole::Demonstration
                } else {
                    ReplayExperienceRole::RandomizedCoverage
                },
                policy_lineage_sha256: None,
                parent_entry_sha256: None,
            })
            .collect::<Vec<_>>();
        NativeReplayCorpus::build(None, &sources).unwrap()
    }

    #[test]
    fn phase_correct_rows_keep_complete_observations_by_reference() {
        let shard = shard();
        let corpus = corpus(&shard);
        let dataset = NativeAuxiliaryDataset::build(
            &corpus,
            std::slice::from_ref(&shard),
            AuxiliarySplitConfig::default(),
        )
        .unwrap();
        assert_eq!(
            dataset.report.examples,
            shard
                .episodes
                .iter()
                .map(|episode| episode.steps.len())
                .sum::<usize>()
        );
        assert_eq!(dataset.report.episodes, shard.episodes.len());
        assert_eq!(
            dataset.report.terminal_reachable_examples,
            shard
                .episodes
                .iter()
                .filter(|episode| episode.success)
                .map(|episode| episode.steps.len())
                .sum::<usize>()
        );
        for episode in &shard.episodes {
            let splits = dataset
                .examples
                .iter()
                .filter(|example| example.episode_id == episode.id)
                .map(|example| example.split)
                .collect::<BTreeSet<_>>();
            assert_eq!(splits.len(), 1);
        }
        let diagnostics = dataset.split_diagnostics().unwrap();
        assert_eq!(
            diagnostics
                .values()
                .map(|split| split.examples)
                .sum::<usize>(),
            dataset.examples.len()
        );
        assert_eq!(
            diagnostics
                .values()
                .map(|split| split.episodes)
                .sum::<usize>(),
            shard.episodes.len()
        );
        dataset.validate().unwrap();
    }

    #[test]
    fn dataset_rejects_missing_source_and_cross_episode_split_tampering() {
        let shard = shard();
        let corpus = corpus(&shard);
        assert!(
            NativeAuxiliaryDataset::build(&corpus, &[], AuxiliarySplitConfig::default()).is_err()
        );
        let mut dataset = NativeAuxiliaryDataset::build(
            &corpus,
            std::slice::from_ref(&shard),
            AuxiliarySplitConfig::default(),
        )
        .unwrap();
        let mut conflicting = dataset.examples[0].clone();
        conflicting.step_index += 1;
        conflicting.split = match dataset.examples[0].split {
            AuxiliarySplit::Training => AuxiliarySplit::Validation,
            AuxiliarySplit::Validation | AuxiliarySplit::Test => AuxiliarySplit::Training,
        };
        conflicting.example_sha256 = conflicting.digest().unwrap();
        dataset.examples.push(conflicting);
        dataset
            .examples
            .sort_by_key(|example| example.example_sha256);
        dataset.report = report(&dataset.examples).unwrap();
        dataset.dataset_sha256 = dataset.digest().unwrap();
        assert!(dataset.validate().is_err());
    }

    #[test]
    fn loading_boundaries_mask_incomparable_player_and_actor_targets() {
        let mut shard = shard();
        let episode = &mut shard.episodes[0];
        let mut step = episode.steps[0].clone();
        step.post_simulation.room = step.post_simulation.room.wrapping_add(1);
        let targets = targets(episode, 0, &step).unwrap();
        assert!(!targets.comparability.same_context);
        assert!(!targets.comparability.same_player);
        assert!(!targets.comparability.link_contacts);
        assert!(!targets.comparability.actor_lifecycle);
        assert!(targets.player_dynamics.is_none());
        assert!(targets.contacts.is_none());
        assert!(targets.action_phase.is_none());
        assert!(targets.actor_lifecycle.is_none());
        assert_ne!(
            targets.event_loading.room_before,
            targets.event_loading.room_after
        );
        assert!(targets_are_valid(&targets));
    }

    #[test]
    fn non_link_players_keep_motion_but_not_link_contact_labels() {
        let mut shard = shard();
        let episode = &mut shard.episodes[0];
        let mut step = episode.steps[0].clone();
        step.pre_input.player_is_link = false;
        step.post_simulation.player_is_link = false;
        step.pre_input.player_contacts = 0x1f;
        step.post_simulation.player_contacts = 0;
        let targets = targets(episode, 0, &step).unwrap();
        assert!(targets.comparability.same_context);
        assert!(targets.comparability.same_player);
        assert!(!targets.comparability.link_contacts);
        assert!(targets.comparability.actor_lifecycle);
        assert!(targets.player_dynamics.is_some());
        assert!(targets.action_phase.is_some());
        assert!(targets.actor_lifecycle.is_some());
        assert!(targets.contacts.is_none());
        assert!(targets_are_valid(&targets));
    }
}
