//! Deterministic goal-conditioned reachability ensembles over native trajectories.
//!
//! Training consumes only authenticated pre-input observations referenced by
//! `NativeGoalTrajectoryDataset`. Whole episodes remain isolated by split.
//! Real episode outcomes supervise reachability and time-to-go directly, while
//! frozen epoch snapshots provide n-step targets for discounted terminal return
//! and tick cost. Episode bootstrap members expose epistemic disagreement.

use crate::artifact::Digest;
use crate::native_auxiliary_dataset::{AuxiliaryPadTarget, AuxiliarySplit};
use crate::native_goal_trajectory::{
    NativeGoalTrajectoryDataset, NativeGoalTrajectoryRow, RETURN_SCALE,
};
use crate::native_policy_features::{
    NATIVE_POLICY_FEATURE_SCHEMA_SHA256, NATIVE_POLICY_FEATURE_WIDTH,
    encode_native_policy_observation,
};
use crate::native_replay_corpus::DemonstrationMode;
use crate::semantic_goal_input::{
    GOAL_METADATA_WIDTH, GOAL_NODE_FEATURE_WIDTH, GOAL_PROJECTION_FEATURE_WIDTH, GoalEdgeRole,
    SemanticGoalInput,
};
use dusklight_evidence::native_episode_shard::{
    NativeEpisodeShard, NativeLearningObservation, NativeRawPad,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

pub const NATIVE_GOAL_REACHABILITY_MODEL_SCHEMA_V1: &str =
    "dusklight-native-goal-reachability-model/v1";
pub const NATIVE_GOAL_EMBEDDING_SCHEMA_V1: &str = "dusklight-semantic-goal-embedding/v1";
pub const NATIVE_GOAL_REACHABILITY_HEADS: usize = 4;
pub const MAX_GOAL_REACHABILITY_TICKS: u32 = 4_096;
const MAX_DATASETS: usize = 256;
const MAX_ROWS: usize = 1_000_000;
const MAX_INPUT_CELLS: usize = 256_000_000;
const MAX_MEMBERS: usize = 31;
const MAX_EPOCHS: usize = 2_048;
const MAX_HIDDEN_WIDTH: usize = 256;
const MAX_GRADIENT_UPDATES: usize = 100_000_000;
const HEAD_SUCCESS: usize = 0;
const HEAD_TIME: usize = 1;
const HEAD_RETURN: usize = 2;
const HEAD_COST: usize = 3;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeGoalReachabilityConfig {
    pub members: u8,
    pub epochs: u16,
    pub hidden_width: u16,
    pub learning_rate: f64,
    pub l2_penalty: f64,
    pub gradient_clip: f64,
    pub minimum_validation_improvement: f64,
    pub maximum_validation_reachability_stddev: f64,
    pub seed: u64,
}

impl Default for NativeGoalReachabilityConfig {
    fn default() -> Self {
        Self {
            members: 7,
            epochs: 64,
            hidden_width: 64,
            learning_rate: 0.002,
            l2_penalty: 1.0e-5,
            gradient_clip: 5.0,
            minimum_validation_improvement: 0.02,
            maximum_validation_reachability_stddev: 0.25,
            seed: 0x474f_414c_5245_4143,
        }
    }
}

impl NativeGoalReachabilityConfig {
    /// Validates the bounded ensemble configuration before training begins.
    pub fn validate(self) -> Result<(), NativeGoalReachabilityError> {
        if usize::from(self.members) < 2
            || usize::from(self.members) > MAX_MEMBERS
            || self.epochs == 0
            || usize::from(self.epochs) > MAX_EPOCHS
            || self.hidden_width == 0
            || usize::from(self.hidden_width) > MAX_HIDDEN_WIDTH
            || !self.learning_rate.is_finite()
            || self.learning_rate <= 0.0
            || !self.l2_penalty.is_finite()
            || self.l2_penalty < 0.0
            || !self.gradient_clip.is_finite()
            || self.gradient_clip <= 0.0
            || !self.minimum_validation_improvement.is_finite()
            || !(0.0..=1.0).contains(&self.minimum_validation_improvement)
            || !self.maximum_validation_reachability_stddev.is_finite()
            || self.maximum_validation_reachability_stddev < 0.0
            || self.maximum_validation_reachability_stddev > 0.5
        {
            return Err(NativeGoalReachabilityError::new(
                "goal reachability configuration is outside its bounded domain",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeGoalReachabilityAdmission {
    RetainTrainingMeanBaseline,
    GoalConditionedCandidate,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeGoalReachabilityMetrics {
    pub rows: usize,
    pub episodes: usize,
    pub successful_rows: usize,
    pub failed_rows: usize,
    pub reachability_brier: f64,
    pub baseline_reachability_brier: f64,
    pub reachability_relative_improvement: f64,
    pub successful_time_mae_ticks: f64,
    pub baseline_successful_time_mae_ticks: f64,
    pub successful_time_relative_improvement: f64,
    pub discounted_return_rmse: f64,
    pub baseline_discounted_return_rmse: f64,
    pub return_relative_improvement: f64,
    pub discounted_tick_cost_mae: f64,
    pub baseline_discounted_tick_cost_mae: f64,
    pub tick_cost_relative_improvement: f64,
    pub mean_reachability_stddev: f64,
    pub mean_return_stddev: f64,
}

impl NativeGoalReachabilityMetrics {
    fn validate(&self) -> bool {
        self.rows > 0
            && self.episodes > 0
            && self.successful_rows > 0
            && self.failed_rows > 0
            && self.successful_rows + self.failed_rows == self.rows
            && [
                self.reachability_brier,
                self.baseline_reachability_brier,
                self.reachability_relative_improvement,
                self.successful_time_mae_ticks,
                self.baseline_successful_time_mae_ticks,
                self.successful_time_relative_improvement,
                self.discounted_return_rmse,
                self.baseline_discounted_return_rmse,
                self.return_relative_improvement,
                self.discounted_tick_cost_mae,
                self.baseline_discounted_tick_cost_mae,
                self.tick_cost_relative_improvement,
                self.mean_reachability_stddev,
                self.mean_return_stddev,
            ]
            .iter()
            .all(|value| value.is_finite() && *value >= 0.0)
            && self.reachability_brier <= 1.0
            && self.baseline_reachability_brier <= 1.0
            && self.reachability_relative_improvement <= 1.0
            && self.return_relative_improvement <= 1.0
            && self.successful_time_relative_improvement <= 1.0
            && self.tick_cost_relative_improvement <= 1.0
            && self.mean_reachability_stddev <= 0.5
            && self.mean_return_stddev <= 0.5
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NativeGoalReachabilityEstimate {
    pub reachability_probability: f64,
    pub reachability_stddev: f64,
    pub expected_ticks_to_goal: f64,
    pub discounted_terminal_return: f64,
    pub return_stddev: f64,
    pub discounted_tick_cost: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ReachabilityMember {
    bootstrap_episode_sha256: Vec<Digest>,
    hidden_weights: Vec<f64>,
    hidden_bias: Vec<f64>,
    output_weights: Vec<f64>,
    output_bias: Vec<f64>,
    gradient_updates: u64,
    target_synchronizations: u32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeGoalReachabilityModel {
    pub schema: String,
    pub input_schema_sha256: Digest,
    pub native_feature_schema_sha256: Digest,
    pub goal_embedding_schema_sha256: Digest,
    pub observation_schema: String,
    pub action_schema: String,
    pub action_schema_sha256: Digest,
    pub input_width: u32,
    pub goal_embedding_width: u32,
    pub source_dataset_sha256: Vec<Digest>,
    pub source_replay_corpus_sha256: Vec<Digest>,
    pub source_goal_input_sha256: Vec<Digest>,
    pub source_goal_objective_identity: Vec<String>,
    pub demonstration_mode: DemonstrationMode,
    pub config: NativeGoalReachabilityConfig,
    pub feature_mean: Vec<f64>,
    pub feature_inverse_stddev: Vec<f64>,
    pub training_n_step_bootstrap_rows: usize,
    members: Vec<ReachabilityMember>,
    pub training: NativeGoalReachabilityMetrics,
    pub validation: NativeGoalReachabilityMetrics,
    pub test: NativeGoalReachabilityMetrics,
    pub admission: NativeGoalReachabilityAdmission,
    pub promotion_authority: bool,
    pub model_sha256: Digest,
}

#[derive(Clone)]
struct MaterializedRow {
    row_sha256: Digest,
    episode_sha256: Digest,
    split: AuxiliarySplit,
    features: Vec<f64>,
    success: bool,
    ticks_to_goal: Option<u32>,
    terminal_reward: f64,
    bootstrap_discount: f64,
    bootstrap_row_sha256: Option<Digest>,
    realized_return: f64,
    discounted_tick_cost: f64,
}

#[derive(Clone, Copy)]
struct Targets {
    values: [f64; NATIVE_GOAL_REACHABILITY_HEADS],
    present: [bool; NATIVE_GOAL_REACHABILITY_HEADS],
}

#[derive(Clone, Copy)]
struct Prediction {
    values: [f64; NATIVE_GOAL_REACHABILITY_HEADS],
}

struct Forward {
    hidden: Vec<f64>,
    prediction: Prediction,
}

impl NativeGoalReachabilityModel {
    pub fn fit(
        datasets: &[NativeGoalTrajectoryDataset],
        shards: &[NativeEpisodeShard],
        config: NativeGoalReachabilityConfig,
    ) -> Result<Self, NativeGoalReachabilityError> {
        config.validate()?;
        let observation_schemas = datasets
            .iter()
            .map(|dataset| dataset.observation_schema.as_str())
            .collect::<BTreeSet<_>>();
        let action_schemas = datasets
            .iter()
            .map(|dataset| dataset.action_schema.as_str())
            .collect::<BTreeSet<_>>();
        let demonstration_modes = datasets
            .iter()
            .map(|dataset| dataset.config.demonstration_mode)
            .collect::<BTreeSet<_>>();
        if observation_schemas.len() != 1
            || action_schemas.len() != 1
            || demonstration_modes.len() != 1
        {
            return Err(NativeGoalReachabilityError::new(
                "goal reachability datasets mix observation, action, or demonstration-mode contracts",
            ));
        }
        let rows = materialize(datasets, shards)?;
        validate_split_isolation(&rows)?;
        validate_split_support(&rows)?;
        let input_width = rows[0].features.len();
        let training_indices = split_indices(&rows, AuxiliarySplit::Training);
        let training_n_step_bootstrap_rows = training_indices
            .iter()
            .filter(|index| rows[**index].bootstrap_row_sha256.is_some())
            .count();
        if training_n_step_bootstrap_rows == 0 {
            return Err(NativeGoalReachabilityError::new(
                "reachability training split has no n-step bootstrap targets",
            ));
        }
        let (feature_mean, feature_inverse_stddev) =
            fit_normalization(&rows, &training_indices, input_width)?;
        let normalized = rows
            .iter()
            .map(|row| normalize(&row.features, &feature_mean, &feature_inverse_stddev))
            .collect::<Vec<_>>();
        let by_identity = rows
            .iter()
            .enumerate()
            .map(|(index, row)| (row.row_sha256, index))
            .collect::<BTreeMap<_, _>>();
        let grouped = group_training_episodes(&rows, &training_indices);
        let group_ids = grouped.keys().copied().collect::<Vec<_>>();
        let work = training_indices
            .len()
            .checked_mul(usize::from(config.members))
            .and_then(|value| value.checked_mul(usize::from(config.epochs)))
            .ok_or_else(|| NativeGoalReachabilityError::new("training work bound overflowed"))?;
        if work > MAX_GRADIENT_UPDATES {
            return Err(NativeGoalReachabilityError::new(
                "goal reachability gradient work exceeds its bound",
            ));
        }
        let mut rng = Rng::new(config.seed);
        let mut member_jobs = Vec::with_capacity(usize::from(config.members));
        for member_index in 0..usize::from(config.members) {
            let selected_groups = (0..group_ids.len())
                .map(|_| group_ids[(rng.next_u64() % group_ids.len() as u64) as usize])
                .collect::<Vec<_>>();
            let selected_rows = selected_groups
                .iter()
                .flat_map(|group| grouped[group].iter().copied())
                .collect::<Vec<_>>();
            member_jobs.push((member_index, selected_groups, selected_rows));
        }
        // Bootstrap members are statistically and computationally independent.
        // Select their samples above in canonical member order, then train them
        // concurrently without changing any seed, shuffle, or output ordering.
        let mut members = std::thread::scope(|scope| {
            let handles = member_jobs
                .into_iter()
                .map(|(member_index, selected_groups, selected_rows)| {
                    let rows = &rows;
                    let normalized = &normalized;
                    let by_identity = &by_identity;
                    scope.spawn(move || {
                        let mut member_rng =
                            Rng::new(derive_seed(config.seed, member_index as u64));
                        let mut member = ReachabilityMember::initialized(
                            input_width,
                            usize::from(config.hidden_width),
                            selected_groups,
                            &mut member_rng,
                        );
                        let mut order = selected_rows;
                        for _ in 0..config.epochs {
                            let target = member.clone();
                            member_rng.shuffle(&mut order);
                            for row_index in order.iter().copied() {
                                let targets = targets(
                                    &rows[row_index],
                                    &target,
                                    normalized,
                                    by_identity,
                                    usize::from(config.hidden_width),
                                )?;
                                member.update(
                                    &normalized[row_index],
                                    targets,
                                    config,
                                    input_width,
                                )?;
                            }
                            member.target_synchronizations = member
                                .target_synchronizations
                                .checked_add(1)
                                .ok_or_else(|| {
                                    NativeGoalReachabilityError::new(
                                        "target synchronization overflowed",
                                    )
                                })?;
                        }
                        Ok::<_, NativeGoalReachabilityError>((member_index, member))
                    })
                })
                .collect::<Vec<_>>();
            handles
                .into_iter()
                .map(|handle| {
                    handle.join().map_err(|_| {
                        NativeGoalReachabilityError::new(
                            "goal reachability training worker panicked",
                        )
                    })?
                })
                .collect::<Result<Vec<_>, _>>()
        })?;
        members.sort_by_key(|(member_index, _)| *member_index);
        let members = members
            .into_iter()
            .map(|(_, member)| member)
            .collect::<Vec<_>>();
        let baseline = TrainingBaseline::from_rows(&rows, &training_indices)?;
        let training = evaluate(
            &rows,
            &normalized,
            &members,
            AuxiliarySplit::Training,
            baseline,
            usize::from(config.hidden_width),
        )?;
        let validation = evaluate(
            &rows,
            &normalized,
            &members,
            AuxiliarySplit::Validation,
            baseline,
            usize::from(config.hidden_width),
        )?;
        let test = evaluate(
            &rows,
            &normalized,
            &members,
            AuxiliarySplit::Test,
            baseline,
            usize::from(config.hidden_width),
        )?;
        let admission = admission(validation.clone(), config);
        let mut source_dataset_sha256 = datasets
            .iter()
            .map(|dataset| dataset.dataset_sha256)
            .collect::<Vec<_>>();
        source_dataset_sha256.sort_unstable();
        let mut source_replay_corpus_sha256 = datasets
            .iter()
            .map(|dataset| dataset.replay_corpus_sha256)
            .collect::<Vec<_>>();
        source_replay_corpus_sha256.sort_unstable();
        source_replay_corpus_sha256.dedup();
        let mut source_goal_input_sha256 = datasets
            .iter()
            .map(|dataset| dataset.goal.input_sha256)
            .collect::<Vec<_>>();
        source_goal_input_sha256.sort_unstable();
        source_goal_input_sha256.dedup();
        let mut source_goal_objective_identity = datasets
            .iter()
            .map(|dataset| dataset.goal_objective_identity.clone())
            .collect::<Vec<_>>();
        source_goal_objective_identity.sort();
        source_goal_objective_identity.dedup();
        let goal_embedding_width = goal_embedding(&datasets[0].goal)?.len();
        let observation_schema = datasets[0].observation_schema.clone();
        let action_schema = datasets[0].action_schema.clone();
        let model = Self {
            schema: NATIVE_GOAL_REACHABILITY_MODEL_SCHEMA_V1.into(),
            input_schema_sha256: reachability_input_schema_sha256(goal_embedding_width),
            native_feature_schema_sha256: Digest(NATIVE_POLICY_FEATURE_SCHEMA_SHA256),
            goal_embedding_schema_sha256: goal_embedding_schema_sha256(),
            observation_schema,
            action_schema_sha256: text_schema_sha256(
                b"dusklight.native-goal-reachability-action-schema/v1\0",
                &action_schema,
            ),
            action_schema,
            input_width: u32::try_from(input_width)
                .map_err(|_| NativeGoalReachabilityError::new("model input width overflowed"))?,
            goal_embedding_width: u32::try_from(goal_embedding_width)
                .map_err(|_| NativeGoalReachabilityError::new("goal embedding width overflowed"))?,
            source_dataset_sha256,
            source_replay_corpus_sha256,
            source_goal_input_sha256,
            source_goal_objective_identity,
            demonstration_mode: datasets[0].config.demonstration_mode,
            config,
            feature_mean,
            feature_inverse_stddev,
            training_n_step_bootstrap_rows,
            members,
            training,
            validation,
            test,
            admission,
            promotion_authority: false,
            model_sha256: Digest::ZERO,
        };
        // `serde_json` may choose an adjacent representable f64 when parsing a
        // decimal emitted from arithmetic over promoted f32 features. Store the
        // parsed representation before sealing so JSON artifacts are stable on
        // every subsequent decode/encode cycle.
        let canonical_bytes = serde_json::to_vec(&model)
            .map_err(|error| NativeGoalReachabilityError::new(error.to_string()))?;
        let mut model: Self = serde_json::from_slice(&canonical_bytes)
            .map_err(|error| NativeGoalReachabilityError::new(error.to_string()))?;
        model.model_sha256 = model.digest()?;
        model.validate()?;
        Ok(model)
    }

    pub fn validate(&self) -> Result<(), NativeGoalReachabilityError> {
        self.config.validate()?;
        let input_width = self.input_width as usize;
        let hidden_width = usize::from(self.config.hidden_width);
        let goal_width = self.goal_embedding_width as usize;
        let sources_are_canonical = |values: &[Digest]| {
            !values.is_empty()
                && !values.contains(&Digest::ZERO)
                && values.windows(2).all(|pair| pair[0] < pair[1])
        };
        if self.schema != NATIVE_GOAL_REACHABILITY_MODEL_SCHEMA_V1
            || self.native_feature_schema_sha256 != Digest(NATIVE_POLICY_FEATURE_SCHEMA_SHA256)
            || self.goal_embedding_schema_sha256 != goal_embedding_schema_sha256()
            || self.observation_schema.is_empty()
            || self.action_schema.is_empty()
            || self.action_schema_sha256
                != text_schema_sha256(
                    b"dusklight.native-goal-reachability-action-schema/v1\0",
                    &self.action_schema,
                )
            || goal_width == 0
            || input_width != NATIVE_POLICY_FEATURE_WIDTH + goal_width
            || self.input_schema_sha256 != reachability_input_schema_sha256(goal_width)
            || !sources_are_canonical(&self.source_dataset_sha256)
            || !sources_are_canonical(&self.source_replay_corpus_sha256)
            || !sources_are_canonical(&self.source_goal_input_sha256)
            || self.source_goal_objective_identity.is_empty()
            || !self
                .source_goal_objective_identity
                .windows(2)
                .all(|pair| pair[0] < pair[1])
            || self.source_goal_objective_identity.iter().any(|identity| {
                identity.len() != 32
                    || !identity
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            })
            || self.feature_mean.len() != input_width
            || self.feature_inverse_stddev.len() != input_width
            || self
                .feature_mean
                .iter()
                .chain(&self.feature_inverse_stddev)
                .any(|value| !value.is_finite())
            || self
                .feature_inverse_stddev
                .iter()
                .any(|value| *value <= 0.0)
            || self.training_n_step_bootstrap_rows == 0
            || self.training_n_step_bootstrap_rows > self.training.rows
            || self.members.len() != usize::from(self.config.members)
            || self.members.iter().any(|member| {
                !member.valid(
                    input_width,
                    hidden_width,
                    self.config.epochs,
                    self.training.episodes,
                )
            })
            || !self.training.validate()
            || !self.validation.validate()
            || !self.test.validate()
            || self.admission != admission(self.validation.clone(), self.config)
            || self.promotion_authority
            || self.model_sha256 != self.digest()?
        {
            return Err(NativeGoalReachabilityError::new(
                "native goal reachability model is invalid or detached",
            ));
        }
        Ok(())
    }

    pub fn estimate(
        &self,
        observation: &NativeLearningObservation,
        goal: &SemanticGoalInput,
    ) -> Result<NativeGoalReachabilityEstimate, NativeGoalReachabilityError> {
        self.validate()?;
        let native = encode_native_policy_observation(observation)
            .map_err(|error| NativeGoalReachabilityError::new(error.to_string()))?;
        let goal = goal_embedding(goal)?;
        if goal.len() != self.goal_embedding_width as usize {
            return Err(NativeGoalReachabilityError::new(
                "semantic goal embedding width differs from the model",
            ));
        }
        let features = native
            .into_iter()
            .map(f64::from)
            .chain(goal)
            .collect::<Vec<_>>();
        let normalized = normalize(&features, &self.feature_mean, &self.feature_inverse_stddev);
        ensemble_estimate(
            &self.members,
            &normalized,
            usize::from(self.config.hidden_width),
        )
    }

    fn digest(&self) -> Result<Digest, NativeGoalReachabilityError> {
        let mut canonical = self.clone();
        canonical.model_sha256 = Digest::ZERO;
        canonical_digest(b"dusklight.native-goal-reachability-model/v1\0", &canonical)
    }
}

impl ReachabilityMember {
    fn initialized(
        input_width: usize,
        hidden_width: usize,
        bootstrap_episode_sha256: Vec<Digest>,
        rng: &mut Rng,
    ) -> Self {
        let mut output_bias = vec![0.0; NATIVE_GOAL_REACHABILITY_HEADS];
        let one_tick = normalize_ticks(1.0);
        output_bias[HEAD_TIME] = logit(one_tick);
        output_bias[HEAD_COST] = logit(one_tick);
        Self {
            bootstrap_episode_sha256,
            hidden_weights: initialized_weights(hidden_width, input_width, rng),
            hidden_bias: vec![0.0; hidden_width],
            output_weights: initialized_weights(NATIVE_GOAL_REACHABILITY_HEADS, hidden_width, rng),
            output_bias,
            gradient_updates: 0,
            target_synchronizations: 0,
        }
    }

    fn valid(
        &self,
        input_width: usize,
        hidden_width: usize,
        epochs: u16,
        training_episodes: usize,
    ) -> bool {
        self.bootstrap_episode_sha256.len() == training_episodes
            && !self.bootstrap_episode_sha256.contains(&Digest::ZERO)
            && self.hidden_weights.len() == input_width * hidden_width
            && self.hidden_bias.len() == hidden_width
            && self.output_weights.len() == hidden_width * NATIVE_GOAL_REACHABILITY_HEADS
            && self.output_bias.len() == NATIVE_GOAL_REACHABILITY_HEADS
            && self
                .hidden_weights
                .iter()
                .chain(&self.hidden_bias)
                .chain(&self.output_weights)
                .chain(&self.output_bias)
                .all(|value| value.is_finite())
            && self.gradient_updates > 0
            && self.target_synchronizations == u32::from(epochs)
    }

    fn forward(&self, features: &[f64], hidden_width: usize) -> Forward {
        let input_width = features.len();
        let hidden = (0..hidden_width)
            .map(|unit| {
                (dot(
                    features,
                    &self.hidden_weights[unit * input_width..(unit + 1) * input_width],
                ) + self.hidden_bias[unit])
                    .tanh()
            })
            .collect::<Vec<_>>();
        let values = std::array::from_fn(|head| {
            logistic(
                dot(
                    &hidden,
                    &self.output_weights[head * hidden_width..(head + 1) * hidden_width],
                ) + self.output_bias[head],
            )
        });
        Forward {
            hidden,
            prediction: Prediction { values },
        }
    }

    fn update(
        &mut self,
        features: &[f64],
        targets: Targets,
        config: NativeGoalReachabilityConfig,
        input_width: usize,
    ) -> Result<(), NativeGoalReachabilityError> {
        let hidden_width = usize::from(config.hidden_width);
        let forward = self.forward(features, hidden_width);
        let output_before = self.output_weights.clone();
        let present = targets.present.iter().filter(|value| **value).count() as f64;
        let mut d_raw = [0.0; NATIVE_GOAL_REACHABILITY_HEADS];
        for (head, raw_gradient) in d_raw.iter_mut().enumerate() {
            if !targets.present[head] {
                continue;
            }
            let prediction = forward.prediction.values[head];
            // Binary cross-entropy with soft labels is well-defined for the
            // bounded regression heads and avoids vanishing gradients near the
            // low normalized time/cost values common in short native episodes.
            let gradient = (prediction - targets.values[head]) / present;
            *raw_gradient = clip(gradient, config.gradient_clip);
            for hidden in 0..hidden_width {
                let parameter = head * hidden_width + hidden;
                let weight_gradient = *raw_gradient * forward.hidden[hidden]
                    + config.l2_penalty * self.output_weights[parameter];
                self.output_weights[parameter] -=
                    config.learning_rate * clip(weight_gradient, config.gradient_clip);
            }
            self.output_bias[head] -= config.learning_rate * *raw_gradient;
        }
        for hidden in 0..hidden_width {
            let d_hidden = (0..NATIVE_GOAL_REACHABILITY_HEADS)
                .map(|head| d_raw[head] * output_before[head * hidden_width + hidden])
                .sum::<f64>()
                * (1.0 - forward.hidden[hidden].powi(2));
            for (input, feature) in features.iter().copied().enumerate().take(input_width) {
                let parameter = hidden * input_width + input;
                let gradient =
                    d_hidden * feature + config.l2_penalty * self.hidden_weights[parameter];
                self.hidden_weights[parameter] -=
                    config.learning_rate * clip(gradient, config.gradient_clip);
            }
            self.hidden_bias[hidden] -= config.learning_rate * clip(d_hidden, config.gradient_clip);
        }
        if self
            .hidden_weights
            .iter()
            .chain(&self.hidden_bias)
            .chain(&self.output_weights)
            .chain(&self.output_bias)
            .any(|value| !value.is_finite())
        {
            return Err(NativeGoalReachabilityError::new(
                "goal reachability update became non-finite",
            ));
        }
        self.gradient_updates = self
            .gradient_updates
            .checked_add(1)
            .ok_or_else(|| NativeGoalReachabilityError::new("gradient update count overflowed"))?;
        Ok(())
    }
}

fn materialize(
    datasets: &[NativeGoalTrajectoryDataset],
    shards: &[NativeEpisodeShard],
) -> Result<Vec<MaterializedRow>, NativeGoalReachabilityError> {
    if datasets.is_empty() || datasets.len() > MAX_DATASETS || shards.is_empty() {
        return Err(NativeGoalReachabilityError::new(
            "goal reachability requires bounded datasets and native shards",
        ));
    }
    let mut shard_by_digest = BTreeMap::new();
    for shard in shards {
        if shard_by_digest
            .insert(shard.content_sha256, shard)
            .is_some()
        {
            return Err(NativeGoalReachabilityError::new(
                "goal reachability received a duplicate native shard",
            ));
        }
    }
    let mut rows = Vec::new();
    let mut row_identities = BTreeSet::new();
    let mut dataset_identities = BTreeSet::new();
    let mut goal_width = None;
    for dataset in datasets {
        dataset
            .validate()
            .map_err(|error| NativeGoalReachabilityError::new(error.to_string()))?;
        if !dataset_identities.insert(dataset.dataset_sha256) {
            return Err(NativeGoalReachabilityError::new(
                "goal reachability received a duplicate dataset",
            ));
        }
        let goal = goal_embedding(&dataset.goal)?;
        if goal_width
            .replace(goal.len())
            .is_some_and(|width| width != goal.len())
        {
            return Err(NativeGoalReachabilityError::new(
                "semantic goal embeddings have inconsistent widths",
            ));
        }
        for row in &dataset.rows {
            if !row_identities.insert(row.row_sha256) {
                return Err(NativeGoalReachabilityError::new(
                    "goal reachability datasets duplicate a trajectory row",
                ));
            }
            let shard = shard_by_digest.get(&row.shard_sha256).ok_or_else(|| {
                NativeGoalReachabilityError::new(format!(
                    "goal reachability is missing native shard {}",
                    row.shard_sha256
                ))
            })?;
            if shard.metadata.observation_schema != dataset.observation_schema
                || shard.metadata.action_schema != dataset.action_schema
                || shard.metadata.objective != dataset.goal_definition_name
                || shard.metadata.objective_identity != dataset.goal_objective_identity
            {
                return Err(NativeGoalReachabilityError::new(
                    "goal reachability shard metadata differs from the dataset",
                ));
            }
            let episode = shard
                .episodes
                .iter()
                .find(|episode| episode.id == row.episode_id)
                .ok_or_else(|| NativeGoalReachabilityError::new("trajectory episode is absent"))?;
            if hex_128(episode.payload_xxh3_128) != row.episode_payload_xxh3_128
                || episode.ticks_executed != row.episode_ticks
                || episode.success != row.success
            {
                return Err(NativeGoalReachabilityError::new(
                    "trajectory episode identity or outcome differs from its dataset row",
                ));
            }
            let step = episode
                .steps
                .get(row.step_index as usize)
                .ok_or_else(|| NativeGoalReachabilityError::new("trajectory step is absent"))?;
            if hex_128(step.pre_input.state_identity) != row.pre_input_state_xxh3_128
                || pad_target(step.consumed_pad) != row.consumed_pad
            {
                return Err(NativeGoalReachabilityError::new(
                    "trajectory state or consumed PAD differs from its native source",
                ));
            }
            let native = encode_native_policy_observation(&step.pre_input)
                .map_err(|error| NativeGoalReachabilityError::new(error.to_string()))?;
            let features = native
                .into_iter()
                .map(f64::from)
                .chain(goal.iter().copied())
                .collect::<Vec<_>>();
            let episode_sha256 = episode_identity(row);
            rows.push(MaterializedRow {
                row_sha256: row.row_sha256,
                episode_sha256,
                split: row.split,
                features,
                success: row.success,
                ticks_to_goal: row.ticks_to_goal,
                terminal_reward: row.terminal_reward_millionths as f64 / RETURN_SCALE as f64,
                bootstrap_discount: row.bootstrap_discount_millionths as f64 / RETURN_SCALE as f64,
                bootstrap_row_sha256: row.bootstrap_row_sha256,
                realized_return: row.realized_return_millionths as f64 / RETURN_SCALE as f64,
                discounted_tick_cost: row.discounted_tick_cost_millionths as f64
                    / RETURN_SCALE as f64,
            });
            if rows.len() > MAX_ROWS {
                return Err(NativeGoalReachabilityError::new(
                    "goal reachability row limit exceeded",
                ));
            }
        }
    }
    if rows.is_empty() {
        return Err(NativeGoalReachabilityError::new(
            "goal reachability has no materialized rows",
        ));
    }
    let cells = rows
        .len()
        .checked_mul(rows[0].features.len())
        .ok_or_else(|| NativeGoalReachabilityError::new("input cell count overflowed"))?;
    if cells > MAX_INPUT_CELLS
        || rows.iter().any(|row| {
            row.features.len() != rows[0].features.len()
                || row.features.iter().any(|value| !value.is_finite())
        })
    {
        return Err(NativeGoalReachabilityError::new(
            "goal reachability input matrix is invalid or too large",
        ));
    }
    Ok(rows)
}

fn validate_split_isolation(rows: &[MaterializedRow]) -> Result<(), NativeGoalReachabilityError> {
    let mut episode_splits = BTreeMap::new();
    let by_identity = rows
        .iter()
        .map(|row| (row.row_sha256, row))
        .collect::<BTreeMap<_, _>>();
    for row in rows {
        if episode_splits
            .insert(row.episode_sha256, row.split)
            .is_some_and(|prior| prior != row.split)
        {
            return Err(NativeGoalReachabilityError::new(
                "one physical episode crosses goal reachability splits",
            ));
        }
        if row.bootstrap_row_sha256.is_some_and(|identity| {
            by_identity.get(&identity).is_none_or(|target| {
                target.episode_sha256 != row.episode_sha256 || target.split != row.split
            })
        }) {
            return Err(NativeGoalReachabilityError::new(
                "n-step target crosses a physical episode or split",
            ));
        }
    }
    Ok(())
}

fn validate_split_support(rows: &[MaterializedRow]) -> Result<(), NativeGoalReachabilityError> {
    for split in [
        AuxiliarySplit::Training,
        AuxiliarySplit::Validation,
        AuxiliarySplit::Test,
    ] {
        let split_rows = rows
            .iter()
            .filter(|row| row.split == split)
            .collect::<Vec<_>>();
        if split_rows.is_empty()
            || !split_rows.iter().any(|row| row.success)
            || !split_rows.iter().any(|row| !row.success)
        {
            return Err(NativeGoalReachabilityError::new(
                "each reachability split requires successful and failed decisions",
            ));
        }
    }
    Ok(())
}

fn targets(
    row: &MaterializedRow,
    target: &ReachabilityMember,
    normalized: &[Vec<f64>],
    by_identity: &BTreeMap<Digest, usize>,
    hidden_width: usize,
) -> Result<Targets, NativeGoalReachabilityError> {
    let bootstrap = row
        .bootstrap_row_sha256
        .map(|identity| {
            let index = *by_identity.get(&identity).ok_or_else(|| {
                NativeGoalReachabilityError::new("n-step bootstrap row is absent")
            })?;
            Ok(target.forward(&normalized[index], hidden_width).prediction)
        })
        .transpose()?;
    let return_target = row.terminal_reward
        + row.bootstrap_discount * bootstrap.map_or(0.0, |value| value.values[HEAD_RETURN]);
    let bootstrap_cost = bootstrap
        .map(|value| denormalize_ticks(value.values[HEAD_COST]))
        .unwrap_or(0.0);
    let cost_target = row.discounted_tick_cost + row.bootstrap_discount * bootstrap_cost;
    let time_target = row
        .ticks_to_goal
        .map(|ticks| normalize_ticks(f64::from(ticks)))
        .unwrap_or(0.0);
    let values = [
        f64::from(row.success),
        time_target,
        return_target.clamp(0.0, 1.0),
        normalize_ticks(cost_target),
    ];
    if values
        .iter()
        .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
    {
        return Err(NativeGoalReachabilityError::new(
            "goal reachability target is non-finite or unbounded",
        ));
    }
    Ok(Targets {
        values,
        present: [true, row.success, true, true],
    })
}

#[derive(Clone, Copy)]
struct TrainingBaseline {
    success_probability: f64,
    successful_time_ticks: f64,
    discounted_return: f64,
    discounted_tick_cost: f64,
}

impl TrainingBaseline {
    fn from_rows(
        rows: &[MaterializedRow],
        indices: &[usize],
    ) -> Result<Self, NativeGoalReachabilityError> {
        let success_probability = indices.iter().filter(|index| rows[**index].success).count()
            as f64
            / indices.len() as f64;
        let successful_times = indices
            .iter()
            .filter_map(|index| rows[*index].ticks_to_goal.map(f64::from))
            .collect::<Vec<_>>();
        let discounted_return = indices
            .iter()
            .map(|index| realized_return(&rows[*index]))
            .sum::<f64>()
            / indices.len() as f64;
        let by_identity = rows
            .iter()
            .map(|row| (row.row_sha256, row))
            .collect::<BTreeMap<_, _>>();
        let discounted_tick_cost = indices
            .iter()
            .map(|index| realized_cost(&by_identity, &rows[*index]))
            .collect::<Result<Vec<_>, _>>()?
            .iter()
            .sum::<f64>()
            / indices.len() as f64;
        if successful_times.is_empty() {
            return Err(NativeGoalReachabilityError::new(
                "training baseline has no successful time targets",
            ));
        }
        Ok(Self {
            success_probability,
            successful_time_ticks: successful_times.iter().sum::<f64>()
                / successful_times.len() as f64,
            discounted_return,
            discounted_tick_cost,
        })
    }
}

fn evaluate(
    rows: &[MaterializedRow],
    normalized: &[Vec<f64>],
    members: &[ReachabilityMember],
    split: AuxiliarySplit,
    baseline: TrainingBaseline,
    hidden_width: usize,
) -> Result<NativeGoalReachabilityMetrics, NativeGoalReachabilityError> {
    let indices = split_indices(rows, split);
    let mut reachability_squared = 0.0;
    let mut baseline_reachability_squared = 0.0;
    let mut time_absolute = 0.0;
    let mut baseline_time_absolute = 0.0;
    let mut time_support = 0_usize;
    let mut return_squared = 0.0;
    let mut baseline_return_squared = 0.0;
    let mut cost_absolute = 0.0;
    let mut baseline_cost_absolute = 0.0;
    let mut reachability_stddev = 0.0;
    let mut return_stddev = 0.0;
    let mut successful_rows = 0;
    let by_identity = rows
        .iter()
        .map(|row| (row.row_sha256, row))
        .collect::<BTreeMap<_, _>>();
    for index in &indices {
        let row = &rows[*index];
        let estimate = ensemble_estimate(members, &normalized[*index], hidden_width)?;
        let expected_success = f64::from(row.success);
        reachability_squared += (estimate.reachability_probability - expected_success).powi(2);
        baseline_reachability_squared += (baseline.success_probability - expected_success).powi(2);
        if let Some(ticks) = row.ticks_to_goal {
            time_absolute += (estimate.expected_ticks_to_goal - f64::from(ticks)).abs();
            baseline_time_absolute += (baseline.successful_time_ticks - f64::from(ticks)).abs();
            time_support += 1;
            successful_rows += 1;
        }
        let expected_return = realized_return(row);
        return_squared += (estimate.discounted_terminal_return - expected_return).powi(2);
        baseline_return_squared += (baseline.discounted_return - expected_return).powi(2);
        let expected_cost = realized_cost(&by_identity, row)?;
        cost_absolute += (estimate.discounted_tick_cost - expected_cost).abs();
        baseline_cost_absolute += (baseline.discounted_tick_cost - expected_cost).abs();
        reachability_stddev += estimate.reachability_stddev;
        return_stddev += estimate.return_stddev;
    }
    let count = indices.len() as f64;
    let reachability_brier = reachability_squared / count;
    let baseline_reachability_brier = baseline_reachability_squared / count;
    let discounted_return_rmse = (return_squared / count).sqrt();
    let baseline_discounted_return_rmse = (baseline_return_squared / count).sqrt();
    let metrics = NativeGoalReachabilityMetrics {
        rows: indices.len(),
        episodes: indices
            .iter()
            .map(|index| rows[*index].episode_sha256)
            .collect::<BTreeSet<_>>()
            .len(),
        successful_rows,
        failed_rows: indices.len() - successful_rows,
        reachability_brier,
        baseline_reachability_brier,
        reachability_relative_improvement: relative_improvement(
            baseline_reachability_brier,
            reachability_brier,
        ),
        successful_time_mae_ticks: time_absolute / time_support as f64,
        baseline_successful_time_mae_ticks: baseline_time_absolute / time_support as f64,
        successful_time_relative_improvement: relative_improvement(
            baseline_time_absolute / time_support as f64,
            time_absolute / time_support as f64,
        ),
        discounted_return_rmse,
        baseline_discounted_return_rmse,
        return_relative_improvement: relative_improvement(
            baseline_discounted_return_rmse,
            discounted_return_rmse,
        ),
        discounted_tick_cost_mae: cost_absolute / count,
        baseline_discounted_tick_cost_mae: baseline_cost_absolute / count,
        tick_cost_relative_improvement: relative_improvement(
            baseline_cost_absolute / count,
            cost_absolute / count,
        ),
        mean_reachability_stddev: reachability_stddev / count,
        mean_return_stddev: return_stddev / count,
    };
    if !metrics.validate() {
        return Err(NativeGoalReachabilityError::new(
            "goal reachability metrics are invalid",
        ));
    }
    Ok(metrics)
}

fn admission(
    validation: NativeGoalReachabilityMetrics,
    config: NativeGoalReachabilityConfig,
) -> NativeGoalReachabilityAdmission {
    if validation.reachability_relative_improvement >= config.minimum_validation_improvement
        && validation.return_relative_improvement >= config.minimum_validation_improvement
        && validation.successful_time_relative_improvement >= config.minimum_validation_improvement
        && validation.tick_cost_relative_improvement >= config.minimum_validation_improvement
        && validation.mean_reachability_stddev <= config.maximum_validation_reachability_stddev
    {
        NativeGoalReachabilityAdmission::GoalConditionedCandidate
    } else {
        NativeGoalReachabilityAdmission::RetainTrainingMeanBaseline
    }
}

fn ensemble_estimate(
    members: &[ReachabilityMember],
    features: &[f64],
    hidden_width: usize,
) -> Result<NativeGoalReachabilityEstimate, NativeGoalReachabilityError> {
    let predictions = members
        .iter()
        .map(|member| member.forward(features, hidden_width).prediction)
        .collect::<Vec<_>>();
    if predictions.is_empty() {
        return Err(NativeGoalReachabilityError::new(
            "reachability ensemble has no members",
        ));
    }
    let mean = std::array::from_fn::<_, NATIVE_GOAL_REACHABILITY_HEADS, _>(|head| {
        predictions
            .iter()
            .map(|prediction| prediction.values[head])
            .sum::<f64>()
            / predictions.len() as f64
    });
    let stddev = |head: usize| {
        (predictions
            .iter()
            .map(|prediction| (prediction.values[head] - mean[head]).powi(2))
            .sum::<f64>()
            / predictions.len() as f64)
            .sqrt()
    };
    let estimate = NativeGoalReachabilityEstimate {
        reachability_probability: mean[HEAD_SUCCESS],
        reachability_stddev: stddev(HEAD_SUCCESS),
        expected_ticks_to_goal: denormalize_ticks(mean[HEAD_TIME]),
        discounted_terminal_return: mean[HEAD_RETURN],
        return_stddev: stddev(HEAD_RETURN),
        discounted_tick_cost: denormalize_ticks(mean[HEAD_COST]),
    };
    if [
        estimate.reachability_probability,
        estimate.reachability_stddev,
        estimate.expected_ticks_to_goal,
        estimate.discounted_terminal_return,
        estimate.return_stddev,
        estimate.discounted_tick_cost,
    ]
    .iter()
    .any(|value| !value.is_finite() || *value < 0.0)
    {
        return Err(NativeGoalReachabilityError::new(
            "reachability ensemble estimate is invalid",
        ));
    }
    Ok(estimate)
}

fn realized_return(row: &MaterializedRow) -> f64 {
    row.realized_return
}

fn realized_cost(
    by_identity: &BTreeMap<Digest, &MaterializedRow>,
    row: &MaterializedRow,
) -> Result<f64, NativeGoalReachabilityError> {
    let mut total = row.discounted_tick_cost;
    let mut discount = row.bootstrap_discount;
    let mut next = row.bootstrap_row_sha256;
    let mut visited = BTreeSet::new();
    while let Some(identity) = next {
        if !visited.insert(identity) {
            return Err(NativeGoalReachabilityError::new(
                "discounted cost bootstrap contains a cycle",
            ));
        }
        let target = by_identity.get(&identity).ok_or_else(|| {
            NativeGoalReachabilityError::new("discounted cost bootstrap row is absent")
        })?;
        total += discount * target.discounted_tick_cost;
        discount *= target.bootstrap_discount;
        next = target.bootstrap_row_sha256;
    }
    Ok(total)
}

fn relative_improvement(baseline: f64, model: f64) -> f64 {
    if baseline > f64::EPSILON {
        ((baseline - model) / baseline).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn fit_normalization(
    rows: &[MaterializedRow],
    training: &[usize],
    width: usize,
) -> Result<(Vec<f64>, Vec<f64>), NativeGoalReachabilityError> {
    let mut mean = vec![0.0; width];
    for index in training {
        for (output, feature) in mean.iter_mut().zip(&rows[*index].features) {
            *output += feature;
        }
    }
    for value in &mut mean {
        *value /= training.len() as f64;
    }
    let mut variance = vec![0.0; width];
    for index in training {
        for ((output, feature), mean) in variance.iter_mut().zip(&rows[*index].features).zip(&mean)
        {
            *output += (feature - mean).powi(2);
        }
    }
    let inverse_stddev = variance
        .into_iter()
        .map(|value| {
            let stddev = (value / training.len() as f64).sqrt();
            if stddev > 1.0e-8 { 1.0 / stddev } else { 1.0 }
        })
        .collect::<Vec<_>>();
    if mean
        .iter()
        .chain(&inverse_stddev)
        .any(|value| !value.is_finite())
    {
        return Err(NativeGoalReachabilityError::new(
            "training-only normalization became non-finite",
        ));
    }
    Ok((mean, inverse_stddev))
}

fn normalize(features: &[f64], mean: &[f64], inverse_stddev: &[f64]) -> Vec<f64> {
    features
        .iter()
        .zip(mean)
        .zip(inverse_stddev)
        .map(|((value, mean), inverse)| (value - mean) * inverse)
        .collect()
}

fn split_indices(rows: &[MaterializedRow], split: AuxiliarySplit) -> Vec<usize> {
    rows.iter()
        .enumerate()
        .filter_map(|(index, row)| (row.split == split).then_some(index))
        .collect()
}

fn group_training_episodes(
    rows: &[MaterializedRow],
    training: &[usize],
) -> BTreeMap<Digest, Vec<usize>> {
    let mut groups = BTreeMap::new();
    for index in training {
        groups
            .entry(rows[*index].episode_sha256)
            .or_insert_with(Vec::new)
            .push(*index);
    }
    groups
}

fn episode_identity(row: &NativeGoalTrajectoryRow) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight.physical-native-episode/v1\0");
    hasher.update(row.shard_sha256.0);
    hasher.update(row.episode_payload_xxh3_128.as_bytes());
    Digest(hasher.finalize().into())
}

/// Fixed semantic embedding. Provenance digests and names are intentionally
/// absent; only typed values, masks, graph roles, roots, and projections enter.
pub fn goal_embedding(goal: &SemanticGoalInput) -> Result<Vec<f64>, NativeGoalReachabilityError> {
    goal.validate()
        .map_err(|error| NativeGoalReachabilityError::new(error.to_string()))?;
    let node_count = goal.node_features.len();
    let mut output = goal
        .metadata
        .iter()
        .map(|value| f64::from(*value))
        .collect::<Vec<_>>();
    output.extend([
        node_count as f64 / f64::from(u16::MAX),
        goal.edges.len() as f64 / f64::from(u16::MAX),
        goal.roots.len() as f64 / f64::from(u16::MAX),
        goal.projection_features.len() as f64 / f64::from(u16::MAX),
    ]);
    append_mean_max(&mut output, &goal.node_features, GOAL_NODE_FEATURE_WIDTH);
    append_mean(
        &mut output,
        &goal.node_feature_masks,
        GOAL_NODE_FEATURE_WIDTH,
    );
    for role in [
        GoalEdgeRole::UnaryChild,
        GoalEdgeRole::LeftChild,
        GoalEdgeRole::RightChild,
    ] {
        let mut messages = vec![vec![0.0_f32; GOAL_NODE_FEATURE_WIDTH]; node_count];
        let mut counts = vec![0_u32; node_count];
        for edge in goal.edges.iter().filter(|edge| edge.role == role) {
            let source = usize::from(edge.source);
            let target = usize::from(edge.target);
            counts[target] += 1;
            for (feature, message) in messages[target].iter_mut().enumerate() {
                *message +=
                    goal.node_features[source][feature] * goal.node_feature_masks[source][feature];
            }
        }
        for (message, count) in messages.iter_mut().zip(counts) {
            if count != 0 {
                for value in message {
                    *value /= count as f32;
                }
            }
        }
        append_mean_max(&mut output, &messages, GOAL_NODE_FEATURE_WIDTH);
    }
    let roots = goal
        .roots
        .iter()
        .map(|root| goal.node_features[usize::from(root.node)].clone())
        .collect::<Vec<_>>();
    append_mean_max(&mut output, &roots, GOAL_NODE_FEATURE_WIDTH);
    append_mean_max(
        &mut output,
        &goal.projection_features,
        GOAL_PROJECTION_FEATURE_WIDTH,
    );
    append_mean(
        &mut output,
        &goal.projection_feature_masks,
        GOAL_PROJECTION_FEATURE_WIDTH,
    );
    if output.is_empty() || output.iter().any(|value| !value.is_finite()) {
        return Err(NativeGoalReachabilityError::new(
            "semantic goal embedding is invalid",
        ));
    }
    Ok(output)
}

fn append_mean_max(output: &mut Vec<f64>, rows: &[Vec<f32>], width: usize) {
    append_mean(output, rows, width);
    let mut maximum = vec![f64::NEG_INFINITY; width];
    for row in rows {
        for (maximum, value) in maximum.iter_mut().zip(row) {
            *maximum = maximum.max(f64::from(*value));
        }
    }
    if rows.is_empty() {
        maximum.fill(0.0);
    }
    output.extend(maximum);
}

fn append_mean(output: &mut Vec<f64>, rows: &[Vec<f32>], width: usize) {
    let mut mean = vec![0.0; width];
    for row in rows {
        for (mean, value) in mean.iter_mut().zip(row) {
            *mean += f64::from(*value);
        }
    }
    if !rows.is_empty() {
        for value in &mut mean {
            *value /= rows.len() as f64;
        }
    }
    output.extend(mean);
}

pub fn goal_embedding_schema_sha256() -> Digest {
    Digest(Sha256::digest(format!(
        "{NATIVE_GOAL_EMBEDDING_SCHEMA_V1}\nmetadata={GOAL_METADATA_WIDTH}\nnode={GOAL_NODE_FEATURE_WIDTH}\nprojection={GOAL_PROJECTION_FEATURE_WIDTH}\nraw=mean,max,mask_mean\nedges=unary,left,right:masked_source_mean,max\nroots=mean,max\nprojections=mean,max,mask_mean\nprovenance_features=none\n"
    )).into())
}

fn reachability_input_schema_sha256(goal_width: usize) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight.native-goal-reachability-input/v1\0");
    hasher.update(NATIVE_POLICY_FEATURE_SCHEMA_SHA256);
    hasher.update(goal_embedding_schema_sha256().0);
    hasher.update((NATIVE_POLICY_FEATURE_WIDTH as u64).to_le_bytes());
    hasher.update((goal_width as u64).to_le_bytes());
    Digest(hasher.finalize().into())
}

fn text_schema_sha256(domain: &[u8], value: &str) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update((value.len() as u64).to_le_bytes());
    hasher.update(value.as_bytes());
    Digest(hasher.finalize().into())
}

fn normalize_ticks(value: f64) -> f64 {
    value.max(0.0).ln_1p() / f64::from(MAX_GOAL_REACHABILITY_TICKS).ln_1p()
}

fn denormalize_ticks(value: f64) -> f64 {
    (value.clamp(0.0, 1.0) * f64::from(MAX_GOAL_REACHABILITY_TICKS).ln_1p()).exp_m1()
}

fn pad_target(pad: NativeRawPad) -> AuxiliaryPadTarget {
    AuxiliaryPadTarget {
        buttons: pad.buttons,
        stick_x: pad.stick_x,
        stick_y: pad.stick_y,
        substick_x: pad.substick_x,
        substick_y: pad.substick_y,
        trigger_left: pad.trigger_left,
        trigger_right: pad.trigger_right,
        analog_a: pad.analog_a,
        analog_b: pad.analog_b,
    }
}

fn initialized_weights(rows: usize, columns: usize, rng: &mut Rng) -> Vec<f64> {
    let scale = (6.0 / (rows + columns) as f64).sqrt();
    (0..rows * columns)
        .map(|_| (rng.unit() * 2.0 - 1.0) * scale)
        .collect()
}

fn dot(left: &[f64], right: &[f64]) -> f64 {
    left.iter()
        .zip(right)
        .map(|(left, right)| left * right)
        .sum()
}

fn logistic(value: f64) -> f64 {
    if value >= 0.0 {
        1.0 / (1.0 + (-value).exp())
    } else {
        let exponential = value.exp();
        exponential / (1.0 + exponential)
    }
}

fn logit(probability: f64) -> f64 {
    (probability / (1.0 - probability)).ln()
}

fn clip(value: f64, maximum: f64) -> f64 {
    value.clamp(-maximum, maximum)
}

fn derive_seed(seed: u64, stream: u64) -> u64 {
    seed ^ stream.wrapping_add(1).wrapping_mul(0x9e37_79b9_7f4a_7c15)
}

struct Rng {
    state: u64,
}

impl Rng {
    fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 {
                0x9e37_79b9_7f4a_7c15
            } else {
                seed
            },
        }
    }

    fn next_u64(&mut self) -> u64 {
        let mut value = self.state;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.state = value;
        value
    }

    fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1_u64 << 53) as f64
    }

    fn shuffle<T>(&mut self, values: &mut [T]) {
        for index in (1..values.len()).rev() {
            values.swap(index, (self.next_u64() % (index as u64 + 1)) as usize);
        }
    }
}

fn hex_128(value: [u8; 16]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn canonical_digest<T: Serialize>(
    domain: &[u8],
    value: &T,
) -> Result<Digest, NativeGoalReachabilityError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| NativeGoalReachabilityError::new(error.to_string()))?;
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
    Ok(Digest(hasher.finalize().into()))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeGoalReachabilityError(String);

impl NativeGoalReachabilityError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for NativeGoalReachabilityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for NativeGoalReachabilityError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiled_goal_graph::CompiledGoalGraph;
    use crate::milestone_dsl::compile_source;
    use crate::native_goal_trajectory::NativeGoalTrajectoryConfig;
    use crate::native_replay_corpus::{
        NativeReplayCorpus, ReplayEpisodeSource, ReplayExperienceRole,
    };
    use crate::semantic_goal_input::SemanticGoalInput;
    use dusklight_evidence::native_episode_shard::authored_milestone_objective_identity;

    const GOAL_SOURCE: &str = r#"milestones 1.8
milestone reach_goal {
  phase post_sim
  when stage.room == 1
}
"#;

    fn graph(source: &str, name: &str) -> CompiledGoalGraph {
        let compiled = compile_source(source).unwrap();
        let index = compiled
            .definitions
            .iter()
            .position(|definition| definition.name == name)
            .unwrap();
        CompiledGoalGraph::from_compiled(&compiled, index).unwrap()
    }

    fn balanced_sources() -> (
        NativeEpisodeShard,
        CompiledGoalGraph,
        NativeReplayCorpus,
        NativeGoalTrajectoryDataset,
    ) {
        let mut shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v14.dseps"
        ))
        .unwrap();
        let graph = graph(GOAL_SOURCE, "reach_goal");
        shard.metadata.objective = graph.definition_name.clone();
        shard.metadata.objective_identity = authored_milestone_objective_identity(
            &graph.program_sha256.to_string(),
            &graph.definition_sha256.to_string(),
        )
        .unwrap();
        let success_template = shard
            .episodes
            .iter()
            .find(|episode| episode.success)
            .unwrap()
            .clone();
        let failure_template = shard
            .episodes
            .iter()
            .find(|episode| !episode.success)
            .unwrap()
            .clone();
        shard.episodes = (0..120_u32)
            .map(|episode_index| {
                let success = episode_index % 2 == 0;
                let mut episode = if success {
                    success_template.clone()
                } else {
                    failure_template.clone()
                };
                episode.id = format!("episode-{episode_index:04}");
                let digest = Sha256::digest(episode.id.as_bytes());
                episode.payload_xxh3_128.copy_from_slice(&digest[..16]);
                episode.success = success;
                episode.ticks_executed = 5;
                episode.first_hit_tick = success.then_some(4);
                let template = episode.steps[0].clone();
                episode.steps = (0..5_u32)
                    .map(|step_index| {
                        let mut step = template.clone();
                        step.pre_input.player_position[0] = if success { 20.0 } else { -20.0 };
                        step.pre_input.remaining_ticks = 5 - step_index;
                        step.pre_input.state_identity =
                            state_identity(episode_index, step_index, 0);
                        step.post_simulation.state_identity =
                            state_identity(episode_index, step_index, 1);
                        step
                    })
                    .collect();
                episode
            })
            .collect();
        let replay_sources = shard
            .episodes
            .iter()
            .enumerate()
            .map(|(episode_index, _)| ReplayEpisodeSource {
                shard: &shard,
                episode_index,
                role: ReplayExperienceRole::RandomizedCoverage,
                policy_lineage_sha256: None,
                parent_entry_sha256: None,
            })
            .collect::<Vec<_>>();
        let corpus = NativeReplayCorpus::build(None, &replay_sources).unwrap();
        let dataset = (0..10_000_u64)
            .find_map(|split_seed| {
                let config = NativeGoalTrajectoryConfig {
                    demonstration_mode: DemonstrationMode::BehaviorCloningWarmStart,
                    n_step: 2,
                    discount_millionths: 900_000,
                    training_basis_points: 6_000,
                    validation_basis_points: 2_000,
                    split_seed,
                };
                let dataset = NativeGoalTrajectoryDataset::build(
                    &corpus,
                    std::slice::from_ref(&shard),
                    &graph,
                    config,
                )
                .unwrap();
                split_has_both(&dataset, AuxiliarySplit::Training)
                    .then_some(dataset)
                    .filter(|dataset| split_has_both(dataset, AuxiliarySplit::Validation))
                    .filter(|dataset| split_has_both(dataset, AuxiliarySplit::Test))
            })
            .expect("a balanced deterministic episode split");
        (shard, graph, corpus, dataset)
    }

    fn state_identity(episode: u32, step: u32, phase: u8) -> [u8; 16] {
        let mut hasher = Sha256::new();
        hasher.update(episode.to_le_bytes());
        hasher.update(step.to_le_bytes());
        hasher.update([phase]);
        hasher.finalize()[..16].try_into().unwrap()
    }

    fn split_has_both(dataset: &NativeGoalTrajectoryDataset, split: AuxiliarySplit) -> bool {
        dataset
            .rows
            .iter()
            .any(|row| row.split == split && row.success)
            && dataset
                .rows
                .iter()
                .any(|row| row.split == split && !row.success)
    }

    fn fit_config() -> NativeGoalReachabilityConfig {
        NativeGoalReachabilityConfig {
            members: 3,
            epochs: 18,
            hidden_width: 4,
            learning_rate: 0.02,
            l2_penalty: 1.0e-6,
            gradient_clip: 5.0,
            minimum_validation_improvement: 0.02,
            maximum_validation_reachability_stddev: 0.5,
            seed: 0x1234_5678_9abc_def0,
        }
    }

    #[test]
    fn ensemble_fits_real_outcomes_n_step_returns_and_held_out_splits() {
        let (shard, graph, _, dataset) = balanced_sources();
        let first = NativeGoalReachabilityModel::fit(
            std::slice::from_ref(&dataset),
            std::slice::from_ref(&shard),
            fit_config(),
        )
        .unwrap();
        first.validate().unwrap();
        let decoded: NativeGoalReachabilityModel =
            serde_json::from_slice(&serde_json::to_vec(&first).unwrap()).unwrap();
        assert_eq!(decoded.digest().unwrap(), decoded.model_sha256);
        decoded.validate().unwrap();
        assert_eq!(decoded.model_sha256, first.model_sha256);
        assert!(first.training_n_step_bootstrap_rows > 0);
        assert_eq!(
            first.admission,
            NativeGoalReachabilityAdmission::GoalConditionedCandidate,
            "{:?}",
            first.validation
        );
        assert!(first.validation.reachability_brier < first.validation.baseline_reachability_brier);
        assert!(
            first.validation.discounted_return_rmse
                < first.validation.baseline_discounted_return_rmse
        );
        assert!(
            first.validation.successful_time_mae_ticks
                < first.validation.baseline_successful_time_mae_ticks
        );
        assert!(
            first.validation.discounted_tick_cost_mae
                < first.validation.baseline_discounted_tick_cost_mae
        );
        let second = NativeGoalReachabilityModel::fit(
            std::slice::from_ref(&dataset),
            std::slice::from_ref(&shard),
            fit_config(),
        )
        .unwrap();
        assert_eq!(first, second);

        let goal = SemanticGoalInput::from_graph(&graph).unwrap();
        let success = &shard
            .episodes
            .iter()
            .find(|episode| episode.success)
            .unwrap()
            .steps[0]
            .pre_input;
        let failure = &shard
            .episodes
            .iter()
            .find(|episode| !episode.success)
            .unwrap()
            .steps[0]
            .pre_input;
        let success_estimate = first.estimate(success, &goal).unwrap();
        let failure_estimate = first.estimate(failure, &goal).unwrap();
        assert!(
            success_estimate.reachability_probability > failure_estimate.reachability_probability
        );
        assert!(
            success_estimate.discounted_terminal_return
                > failure_estimate.discounted_terminal_return
        );
    }

    #[test]
    fn semantic_embedding_excludes_provenance_and_split_leakage_fails_closed() {
        let single = graph(GOAL_SOURCE, "reach_goal");
        let multi = graph(
            r#"milestones 1.8
milestone unrelated {
  phase pre_input
  when stage.room == 9
}
milestone reach_goal {
  phase post_sim
  when stage.room == 1
}
"#,
            "reach_goal",
        );
        assert_ne!(single.program_sha256, multi.program_sha256);
        let single_input = SemanticGoalInput::from_graph(&single).unwrap();
        let multi_input = SemanticGoalInput::from_graph(&multi).unwrap();
        assert_ne!(single_input.input_sha256, multi_input.input_sha256);
        assert_eq!(
            goal_embedding(&single_input).unwrap(),
            goal_embedding(&multi_input).unwrap()
        );
        let changed = graph(
            r#"milestones 1.8
milestone reach_goal {
  phase post_sim
  when stage.room == 2
}
"#,
            "reach_goal",
        );
        assert_ne!(
            goal_embedding(&single_input).unwrap(),
            goal_embedding(&SemanticGoalInput::from_graph(&changed).unwrap()).unwrap()
        );

        let (shard, graph, corpus, dataset) = balanced_sources();
        let crossed = NativeGoalTrajectoryDataset::build(
            &corpus,
            std::slice::from_ref(&shard),
            &graph,
            NativeGoalTrajectoryConfig {
                split_seed: dataset.config.split_seed.wrapping_add(1),
                ..dataset.config
            },
        )
        .unwrap();
        assert!(
            NativeGoalReachabilityModel::fit(
                &[dataset, crossed],
                std::slice::from_ref(&shard),
                fit_config(),
            )
            .is_err()
        );
    }

    #[test]
    fn source_and_resealed_model_tampering_fail_closed() {
        let (shard, _, _, dataset) = balanced_sources();
        let model = NativeGoalReachabilityModel::fit(
            std::slice::from_ref(&dataset),
            std::slice::from_ref(&shard),
            fit_config(),
        )
        .unwrap();
        let mut detached = shard.clone();
        detached.episodes[0].steps[0].pre_input.state_identity[0] ^= 1;
        assert!(
            NativeGoalReachabilityModel::fit(
                std::slice::from_ref(&dataset),
                std::slice::from_ref(&detached),
                fit_config(),
            )
            .is_err()
        );

        let mut tampered = model;
        tampered.admission = NativeGoalReachabilityAdmission::RetainTrainingMeanBaseline;
        tampered.model_sha256 = tampered.digest().unwrap();
        assert!(tampered.validate().is_err());
    }
}
