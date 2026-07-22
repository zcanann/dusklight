//! Authenticated goal-conditioned targets over complete native trajectories.
//!
//! Rows retain source identities instead of duplicating the large observation
//! payload. A fitter must resolve each row back to its `.dseps` pre-input
//! observation and use the bound native policy feature schema. Goal structure
//! is carried by `SemanticGoalInput`; objective digests are provenance only.

use crate::artifact::Digest;
use crate::compiled_goal_graph::CompiledGoalGraph;
use crate::native_auxiliary_dataset::{AuxiliaryPadTarget, AuxiliarySplit};
use crate::native_policy_features::{
    NATIVE_POLICY_FEATURE_SCHEMA_SHA256, encode_native_policy_observation,
};
use crate::native_replay_corpus::{NativeReplayCorpus, NativeReplayEntry, ReplayExperienceRole};
use crate::semantic_goal_input::SemanticGoalInput;
use dusklight_evidence::native_episode_shard::{
    NativeEpisode, NativeEpisodeShard, NativeRawPad, authored_milestone_objective_identity,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

pub const NATIVE_GOAL_TRAJECTORY_DATASET_SCHEMA_V1: &str =
    "dusklight-native-goal-trajectory-dataset/v1";
pub const NATIVE_GOAL_TRAJECTORY_ROW_SCHEMA_V1: &str = "dusklight-native-goal-trajectory-row/v1";
pub const RETURN_SCALE: u64 = 1_000_000;
const MAX_ROWS: usize = 16_000_000;
const MAX_N_STEP: u16 = 4_096;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeGoalTrajectoryConfig {
    pub n_step: u16,
    pub discount_millionths: u32,
    pub training_basis_points: u16,
    pub validation_basis_points: u16,
    pub split_seed: u64,
}

impl Default for NativeGoalTrajectoryConfig {
    fn default() -> Self {
        Self {
            n_step: 8,
            discount_millionths: 990_000,
            training_basis_points: 8_000,
            validation_basis_points: 1_000,
            split_seed: 0x474f_414c_5452_4a01,
        }
    }
}

impl NativeGoalTrajectoryConfig {
    /// Validates the bounded, deterministic trajectory-target configuration.
    ///
    /// Orchestrators call this before sealing unattended learning-loop requests,
    /// so an invalid configuration cannot become durable campaign authority.
    pub fn validate(self) -> Result<(), NativeGoalTrajectoryError> {
        if self.n_step == 0
            || self.n_step > MAX_N_STEP
            || self.discount_millionths == 0
            || u64::from(self.discount_millionths) > RETURN_SCALE
            || self.training_basis_points == 0
            || self.validation_basis_points == 0
            || u32::from(self.training_basis_points) + u32::from(self.validation_basis_points)
                >= 10_000
        {
            return Err(NativeGoalTrajectoryError::new(
                "goal trajectory configuration is outside its bounded domain",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeGoalTrajectoryRow {
    pub schema: String,
    pub row_sha256: Digest,
    pub replay_entry_sha256: Digest,
    pub shard_sha256: Digest,
    pub episode_id: String,
    pub episode_payload_xxh3_128: String,
    pub role: ReplayExperienceRole,
    pub step_index: u32,
    pub episode_ticks: u32,
    pub split: AuxiliarySplit,
    pub pre_input_state_xxh3_128: String,
    pub consumed_pad: AuxiliaryPadTarget,
    pub success: bool,
    /// One-based number of decisions from this pre-input state to the hit.
    pub ticks_to_goal: Option<u32>,
    /// Number of transitions consumed by this bounded return target.
    pub n_step_ticks: u16,
    pub terminal_within_n_steps: bool,
    /// Exact row at `step_index + n_step`; absent at either terminal boundary.
    pub bootstrap_row_sha256: Option<Digest>,
    /// Discounted unit success reward observed inside this n-step window.
    pub terminal_reward_millionths: u64,
    /// Discount multiplying the linked bootstrap value.
    pub bootstrap_discount_millionths: u64,
    /// Full Monte Carlo success return, retained as a supervised target.
    pub realized_return_millionths: u64,
    /// Discounted count of transitions in the bounded target window.
    pub discounted_tick_cost_millionths: u64,
}

impl NativeGoalTrajectoryRow {
    fn build(
        entry: &NativeReplayEntry,
        episode: &NativeEpisode,
        step_index: usize,
        split: AuxiliarySplit,
        config: NativeGoalTrajectoryConfig,
        future_rows: &BTreeMap<u32, Digest>,
    ) -> Result<Self, NativeGoalTrajectoryError> {
        let step = episode
            .steps
            .get(step_index)
            .ok_or_else(|| NativeGoalTrajectoryError::new("trajectory step is absent"))?;
        // This fail-closes before a dataset can advertise a feature schema that
        // cannot actually encode one of its referenced policy observations.
        encode_native_policy_observation(&step.pre_input)
            .map_err(|error| NativeGoalTrajectoryError::new(error.to_string()))?;
        let step_index = u32::try_from(step_index)
            .map_err(|_| NativeGoalTrajectoryError::new("trajectory step index overflowed"))?;
        let remaining = entry
            .ticks_executed
            .checked_sub(step_index)
            .ok_or_else(|| NativeGoalTrajectoryError::new("trajectory step exceeds episode"))?;
        let ticks_to_goal = entry
            .first_hit_tick
            .map(|hit| {
                hit.checked_sub(step_index)
                    .and_then(|distance| distance.checked_add(1))
                    .ok_or_else(|| {
                        NativeGoalTrajectoryError::new("goal hit precedes trajectory decision")
                    })
            })
            .transpose()?;
        let n_step_ticks_u32 = remaining.min(u32::from(config.n_step));
        let n_step_ticks = u16::try_from(n_step_ticks_u32)
            .map_err(|_| NativeGoalTrajectoryError::new("n-step width overflowed"))?;
        let terminal_within_n_steps = ticks_to_goal.is_some_and(|ticks| ticks <= n_step_ticks_u32);
        let bootstrap_index = step_index.checked_add(u32::from(config.n_step));
        let bootstrap_row_sha256 =
            if !terminal_within_n_steps && remaining > u32::from(config.n_step) {
                Some(
                    *future_rows
                        .get(&bootstrap_index.expect("bounded addition"))
                        .ok_or_else(|| {
                            NativeGoalTrajectoryError::new("trajectory bootstrap row is absent")
                        })?,
                )
            } else {
                None
            };
        let terminal_reward_millionths = if terminal_within_n_steps {
            fixed_power(
                config.discount_millionths,
                ticks_to_goal.expect("terminal distance") - 1,
            )?
        } else {
            0
        };
        let bootstrap_discount_millionths = if bootstrap_row_sha256.is_some() {
            fixed_power(config.discount_millionths, u32::from(config.n_step))?
        } else {
            0
        };
        let realized_return_millionths = ticks_to_goal
            .map(|ticks| fixed_power(config.discount_millionths, ticks - 1))
            .transpose()?
            .unwrap_or(0);
        let discounted_tick_cost_millionths =
            fixed_geometric_sum(config.discount_millionths, u32::from(n_step_ticks))?;
        let mut row = Self {
            schema: NATIVE_GOAL_TRAJECTORY_ROW_SCHEMA_V1.into(),
            row_sha256: Digest::ZERO,
            replay_entry_sha256: entry.entry_sha256,
            shard_sha256: entry.shard_sha256,
            episode_id: entry.episode_id.clone(),
            episode_payload_xxh3_128: entry.episode_payload_xxh3_128.clone(),
            role: entry.role,
            step_index,
            episode_ticks: entry.ticks_executed,
            split,
            pre_input_state_xxh3_128: hex_128(step.pre_input.state_identity),
            consumed_pad: pad_target(step.consumed_pad)?,
            success: entry.success,
            ticks_to_goal,
            n_step_ticks,
            terminal_within_n_steps,
            bootstrap_row_sha256,
            terminal_reward_millionths,
            bootstrap_discount_millionths,
            realized_return_millionths,
            discounted_tick_cost_millionths,
        };
        row.row_sha256 = row.digest()?;
        row.validate(config)?;
        Ok(row)
    }

    fn validate(
        &self,
        config: NativeGoalTrajectoryConfig,
    ) -> Result<(), NativeGoalTrajectoryError> {
        let remaining = self.episode_ticks.checked_sub(self.step_index);
        let expected_ticks = remaining.map(|value| value.min(u32::from(config.n_step)));
        let expected_terminal = self
            .ticks_to_goal
            .is_some_and(|ticks| ticks <= u32::from(self.n_step_ticks));
        let expected_reward = if expected_terminal {
            fixed_power(config.discount_millionths, self.ticks_to_goal.unwrap() - 1)?
        } else {
            0
        };
        let expected_realized = self
            .ticks_to_goal
            .map(|ticks| fixed_power(config.discount_millionths, ticks - 1))
            .transpose()?
            .unwrap_or(0);
        let expects_bootstrap =
            !expected_terminal && remaining.is_some_and(|value| value > u32::from(config.n_step));
        if self.schema != NATIVE_GOAL_TRAJECTORY_ROW_SCHEMA_V1
            || self.row_sha256 == Digest::ZERO
            || self.replay_entry_sha256 == Digest::ZERO
            || self.shard_sha256 == Digest::ZERO
            || self.episode_id.is_empty()
            || !valid_hex_128(&self.episode_payload_xxh3_128)
            || !valid_hex_128(&self.pre_input_state_xxh3_128)
            || self.episode_ticks == 0
            || expected_ticks != Some(u32::from(self.n_step_ticks))
            || self.n_step_ticks == 0
            || self.success != self.ticks_to_goal.is_some()
            || (self.success && self.ticks_to_goal != remaining)
            || self
                .ticks_to_goal
                .is_some_and(|ticks| ticks == 0 || ticks > remaining.unwrap_or(0))
            || self.terminal_within_n_steps != expected_terminal
            || self.bootstrap_row_sha256.is_some() != expects_bootstrap
            || self.terminal_reward_millionths != expected_reward
            || self.bootstrap_discount_millionths
                != if expects_bootstrap {
                    fixed_power(config.discount_millionths, u32::from(config.n_step))?
                } else {
                    0
                }
            || self.realized_return_millionths != expected_realized
            || self.discounted_tick_cost_millionths
                != fixed_geometric_sum(config.discount_millionths, u32::from(self.n_step_ticks))?
            || self.row_sha256 != self.digest()?
        {
            return Err(NativeGoalTrajectoryError::new(
                "native goal trajectory row is invalid or detached",
            ));
        }
        Ok(())
    }

    fn digest(&self) -> Result<Digest, NativeGoalTrajectoryError> {
        canonical_digest(
            b"dusklight.native-goal-trajectory-row/v1\0",
            &(
                (
                    &self.schema,
                    self.replay_entry_sha256,
                    self.shard_sha256,
                    &self.episode_id,
                    &self.episode_payload_xxh3_128,
                    self.role,
                    self.step_index,
                    self.episode_ticks,
                    self.split,
                    &self.pre_input_state_xxh3_128,
                ),
                (
                    self.consumed_pad,
                    self.success,
                    self.ticks_to_goal,
                    self.n_step_ticks,
                    self.terminal_within_n_steps,
                    self.bootstrap_row_sha256,
                    self.terminal_reward_millionths,
                    self.bootstrap_discount_millionths,
                    self.realized_return_millionths,
                    self.discounted_tick_cost_millionths,
                ),
            ),
        )
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeGoalTrajectoryReport {
    pub rows: usize,
    pub episodes: usize,
    pub successful_episodes: usize,
    pub failed_episodes: usize,
    pub goal_reachable_rows: usize,
    pub bootstrap_rows: usize,
    pub splits: BTreeMap<AuxiliarySplit, usize>,
    pub roles: BTreeMap<ReplayExperienceRole, usize>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeGoalTrajectoryDataset {
    pub schema: String,
    pub replay_corpus_sha256: Digest,
    pub observation_schema: String,
    pub action_schema: String,
    pub native_feature_schema_sha256: Digest,
    pub goal_program_sha256: Digest,
    pub goal_definition_name: String,
    pub goal_objective_identity: String,
    pub goal: SemanticGoalInput,
    pub config: NativeGoalTrajectoryConfig,
    pub rows: Vec<NativeGoalTrajectoryRow>,
    pub report: NativeGoalTrajectoryReport,
    pub dataset_sha256: Digest,
}

impl NativeGoalTrajectoryDataset {
    pub fn build(
        corpus: &NativeReplayCorpus,
        shards: &[NativeEpisodeShard],
        graph: &CompiledGoalGraph,
        config: NativeGoalTrajectoryConfig,
    ) -> Result<Self, NativeGoalTrajectoryError> {
        corpus
            .validate()
            .map_err(|error| NativeGoalTrajectoryError::new(error.to_string()))?;
        graph
            .validate()
            .map_err(|error| NativeGoalTrajectoryError::new(error.to_string()))?;
        config.validate()?;
        let objective_identity = authored_milestone_objective_identity(
            &graph.program_sha256.to_string(),
            &graph.definition_sha256.to_string(),
        )
        .map_err(|error| NativeGoalTrajectoryError::new(error.to_string()))?;
        let goal = SemanticGoalInput::from_graph(graph)
            .map_err(|error| NativeGoalTrajectoryError::new(error.to_string()))?;
        let mut shard_by_digest = BTreeMap::new();
        for shard in shards {
            if shard_by_digest
                .insert(shard.content_sha256, shard)
                .is_some()
            {
                return Err(NativeGoalTrajectoryError::new(
                    "goal trajectory dataset received a duplicate shard",
                ));
            }
        }
        let mut rows = Vec::new();
        for entry in &corpus.entries {
            if entry.objective != graph.definition_name
                || entry.objective_identity != objective_identity
            {
                return Err(NativeGoalTrajectoryError::new(
                    "replay objective does not match the compiled goal",
                ));
            }
            let shard = shard_by_digest.get(&entry.shard_sha256).ok_or_else(|| {
                NativeGoalTrajectoryError::new(format!(
                    "goal trajectory dataset is missing shard {}",
                    entry.shard_sha256
                ))
            })?;
            validate_entry_source(corpus, entry, shard)?;
            let episode = shard
                .episodes
                .iter()
                .find(|episode| episode.id == entry.episode_id)
                .ok_or_else(|| NativeGoalTrajectoryError::new("replay episode is absent"))?;
            validate_episode(entry, episode)?;
            let split = split_for(entry, config);
            let mut future_rows = BTreeMap::new();
            let mut episode_rows = Vec::with_capacity(episode.steps.len());
            for step_index in (0..episode.steps.len()).rev() {
                let row = NativeGoalTrajectoryRow::build(
                    entry,
                    episode,
                    step_index,
                    split,
                    config,
                    &future_rows,
                )?;
                future_rows.insert(row.step_index, row.row_sha256);
                episode_rows.push(row);
            }
            rows.extend(episode_rows);
            if rows.len() > MAX_ROWS {
                return Err(NativeGoalTrajectoryError::new(
                    "goal trajectory row limit exceeded",
                ));
            }
        }
        rows.sort_by_key(|row| row.row_sha256);
        let report = report(&rows)?;
        let mut dataset = Self {
            schema: NATIVE_GOAL_TRAJECTORY_DATASET_SCHEMA_V1.into(),
            replay_corpus_sha256: corpus.corpus_sha256,
            observation_schema: corpus.observation_schema.clone(),
            action_schema: corpus.action_schema.clone(),
            native_feature_schema_sha256: Digest(NATIVE_POLICY_FEATURE_SCHEMA_SHA256),
            goal_program_sha256: graph.program_sha256,
            goal_definition_name: graph.definition_name.clone(),
            goal_objective_identity: objective_identity,
            goal,
            config,
            rows,
            report,
            dataset_sha256: Digest::ZERO,
        };
        dataset.dataset_sha256 = dataset.digest()?;
        dataset.validate()?;
        Ok(dataset)
    }

    pub fn validate(&self) -> Result<(), NativeGoalTrajectoryError> {
        self.config.validate()?;
        self.goal
            .validate()
            .map_err(|error| NativeGoalTrajectoryError::new(error.to_string()))?;
        let expected_objective_identity = authored_milestone_objective_identity(
            &self.goal_program_sha256.to_string(),
            &self.goal.definition_sha256.to_string(),
        )
        .map_err(|error| NativeGoalTrajectoryError::new(error.to_string()))?;
        if self.schema != NATIVE_GOAL_TRAJECTORY_DATASET_SCHEMA_V1
            || self.replay_corpus_sha256 == Digest::ZERO
            || self.observation_schema.is_empty()
            || self.action_schema.is_empty()
            || self.native_feature_schema_sha256 != Digest(NATIVE_POLICY_FEATURE_SCHEMA_SHA256)
            || self.goal_program_sha256 == Digest::ZERO
            || self.goal_definition_name.is_empty()
            || self.goal_objective_identity != expected_objective_identity
            || self.rows.is_empty()
            || self.rows.len() > MAX_ROWS
            || !self
                .rows
                .windows(2)
                .all(|pair| pair[0].row_sha256 < pair[1].row_sha256)
            || self
                .rows
                .iter()
                .any(|row| row.validate(self.config).is_err())
            || self.report != report(&self.rows)?
            || self.dataset_sha256 != self.digest()?
        {
            return Err(NativeGoalTrajectoryError::new(
                "native goal trajectory dataset is invalid or detached",
            ));
        }
        let by_identity = self
            .rows
            .iter()
            .map(|row| (row.row_sha256, row))
            .collect::<BTreeMap<_, _>>();
        let mut episodes = BTreeMap::<Digest, (&NativeGoalTrajectoryRow, BTreeSet<u32>)>::new();
        for row in &self.rows {
            let (first, steps) = episodes
                .entry(row.replay_entry_sha256)
                .or_insert_with(|| (row, BTreeSet::new()));
            if first.shard_sha256 != row.shard_sha256
                || first.episode_id != row.episode_id
                || first.episode_payload_xxh3_128 != row.episode_payload_xxh3_128
                || first.role != row.role
                || first.episode_ticks != row.episode_ticks
                || first.split != row.split
                || first.success != row.success
                || !steps.insert(row.step_index)
            {
                return Err(NativeGoalTrajectoryError::new(
                    "trajectory episode metadata, split, or step identity is inconsistent",
                ));
            }
            if let Some(bootstrap) = row.bootstrap_row_sha256 {
                let target = by_identity.get(&bootstrap).ok_or_else(|| {
                    NativeGoalTrajectoryError::new("trajectory bootstrap target is absent")
                })?;
                if target.replay_entry_sha256 != row.replay_entry_sha256
                    || target.split != row.split
                    || target.step_index
                        != row
                            .step_index
                            .checked_add(u32::from(self.config.n_step))
                            .ok_or_else(|| {
                                NativeGoalTrajectoryError::new("bootstrap index overflowed")
                            })?
                {
                    return Err(NativeGoalTrajectoryError::new(
                        "trajectory bootstrap crosses an episode or split",
                    ));
                }
            }
        }
        if episodes.values().any(|(row, steps)| {
            steps.len() != row.episode_ticks as usize
                || steps.iter().copied().ne(0..row.episode_ticks)
        }) {
            return Err(NativeGoalTrajectoryError::new(
                "trajectory dataset does not retain every episode decision exactly once",
            ));
        }
        Ok(())
    }

    pub fn validate_sources(
        &self,
        corpus: &NativeReplayCorpus,
        shards: &[NativeEpisodeShard],
        graph: &CompiledGoalGraph,
    ) -> Result<(), NativeGoalTrajectoryError> {
        let rebuilt = Self::build(corpus, shards, graph, self.config)?;
        if &rebuilt != self {
            return Err(NativeGoalTrajectoryError::new(
                "goal trajectory dataset is detached from its source artifacts",
            ));
        }
        Ok(())
    }

    fn digest(&self) -> Result<Digest, NativeGoalTrajectoryError> {
        canonical_digest(
            b"dusklight.native-goal-trajectory-dataset/v1\0",
            &(
                &self.schema,
                self.replay_corpus_sha256,
                &self.observation_schema,
                &self.action_schema,
                self.native_feature_schema_sha256,
                self.goal_program_sha256,
                &self.goal_definition_name,
                &self.goal_objective_identity,
                &self.goal,
                self.config,
                &self.rows,
                &self.report,
            ),
        )
    }
}

fn validate_entry_source(
    corpus: &NativeReplayCorpus,
    entry: &NativeReplayEntry,
    shard: &NativeEpisodeShard,
) -> Result<(), NativeGoalTrajectoryError> {
    if shard.metadata.observation_schema != corpus.observation_schema
        || shard.metadata.action_schema != corpus.action_schema
        || shard.source_frame != entry.source_frame
        || shard.metadata.source_boundary_fingerprint != entry.source_boundary_fingerprint
        || shard.metadata.checkpoint_identity != entry.checkpoint_identity
        || shard.metadata.objective != entry.objective
        || shard.metadata.objective_identity != entry.objective_identity
    {
        return Err(NativeGoalTrajectoryError::new(
            "replay entry is detached from its native shard metadata",
        ));
    }
    Ok(())
}

fn validate_episode(
    entry: &NativeReplayEntry,
    episode: &NativeEpisode,
) -> Result<(), NativeGoalTrajectoryError> {
    if hex_128(episode.payload_xxh3_128) != entry.episode_payload_xxh3_128
        || episode.steps.len() != entry.ticks_executed as usize
        || episode.ticks_executed != entry.ticks_executed
        || episode.success != entry.success
        || episode.first_hit_tick != entry.first_hit_tick
    {
        return Err(NativeGoalTrajectoryError::new(
            "replay episode identity, length, or outcome is detached",
        ));
    }
    Ok(())
}

fn split_for(entry: &NativeReplayEntry, config: NativeGoalTrajectoryConfig) -> AuxiliarySplit {
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight.native-goal-trajectory-split/v1\0");
    hasher.update(config.split_seed.to_le_bytes());
    hasher.update(entry.entry_sha256.0);
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
    rows: &[NativeGoalTrajectoryRow],
) -> Result<NativeGoalTrajectoryReport, NativeGoalTrajectoryError> {
    let mut splits = BTreeMap::new();
    let mut roles = BTreeMap::new();
    for row in rows {
        *splits.entry(row.split).or_default() += 1;
        *roles.entry(row.role).or_default() += 1;
    }
    let episodes = rows
        .iter()
        .map(|row| (row.replay_entry_sha256, row.success))
        .collect::<BTreeSet<_>>();
    Ok(NativeGoalTrajectoryReport {
        rows: rows.len(),
        episodes: episodes.len(),
        successful_episodes: episodes.iter().filter(|(_, success)| *success).count(),
        failed_episodes: episodes.iter().filter(|(_, success)| !*success).count(),
        goal_reachable_rows: rows
            .iter()
            .filter(|row| row.ticks_to_goal.is_some())
            .count(),
        bootstrap_rows: rows
            .iter()
            .filter(|row| row.bootstrap_row_sha256.is_some())
            .count(),
        splits,
        roles,
    })
}

fn pad_target(pad: NativeRawPad) -> Result<AuxiliaryPadTarget, NativeGoalTrajectoryError> {
    if !pad.connected || pad.error != 0 {
        return Err(NativeGoalTrajectoryError::new(
            "goal trajectory action requires a connected error-free PAD",
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

fn fixed_power(discount_millionths: u32, exponent: u32) -> Result<u64, NativeGoalTrajectoryError> {
    Ok(fixed_power_and_sum(discount_millionths, exponent)?.0)
}

fn fixed_geometric_sum(
    discount_millionths: u32,
    terms: u32,
) -> Result<u64, NativeGoalTrajectoryError> {
    Ok(fixed_power_and_sum(discount_millionths, terms)?.1)
}

/// Returns `(discount^terms, sum(discount^0..discount^(terms-1)))` in
/// millionths using a deterministic logarithmic-time segment composition.
fn fixed_power_and_sum(
    discount_millionths: u32,
    mut terms: u32,
) -> Result<(u64, u64), NativeGoalTrajectoryError> {
    let mut result = (RETURN_SCALE, 0_u64);
    let mut segment = (u64::from(discount_millionths), RETURN_SCALE);
    while terms != 0 {
        if terms & 1 != 0 {
            result = compose_discount_segments(result, segment)?;
        }
        terms >>= 1;
        if terms != 0 {
            segment = compose_discount_segments(segment, segment)?;
        }
    }
    Ok(result)
}

fn compose_discount_segments(
    left: (u64, u64),
    right: (u64, u64),
) -> Result<(u64, u64), NativeGoalTrajectoryError> {
    let power = rounded_fixed_product(left.0, right.0)?;
    let discounted_right_sum = rounded_fixed_product(left.0, right.1)?;
    let sum = left
        .1
        .checked_add(discounted_right_sum)
        .ok_or_else(|| NativeGoalTrajectoryError::new("discounted tick cost overflowed"))?;
    Ok((power, sum))
}

fn rounded_fixed_product(left: u64, right: u64) -> Result<u64, NativeGoalTrajectoryError> {
    let product = u128::from(left)
        .checked_mul(u128::from(right))
        .and_then(|value| value.checked_add(u128::from(RETURN_SCALE / 2)))
        .ok_or_else(|| NativeGoalTrajectoryError::new("fixed-point return overflowed"))?;
    u64::try_from(product / u128::from(RETURN_SCALE))
        .map_err(|_| NativeGoalTrajectoryError::new("fixed-point return exceeds u64"))
}

fn hex_128(value: [u8; 16]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn valid_hex_128(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn canonical_digest<T: Serialize>(
    domain: &[u8],
    value: &T,
) -> Result<Digest, NativeGoalTrajectoryError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| NativeGoalTrajectoryError::new(error.to_string()))?;
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
    Ok(Digest(hasher.finalize().into()))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeGoalTrajectoryError(String);

impl NativeGoalTrajectoryError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for NativeGoalTrajectoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for NativeGoalTrajectoryError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::milestone_dsl::compile_source;
    use crate::native_replay_corpus::{ReplayEpisodeSource, ReplayExperienceRole};

    const GOAL_SOURCE: &str = r#"milestones 1.8
milestone test_goal {
  phase post_sim
  when stage.room == 1
}
"#;

    fn sources() -> (NativeEpisodeShard, CompiledGoalGraph, NativeReplayCorpus) {
        let mut shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v14.dseps"
        ))
        .unwrap();
        let compiled = compile_source(GOAL_SOURCE).unwrap();
        let graph = CompiledGoalGraph::from_compiled(&compiled, 0).unwrap();
        shard.metadata.objective = graph.definition_name.clone();
        shard.metadata.objective_identity = authored_milestone_objective_identity(
            &graph.program_sha256.to_string(),
            &graph.definition_sha256.to_string(),
        )
        .unwrap();
        let replay_sources = shard
            .episodes
            .iter()
            .enumerate()
            .map(|(episode_index, episode)| ReplayEpisodeSource {
                shard: &shard,
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
        let corpus = NativeReplayCorpus::build(None, &replay_sources).unwrap();
        (shard, graph, corpus)
    }

    #[test]
    fn complete_trajectories_bind_goal_features_actions_and_outcomes() {
        let (shard, graph, corpus) = sources();
        let dataset = NativeGoalTrajectoryDataset::build(
            &corpus,
            std::slice::from_ref(&shard),
            &graph,
            NativeGoalTrajectoryConfig::default(),
        )
        .unwrap();
        dataset
            .validate_sources(&corpus, std::slice::from_ref(&shard), &graph)
            .unwrap();
        assert_eq!(dataset.report.episodes, 2);
        assert_eq!(dataset.report.successful_episodes, 1);
        assert_eq!(dataset.report.failed_episodes, 1);
        assert_eq!(
            dataset.native_feature_schema_sha256,
            Digest(NATIVE_POLICY_FEATURE_SCHEMA_SHA256)
        );
        let successful = dataset.rows.iter().find(|row| row.success).unwrap();
        assert_eq!(successful.ticks_to_goal, Some(1));
        assert_eq!(successful.terminal_reward_millionths, RETURN_SCALE);
        assert_eq!(successful.realized_return_millionths, RETURN_SCALE);
        let failed = dataset.rows.iter().find(|row| !row.success).unwrap();
        assert_eq!(failed.ticks_to_goal, None);
        assert_eq!(failed.realized_return_millionths, 0);
    }

    #[test]
    fn n_step_links_and_episode_splits_are_exact() {
        let (mut shard, graph, _) = sources();
        for episode in &mut shard.episodes {
            let original = episode.steps[0].clone();
            episode.steps = vec![original; 5];
            episode.ticks_executed = 5;
            if episode.success {
                episode.first_hit_tick = Some(4);
            }
        }
        let replay_sources = shard
            .episodes
            .iter()
            .enumerate()
            .map(|(episode_index, episode)| ReplayEpisodeSource {
                shard: &shard,
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
        let corpus = NativeReplayCorpus::build(None, &replay_sources).unwrap();
        let config = NativeGoalTrajectoryConfig {
            n_step: 2,
            discount_millionths: 500_000,
            ..NativeGoalTrajectoryConfig::default()
        };
        let dataset = NativeGoalTrajectoryDataset::build(
            &corpus,
            std::slice::from_ref(&shard),
            &graph,
            config,
        )
        .unwrap();
        let success_entry = corpus.entries.iter().find(|entry| entry.success).unwrap();
        let mut success_rows = dataset
            .rows
            .iter()
            .filter(|row| row.replay_entry_sha256 == success_entry.entry_sha256)
            .collect::<Vec<_>>();
        success_rows.sort_by_key(|row| row.step_index);
        assert!(
            success_rows
                .iter()
                .all(|row| row.split == success_rows[0].split)
        );
        assert_eq!(success_rows[0].ticks_to_goal, Some(5));
        assert_eq!(success_rows[0].realized_return_millionths, 62_500);
        assert_eq!(success_rows[0].bootstrap_discount_millionths, 250_000);
        assert_eq!(
            success_rows[0].bootstrap_row_sha256,
            Some(success_rows[2].row_sha256)
        );
        assert_eq!(success_rows[3].terminal_reward_millionths, 500_000);
        assert_eq!(success_rows[4].terminal_reward_millionths, RETURN_SCALE);
        assert!(success_rows[4].bootstrap_row_sha256.is_none());

        // Re-sealing cannot disguise a missing decision in a complete episode.
        let mut missing = dataset.clone();
        missing.rows.retain(|row| {
            row.replay_entry_sha256 != success_entry.entry_sha256 || row.step_index != 1
        });
        missing.report = report(&missing.rows).unwrap();
        missing.dataset_sha256 = missing.digest().unwrap();
        assert!(missing.validate().is_err());

        // Re-sealing cannot move an n-step bootstrap across episode boundaries.
        let mut crossed = dataset;
        let failure_row = crossed
            .rows
            .iter()
            .find(|row| !row.success && row.step_index == 2)
            .unwrap()
            .row_sha256;
        let source = crossed
            .rows
            .iter_mut()
            .find(|row| {
                row.replay_entry_sha256 == success_entry.entry_sha256 && row.step_index == 0
            })
            .unwrap();
        source.bootstrap_row_sha256 = Some(failure_row);
        source.row_sha256 = source.digest().unwrap();
        crossed.rows.sort_by_key(|row| row.row_sha256);
        crossed.report = report(&crossed.rows).unwrap();
        crossed.dataset_sha256 = crossed.digest().unwrap();
        assert!(crossed.validate().is_err());
    }

    #[test]
    fn source_goal_and_bootstrap_tampering_fail_closed() {
        let (shard, graph, corpus) = sources();
        let config = NativeGoalTrajectoryConfig::default();
        let dataset = NativeGoalTrajectoryDataset::build(
            &corpus,
            std::slice::from_ref(&shard),
            &graph,
            config,
        )
        .unwrap();
        let other = CompiledGoalGraph::from_compiled(
            &compile_source(
                r#"milestones 1.8
milestone other_goal {
  phase post_sim
  when stage.room == 2
}
"#,
            )
            .unwrap(),
            0,
        )
        .unwrap();
        assert!(
            dataset
                .validate_sources(&corpus, std::slice::from_ref(&shard), &other)
                .is_err()
        );

        let mut tampered = dataset.clone();
        tampered.rows[0].terminal_reward_millionths ^= 1;
        assert!(tampered.validate().is_err());
        let mut tampered = dataset;
        tampered.rows[0]
            .pre_input_state_xxh3_128
            .replace_range(0..1, "g");
        assert!(tampered.validate().is_err());
    }
}
