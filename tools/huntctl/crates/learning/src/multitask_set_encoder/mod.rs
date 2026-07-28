//! Trainable shared complete-set encoder with masked auxiliary heads.
//!
//! One actor transform and state latent are updated by every supported target.
//! Missing targets are masked, target normalization is fitted on training rows
//! only, and held-out results are compared with training-mean predictors.

use crate::artifact::Digest;
use crate::gated_recurrent::{GatedRecurrent, GatedRecurrentStep};
use crate::history_critics::Reservoir;
use crate::native_actor_features::NativeActorFeatureView;
use crate::native_auxiliary_dataset::{
    AuxiliarySplit, NativeAuxiliaryDataset, NativeAuxiliaryExample,
};
use crate::native_episode_history::{
    EpisodeHistoryPad, EpisodeHistoryTransition, MAX_EPISODE_HISTORY_DEPTH,
    NativeEpisodeHistoryView,
};
use crate::trainable_set_encoder::{
    DeterministicRng, Dimensions, FeatureLayout, TrainableSetConfig, TrainableSetError,
    TypedSetNode, TypedSetSample, clip, dense_tanh, dot, initialized_weights, ordered_nodes,
    validate_sample_dimensions,
};
use dusklight_evidence::native_episode_shard::{
    NativeActorObservation, NativeAttentionCandidateObservation, NativeChannelStatus,
    NativeEpisode, NativeEpisodeShard, NativeLearningObservation,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

mod actor_nodes;
mod feature_schema;
mod model;
mod observations;
#[cfg(test)]
mod tests;

use actor_nodes::*;
use feature_schema::*;
pub use model::CompleteSetMultiTaskEncoder;
use observations::*;

pub const MULTITASK_SET_ENCODER_REPORT_SCHEMA_V12: &str =
    "dusklight-multitask-set-encoder-report/v12";
pub const SHUFFLED_AUXILIARY_CONTROL_SCHEMA_V1: &str = "dusklight-shuffled-auxiliary-control/v1";
const MAX_TARGETS: usize = 64;
const MAX_SAMPLES: usize = 100_000;
const MAX_HIDDEN_WIDTH: usize = 256;
const MAX_EPOCHS: usize = 2_048;
const MAX_PARAMETERS: usize = 16_000_000;
const ACTION_CONTEXT_WIDTH: usize = 24;
const LEARNED_ATTENTION_HEADS: usize = 4;
pub const DEFAULT_HISTORY_RECURRENT_WIDTH: usize = 16;
const MAX_HISTORY_RECURRENT_WIDTH: usize = 256;
const HISTORY_RESERVOIR_SEED: u64 = 0x4e41_5449_5645_4801;
struct TargetNormalization {
    mean: Vec<f64>,
    inverse_stddev: Vec<f64>,
    positive_weight: Vec<f64>,
    negative_weight: Vec<f64>,
    support: Vec<usize>,
}

#[derive(Clone, Debug)]
pub struct MultiTaskSetSample {
    pub input: TypedSetSample,
    pub post_input: TypedSetSample,
    pub history: Vec<MultiTaskHistoryStep>,
    pub action_context: Vec<f32>,
    pub targets: Vec<f32>,
    pub target_present: Vec<bool>,
}

#[derive(Clone, Debug)]
pub struct MultiTaskHistoryStep {
    pub transition_sha256: Digest,
    pub state: Arc<TypedSetSample>,
    pub action_context: Vec<f32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuxiliaryHeadConditioning {
    PreStateAndAction,
    PreAndPostState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuxiliaryHeadObjective {
    NormalizedRegression,
    ClassBalancedBernoulli,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MultiTaskSetPooling {
    MeanMax,
    MeanMaxLearnedAttention,
    MeanMaxTaskAttention,
}

impl MultiTaskSetPooling {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "mean-max" => Some(Self::MeanMax),
            "mean-max-learned-attention" => Some(Self::MeanMaxLearnedAttention),
            "mean-max-task-attention" => Some(Self::MeanMaxTaskAttention),
            _ => None,
        }
    }

    fn global_attention_heads(self) -> usize {
        match self {
            Self::MeanMax => 0,
            Self::MeanMaxLearnedAttention => LEARNED_ATTENTION_HEADS,
            Self::MeanMaxTaskAttention => 0,
        }
    }

    fn task_attention_heads(self, target_count: usize) -> usize {
        match self {
            Self::MeanMaxTaskAttention => target_count,
            Self::MeanMax | Self::MeanMaxLearnedAttention => 0,
        }
    }

    fn attention_heads(self, target_count: usize) -> usize {
        self.global_attention_heads() + self.task_attention_heads(target_count)
    }

    fn uses_task_attention(self) -> bool {
        self == Self::MeanMaxTaskAttention
    }
}

#[derive(Clone, Debug)]
pub struct NativeMultiTaskActorCorpus {
    pub actor_feature_schema_sha256: Digest,
    pub feature_spec: NativeEncoderFeatureSpec,
    pub target_names: Vec<String>,
    pub training_dataset_sha256: Digest,
    pub validation_dataset_sha256: Digest,
    pub test_dataset_sha256: Digest,
    pub training: Vec<MultiTaskSetSample>,
    pub validation: Vec<MultiTaskSetSample>,
    pub test: Vec<MultiTaskSetSample>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeEncoderChannelFamily {
    CorePlayerMotion,
    CoreActionPhase,
    CoreEventContext,
    CoreEventTransition,
    CoreClockDomains,
    CoreWarpSession,
    CorePreviousInput,
    CoreCameraCollisionWorld,
    CoreRng,
    CoreGoal,
    CoreAttentionCandidates,
    CoreTemporalDelta,
    ActorPopulation,
    ActorTemporalDelta,
    ActorIdentity,
    ActorMotion,
    ActorLifecyclePhysics,
    ActorLinkRelative,
    ActorParentRelative,
    ActorAttention,
    ActorAttentionCandidates,
    ActorEventParticipation,
    ActorReturnWriter,
    ActorEnemyBase,
    ActorTriggerVolume,
    ActorDoor20,
    ActorPlayerRelationships,
}

impl NativeEncoderChannelFamily {
    pub const ALL: [Self; 27] = [
        Self::CorePlayerMotion,
        Self::CoreActionPhase,
        Self::CoreEventContext,
        Self::CoreEventTransition,
        Self::CoreClockDomains,
        Self::CoreWarpSession,
        Self::CorePreviousInput,
        Self::CoreCameraCollisionWorld,
        Self::CoreRng,
        Self::CoreGoal,
        Self::CoreAttentionCandidates,
        Self::CoreTemporalDelta,
        Self::ActorPopulation,
        Self::ActorTemporalDelta,
        Self::ActorIdentity,
        Self::ActorMotion,
        Self::ActorLifecyclePhysics,
        Self::ActorLinkRelative,
        Self::ActorParentRelative,
        Self::ActorAttention,
        Self::ActorAttentionCandidates,
        Self::ActorEventParticipation,
        Self::ActorReturnWriter,
        Self::ActorEnemyBase,
        Self::ActorTriggerVolume,
        Self::ActorDoor20,
        Self::ActorPlayerRelationships,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::CorePlayerMotion => "core_player_motion",
            Self::CoreActionPhase => "core_action_phase",
            Self::CoreEventContext => "core_event_context",
            Self::CoreEventTransition => "core_event_transition",
            Self::CoreClockDomains => "core_clock_domains",
            Self::CoreWarpSession => "core_warp_session",
            Self::CorePreviousInput => "core_previous_input",
            Self::CoreCameraCollisionWorld => "core_camera_collision_world",
            Self::CoreRng => "core_rng",
            Self::CoreGoal => "core_goal",
            Self::CoreAttentionCandidates => "core_attention_candidates",
            Self::CoreTemporalDelta => "core_temporal_delta",
            Self::ActorPopulation => "actor_population",
            Self::ActorTemporalDelta => "actor_temporal_delta",
            Self::ActorIdentity => "actor_identity",
            Self::ActorMotion => "actor_motion",
            Self::ActorLifecyclePhysics => "actor_lifecycle_physics",
            Self::ActorLinkRelative => "actor_link_relative",
            Self::ActorParentRelative => "actor_parent_relative",
            Self::ActorAttention => "actor_attention",
            Self::ActorAttentionCandidates => "actor_attention_candidates",
            Self::ActorEventParticipation => "actor_event_participation",
            Self::ActorReturnWriter => "actor_return_writer",
            Self::ActorEnemyBase => "actor_enemy_base",
            Self::ActorTriggerVolume => "actor_trigger_volume",
            Self::ActorDoor20 => "actor_door20",
            Self::ActorPlayerRelationships => "actor_player_relationships",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|family| family.name() == name)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeEncoderFeatureSpec {
    pub families: Vec<NativeEncoderChannelFamily>,
    pub history_depth: usize,
    pub history_encoding: NativeEncoderHistoryEncoding,
    pub history_recurrent_width: usize,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeEncoderHistoryEncoding {
    None,
    Stacked,
    RecurrentReservoir,
    TrainableGru,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MultiTaskTemporalConfig {
    pub encoding: MultiTaskTemporalEncoding,
    pub history_depth: usize,
    pub hidden_width: usize,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MultiTaskTemporalEncoding {
    None,
    GatedRecurrent,
}

impl MultiTaskTemporalConfig {
    pub fn none() -> Self {
        Self {
            encoding: MultiTaskTemporalEncoding::None,
            history_depth: 0,
            hidden_width: 0,
        }
    }

    pub fn gated_recurrent(history_depth: usize, hidden_width: usize) -> Self {
        Self {
            encoding: MultiTaskTemporalEncoding::GatedRecurrent,
            history_depth,
            hidden_width,
        }
    }

    fn validate(self) -> Result<(), TrainableSetError> {
        if !matches!(
            (self.encoding, self.history_depth, self.hidden_width),
            (MultiTaskTemporalEncoding::None, 0, 0)
                | (
                    MultiTaskTemporalEncoding::GatedRecurrent,
                    1..=MAX_EPISODE_HISTORY_DEPTH,
                    1..=MAX_HISTORY_RECURRENT_WIDTH
                )
        ) {
            return Err(TrainableSetError::new(
                "multitask temporal configuration is invalid",
            ));
        }
        Ok(())
    }
}

impl NativeEncoderFeatureSpec {
    pub fn all() -> Self {
        Self {
            families: NativeEncoderChannelFamily::ALL.into(),
            history_depth: 0,
            history_encoding: NativeEncoderHistoryEncoding::None,
            history_recurrent_width: 0,
        }
    }

    pub fn excluding(
        excluded: impl IntoIterator<Item = NativeEncoderChannelFamily>,
    ) -> Result<Self, TrainableSetError> {
        let excluded = excluded.into_iter().collect::<BTreeSet<_>>();
        Self::new(
            NativeEncoderChannelFamily::ALL
                .into_iter()
                .filter(|family| !excluded.contains(family)),
        )
    }

    pub fn new(
        families: impl IntoIterator<Item = NativeEncoderChannelFamily>,
    ) -> Result<Self, TrainableSetError> {
        let families = families.into_iter().collect::<BTreeSet<_>>();
        let spec = Self {
            families: families.into_iter().collect(),
            history_depth: 0,
            history_encoding: NativeEncoderHistoryEncoding::None,
            history_recurrent_width: 0,
        };
        spec.validate()?;
        Ok(spec)
    }

    pub fn validate(&self) -> Result<(), TrainableSetError> {
        if self.families.is_empty()
            || self.families.windows(2).any(|pair| pair[0] >= pair[1])
            || self.history_depth > MAX_EPISODE_HISTORY_DEPTH
            || !matches!(
                (
                    self.history_depth,
                    self.history_encoding,
                    self.history_recurrent_width
                ),
                (0, NativeEncoderHistoryEncoding::None, 0)
                    | (1.., NativeEncoderHistoryEncoding::Stacked, 0)
                    | (
                        1..,
                        NativeEncoderHistoryEncoding::RecurrentReservoir,
                        1..=MAX_HISTORY_RECURRENT_WIDTH
                    )
                    | (
                        1..,
                        NativeEncoderHistoryEncoding::TrainableGru,
                        1..=MAX_HISTORY_RECURRENT_WIDTH
                    )
            )
        {
            return Err(TrainableSetError::new(
                "native encoder feature spec must be nonempty, unique, canonical, and bounded",
            ));
        }
        let has_actor_features = self
            .families
            .iter()
            .any(|family| actor_column_family(*family));
        if has_actor_features
            && !self
                .families
                .contains(&NativeEncoderChannelFamily::ActorPopulation)
        {
            return Err(TrainableSetError::new(
                "native encoder actor columns require the actor_population family",
            ));
        }
        Ok(())
    }

    pub fn contains(&self, family: NativeEncoderChannelFamily) -> bool {
        self.families.binary_search(&family).is_ok()
    }

    pub fn with_history_depth(mut self, history_depth: usize) -> Result<Self, TrainableSetError> {
        self.history_depth = history_depth;
        self.history_encoding = if history_depth == 0 {
            NativeEncoderHistoryEncoding::None
        } else {
            NativeEncoderHistoryEncoding::Stacked
        };
        self.history_recurrent_width = 0;
        self.validate()?;
        Ok(self)
    }

    pub fn with_recurrent_history(
        mut self,
        history_depth: usize,
        history_recurrent_width: usize,
    ) -> Result<Self, TrainableSetError> {
        self.history_depth = history_depth;
        self.history_encoding = NativeEncoderHistoryEncoding::RecurrentReservoir;
        self.history_recurrent_width = history_recurrent_width;
        self.validate()?;
        Ok(self)
    }

    pub fn with_trainable_history(
        mut self,
        history_depth: usize,
        history_hidden_width: usize,
    ) -> Result<Self, TrainableSetError> {
        self.history_depth = history_depth;
        self.history_encoding = NativeEncoderHistoryEncoding::TrainableGru;
        self.history_recurrent_width = history_hidden_width;
        self.validate()?;
        Ok(self)
    }

    pub fn temporal_config(&self) -> MultiTaskTemporalConfig {
        if self.history_encoding == NativeEncoderHistoryEncoding::TrainableGru {
            MultiTaskTemporalConfig::gated_recurrent(
                self.history_depth,
                self.history_recurrent_width,
            )
        } else {
            MultiTaskTemporalConfig::none()
        }
    }
}

fn actor_column_family(family: NativeEncoderChannelFamily) -> bool {
    matches!(
        family,
        NativeEncoderChannelFamily::ActorTemporalDelta
            | NativeEncoderChannelFamily::ActorIdentity
            | NativeEncoderChannelFamily::ActorMotion
            | NativeEncoderChannelFamily::ActorLifecyclePhysics
            | NativeEncoderChannelFamily::ActorLinkRelative
            | NativeEncoderChannelFamily::ActorParentRelative
            | NativeEncoderChannelFamily::ActorAttention
            | NativeEncoderChannelFamily::ActorEventParticipation
            | NativeEncoderChannelFamily::ActorReturnWriter
            | NativeEncoderChannelFamily::ActorEnemyBase
            | NativeEncoderChannelFamily::ActorTriggerVolume
            | NativeEncoderChannelFamily::ActorDoor20
            | NativeEncoderChannelFamily::ActorPlayerRelationships
    )
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ShuffledAuxiliaryControl {
    pub schema: &'static str,
    pub shuffled_training_dataset_sha256: Digest,
    pub report: MultiTaskSetEncoderReport,
    pub test_evaluation: MultiTaskSetEvaluation,
}

impl NativeMultiTaskActorCorpus {
    pub fn build(
        dataset: &NativeAuxiliaryDataset,
        shard: &NativeEpisodeShard,
    ) -> Result<Self, TrainableSetError> {
        Self::build_with_spec(dataset, shard, NativeEncoderFeatureSpec::all())
    }

    pub fn build_with_spec(
        dataset: &NativeAuxiliaryDataset,
        shard: &NativeEpisodeShard,
        feature_spec: NativeEncoderFeatureSpec,
    ) -> Result<Self, TrainableSetError> {
        feature_spec.validate()?;
        dataset
            .validate()
            .map_err(|error| TrainableSetError::new(error.to_string()))?;
        if dataset.observation_schema != shard.metadata.observation_schema
            || dataset.action_schema != shard.metadata.action_schema
            || dataset
                .examples
                .iter()
                .any(|example| example.shard_sha256 != shard.content_sha256)
        {
            return Err(TrainableSetError::new(
                "native multitask sources are detached or span unsupported shards",
            ));
        }
        let actor_feature_schema_sha256 = native_actor_feature_schema(&feature_spec)?;
        let episodes = shard
            .episodes
            .iter()
            .map(|episode| (episode.id.as_str(), episode))
            .collect::<BTreeMap<_, _>>();
        let mut episode_offsets = BTreeMap::new();
        let mut episode_offset = 0_usize;
        for episode in &shard.episodes {
            episode_offsets.insert(episode.id.as_str(), episode_offset);
            episode_offset = episode_offset
                .checked_add(episode.steps.len())
                .ok_or_else(|| TrainableSetError::new("native history offset overflowed"))?;
        }
        let history = (feature_spec.history_depth > 0)
            .then(|| NativeEpisodeHistoryView::build(shard, feature_spec.history_depth))
            .transpose()
            .map_err(|error| TrainableSetError::new(error.to_string()))?;
        let history_reservoir = native_recurrent_history_reservoir(&feature_spec)?;
        let target_names = native_target_names();
        debug_assert_eq!(
            target_conditioning_for_names(&target_names),
            native_target_conditioning()
        );
        let mut training = Vec::new();
        let mut validation = Vec::new();
        let mut test = Vec::new();
        let mut trainable_history_states = BTreeMap::new();
        for example in &dataset.examples {
            let episode = episodes.get(example.episode_id.as_str()).ok_or_else(|| {
                TrainableSetError::new("native multitask episode is absent from shard")
            })?;
            let step = episode
                .steps
                .get(example.step_index as usize)
                .ok_or_else(|| {
                    TrainableSetError::new("native multitask step is absent from episode")
                })?;
            let previous_pre_input = example
                .step_index
                .checked_sub(1)
                .and_then(|index| episode.steps.get(index as usize))
                .map(|step| &step.pre_input);
            let completed_history = if let Some(history) = &history {
                let decision_index = episode_offsets
                    .get(example.episode_id.as_str())
                    .and_then(|offset| offset.checked_add(example.step_index as usize))
                    .ok_or_else(|| {
                        TrainableSetError::new("native history decision index overflowed")
                    })?;
                let decision = history
                    .decisions
                    .get(decision_index)
                    .ok_or_else(|| TrainableSetError::new("native history decision is absent"))?;
                if decision.episode_id != example.episode_id
                    || decision.step_index != example.step_index
                {
                    return Err(TrainableSetError::new(
                        "native history decision is detached from auxiliary example",
                    ));
                }
                decision
                    .completed_transition_indices
                    .iter()
                    .map(|index| {
                        history.transitions.get(*index as usize).ok_or_else(|| {
                            TrainableSetError::new("native history transition is absent")
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?
            } else {
                Vec::new()
            };
            if hex_128(step.pre_input.state_identity) != example.pre_input_state_xxh3_128
                || hex_128(step.post_simulation.state_identity)
                    != example.post_simulation_state_xxh3_128
            {
                return Err(TrainableSetError::new(
                    "native multitask pre/post state identity is detached",
                ));
            }
            if step.pre_input.actors_truncated || step.post_simulation.actors_truncated {
                return Err(TrainableSetError::new(
                    "native multitask pre/post actor observations must be complete",
                ));
            }
            let sample_history = trainable_episode_history_steps(
                episode,
                &completed_history,
                &feature_spec,
                actor_feature_schema_sha256,
                &mut trainable_history_states,
            )?;
            let (mut base, mut base_present) = broad_base(&step.pre_input);
            append_core_temporal_features(
                &mut base,
                &mut base_present,
                &step.pre_input,
                previous_pre_input,
            );
            retain_feature_families(
                &mut base,
                &mut base_present,
                &native_base_feature_families(),
                &feature_spec,
            );
            append_encoded_episode_history_features(
                &mut base,
                &mut base_present,
                episode,
                &completed_history,
                &feature_spec,
                history_reservoir.as_ref(),
            )?;
            let (targets, target_present) = native_targets(example);
            let mut nodes = if feature_spec.contains(NativeEncoderChannelFamily::ActorPopulation) {
                native_actor_nodes(&step.pre_input, previous_pre_input)
            } else {
                Vec::new()
            };
            for node in &mut nodes {
                retain_node_feature_families(node, &feature_spec);
            }
            let (mut post_base, mut post_base_present) = broad_base(&step.post_simulation);
            append_core_temporal_features(
                &mut post_base,
                &mut post_base_present,
                &step.post_simulation,
                Some(&step.pre_input),
            );
            suppress_base_family(
                &mut post_base,
                &mut post_base_present,
                NativeEncoderChannelFamily::CorePreviousInput,
            );
            retain_feature_families(
                &mut post_base,
                &mut post_base_present,
                &native_base_feature_families(),
                &feature_spec,
            );
            append_encoded_episode_history_features(
                &mut post_base,
                &mut post_base_present,
                episode,
                &completed_history,
                &feature_spec,
                history_reservoir.as_ref(),
            )?;
            let mut post_nodes =
                if feature_spec.contains(NativeEncoderChannelFamily::ActorPopulation) {
                    native_actor_nodes(&step.post_simulation, Some(&step.pre_input))
                } else {
                    Vec::new()
                };
            for node in &mut post_nodes {
                retain_node_feature_families(node, &feature_spec);
            }
            let post_sample_sha256 = canonical_digest(
                b"dusklight.native-multitask-post-input/v1\0",
                &(
                    example.example_sha256,
                    &example.post_simulation_state_xxh3_128,
                ),
            )?;
            let sample = MultiTaskSetSample {
                input: TypedSetSample {
                    sample_sha256: example.example_sha256,
                    actor_feature_schema_sha256,
                    base,
                    base_present,
                    nodes,
                    target: 0.0,
                },
                post_input: TypedSetSample {
                    sample_sha256: post_sample_sha256,
                    actor_feature_schema_sha256,
                    base: post_base,
                    base_present: post_base_present,
                    nodes: post_nodes,
                    target: 0.0,
                },
                history: sample_history,
                action_context: native_action_context(example),
                targets,
                target_present,
            };
            match example.split {
                AuxiliarySplit::Training => training.push(sample),
                AuxiliarySplit::Validation => validation.push(sample),
                AuxiliarySplit::Test => test.push(sample),
            }
        }
        if training.is_empty() || validation.is_empty() || test.is_empty() {
            return Err(TrainableSetError::new(
                "native multitask corpus requires all three episode splits",
            ));
        }
        Ok(Self {
            actor_feature_schema_sha256,
            feature_spec,
            target_names,
            training_dataset_sha256: sample_manifest_digest(&training)?,
            validation_dataset_sha256: sample_manifest_digest(&validation)?,
            test_dataset_sha256: sample_manifest_digest(&test)?,
            training,
            validation,
            test,
        })
    }
}

impl MultiTaskSetSample {
    #[allow(clippy::too_many_arguments)]
    pub fn from_native_actor_transition(
        view: &NativeActorFeatureView,
        pre_observation_index: usize,
        post_observation_index: usize,
        pre_sample_sha256: Digest,
        post_sample_sha256: Digest,
        pre_base: Vec<f32>,
        pre_base_present: Vec<bool>,
        post_base: Vec<f32>,
        post_base_present: Vec<bool>,
        action_context: Vec<f32>,
        targets: Vec<f32>,
        target_present: Vec<bool>,
    ) -> Result<Self, TrainableSetError> {
        if action_context.len() != ACTION_CONTEXT_WIDTH
            || action_context.iter().any(|value| !value.is_finite())
        {
            return Err(TrainableSetError::new(
                "native actor transition action context is invalid",
            ));
        }
        Ok(Self {
            input: TypedSetSample::from_native_actor_observation(
                view,
                pre_observation_index,
                pre_sample_sha256,
                pre_base,
                pre_base_present,
                0.0,
            )?,
            post_input: TypedSetSample::from_native_actor_observation(
                view,
                post_observation_index,
                post_sample_sha256,
                post_base,
                post_base_present,
                0.0,
            )?,
            history: Vec::new(),
            action_context,
            targets,
            target_present,
        })
    }
}

pub fn fit_shuffled_auxiliary_control(
    actor_feature_schema_sha256: Digest,
    target_names: Vec<String>,
    training: Vec<MultiTaskSetSample>,
    validation_dataset_sha256: Digest,
    validation: &[MultiTaskSetSample],
    test: &[MultiTaskSetSample],
    config: TrainableSetConfig,
) -> Result<ShuffledAuxiliaryControl, TrainableSetError> {
    fit_shuffled_auxiliary_control_with_pooling(
        actor_feature_schema_sha256,
        target_names,
        training,
        validation_dataset_sha256,
        validation,
        test,
        config,
        MultiTaskSetPooling::MeanMax,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn fit_shuffled_auxiliary_control_with_pooling(
    actor_feature_schema_sha256: Digest,
    target_names: Vec<String>,
    training: Vec<MultiTaskSetSample>,
    validation_dataset_sha256: Digest,
    validation: &[MultiTaskSetSample],
    test: &[MultiTaskSetSample],
    config: TrainableSetConfig,
    pooling: MultiTaskSetPooling,
) -> Result<ShuffledAuxiliaryControl, TrainableSetError> {
    fit_shuffled_auxiliary_control_with_pooling_and_temporal(
        actor_feature_schema_sha256,
        target_names,
        training,
        validation_dataset_sha256,
        validation,
        test,
        config,
        pooling,
        MultiTaskTemporalConfig::none(),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn fit_shuffled_auxiliary_control_with_pooling_and_temporal(
    actor_feature_schema_sha256: Digest,
    target_names: Vec<String>,
    mut training: Vec<MultiTaskSetSample>,
    validation_dataset_sha256: Digest,
    validation: &[MultiTaskSetSample],
    test: &[MultiTaskSetSample],
    config: TrainableSetConfig,
    pooling: MultiTaskSetPooling,
    temporal: MultiTaskTemporalConfig,
) -> Result<ShuffledAuxiliaryControl, TrainableSetError> {
    if training.is_empty() || target_names.is_empty() {
        return Err(TrainableSetError::new(
            "shuffled auxiliary control requires training rows and targets",
        ));
    }
    let mut rng = DeterministicRng::new(config.seed ^ 0x5a11_f1ed_c017_0001);
    for target in 0..target_names.len() {
        let rows = training
            .iter()
            .enumerate()
            .filter_map(|(row, sample)| sample.target_present[target].then_some(row))
            .collect::<Vec<_>>();
        let mut shuffled_rows = rows.clone();
        rng.shuffle(&mut shuffled_rows);
        let values = shuffled_rows
            .iter()
            .map(|row| training[*row].targets[target])
            .collect::<Vec<_>>();
        for (row, value) in rows.into_iter().zip(values) {
            training[row].targets[target] = value;
        }
    }
    let shuffled_training_dataset_sha256 = sample_manifest_digest(&training)?;
    let (report, model) = CompleteSetMultiTaskEncoder::fit_with_pooling_and_temporal(
        actor_feature_schema_sha256,
        shuffled_training_dataset_sha256,
        validation_dataset_sha256,
        target_names,
        &training,
        validation,
        config,
        pooling,
        temporal,
    )?;
    let test_evaluation = model.evaluate(test)?;
    Ok(ShuffledAuxiliaryControl {
        schema: SHUFFLED_AUXILIARY_CONTROL_SCHEMA_V1,
        shuffled_training_dataset_sha256,
        report,
        test_evaluation,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MultiTaskEncoderDecision {
    RetainTrainingMeanBaseline,
    SharedEncoderCandidate,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AuxiliaryHeadMetrics {
    pub name: String,
    pub objective: AuxiliaryHeadObjective,
    pub training_support: usize,
    pub held_out_support: usize,
    pub training_loss: f64,
    pub held_out_loss: f64,
    pub held_out_constant_baseline_loss: f64,
    pub relative_held_out_improvement: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AuxiliaryHeadEvaluation {
    pub name: String,
    pub objective: AuxiliaryHeadObjective,
    pub support: usize,
    pub loss: f64,
    pub constant_baseline_loss: f64,
    pub relative_improvement: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MultiTaskSetEvaluation {
    pub samples: usize,
    pub objective_loss: f64,
    pub constant_baseline_objective_loss: f64,
    pub relative_improvement: f64,
    pub heads: Vec<AuxiliaryHeadEvaluation>,
    pub rare_events: Vec<RareEventMetrics>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BinaryEventMetrics {
    pub positives: usize,
    pub negatives: usize,
    pub true_positives: usize,
    pub false_positives: usize,
    pub true_negatives: usize,
    pub false_negatives: usize,
    pub precision: Option<f64>,
    pub recall: Option<f64>,
    pub specificity: Option<f64>,
    pub balanced_accuracy: Option<f64>,
    pub f1: Option<f64>,
    pub brier_score: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RareEventMetrics {
    pub name: String,
    pub threshold: f64,
    pub model: BinaryEventMetrics,
    pub training_mean_baseline: BinaryEventMetrics,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AttentionHeadDiagnostics {
    pub head: usize,
    pub target: Option<String>,
    pub conditioning: Option<AuxiliaryHeadConditioning>,
    pub observation_support: usize,
    pub query_l2_norm: f64,
    pub mean_normalized_entropy: f64,
    pub mean_maximum_weight: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MultiTaskSetEncoderReport {
    pub schema: &'static str,
    pub actor_feature_schema_sha256: Digest,
    pub training_dataset_sha256: Digest,
    pub held_out_dataset_sha256: Digest,
    pub config: TrainableSetConfig,
    pub pooling: MultiTaskSetPooling,
    pub temporal: MultiTaskTemporalConfig,
    pub target_names: Vec<String>,
    pub target_conditioning: Vec<AuxiliaryHeadConditioning>,
    pub target_objectives: Vec<AuxiliaryHeadObjective>,
    pub target_positive_weights: Vec<f64>,
    pub target_negative_weights: Vec<f64>,
    pub target_decision_thresholds: Vec<Option<f64>>,
    pub target_support_training: Vec<usize>,
    pub target_support_held_out: Vec<usize>,
    pub maximum_training_nodes: usize,
    pub maximum_held_out_nodes: usize,
    pub parameter_count: usize,
    pub optimizer_steps: u64,
    pub training_objective_loss: f64,
    pub held_out_objective_loss: f64,
    pub held_out_constant_baseline_objective_loss: f64,
    pub relative_held_out_improvement: f64,
    pub heads: Vec<AuxiliaryHeadMetrics>,
    pub held_out_rare_events: Vec<RareEventMetrics>,
    pub held_out_attention: Vec<AttentionHeadDiagnostics>,
    pub decision: MultiTaskEncoderDecision,
    pub model_sha256: Digest,
    pub promotion_authority: bool,
    pub report_sha256: Digest,
}

fn accumulate_attention_distribution(
    weights: &[f64],
    entropy_sum: &mut f64,
    maximum_sum: &mut f64,
    support: &mut usize,
) {
    if weights.is_empty() {
        return;
    }
    let entropy = -weights
        .iter()
        .filter(|weight| **weight > 0.0)
        .map(|weight| weight * weight.ln())
        .sum::<f64>();
    let maximum_entropy = (weights.len() as f64).ln();
    *entropy_sum += if maximum_entropy > 0.0 {
        entropy / maximum_entropy
    } else {
        0.0
    };
    *maximum_sum += weights
        .iter()
        .copied()
        .max_by(f64::total_cmp)
        .unwrap_or(0.0);
    *support += 1;
}

#[derive(Clone, Default)]
struct BinaryEventAccumulator {
    positives: usize,
    negatives: usize,
    true_positives: usize,
    false_positives: usize,
    true_negatives: usize,
    false_negatives: usize,
    brier_sum: f64,
}

impl BinaryEventAccumulator {
    fn observe(&mut self, expected: bool, score: f64, threshold: f64) {
        let probability = score.clamp(0.0, 1.0);
        let predicted = probability >= threshold;
        self.brier_sum += (probability - f64::from(expected)).powi(2);
        match (expected, predicted) {
            (true, true) => self.true_positives += 1,
            (true, false) => self.false_negatives += 1,
            (false, true) => self.false_positives += 1,
            (false, false) => self.true_negatives += 1,
        }
        if expected {
            self.positives += 1;
        } else {
            self.negatives += 1;
        }
    }

    fn finish(&self) -> Result<BinaryEventMetrics, TrainableSetError> {
        let total = self.positives + self.negatives;
        if total == 0 {
            return Err(TrainableSetError::new(
                "rare-event metric has no supported examples",
            ));
        }
        let precision = ratio(
            self.true_positives,
            self.true_positives + self.false_positives,
        );
        let recall = ratio(self.true_positives, self.positives);
        let specificity = ratio(self.true_negatives, self.negatives);
        let balanced_accuracy = recall
            .zip(specificity)
            .map(|(recall, specificity)| (recall + specificity) / 2.0);
        let f1 = precision.zip(recall).map(|(precision, recall)| {
            if precision + recall > 0.0 {
                2.0 * precision * recall / (precision + recall)
            } else {
                0.0
            }
        });
        Ok(BinaryEventMetrics {
            positives: self.positives,
            negatives: self.negatives,
            true_positives: self.true_positives,
            false_positives: self.false_positives,
            true_negatives: self.true_negatives,
            false_negatives: self.false_negatives,
            precision,
            recall,
            specificity,
            balanced_accuracy,
            f1,
            brier_score: self.brier_sum / total as f64,
        })
    }
}

fn ratio(numerator: usize, denominator: usize) -> Option<f64> {
    (denominator != 0).then_some(numerator as f64 / denominator as f64)
}

fn rare_event_target(name: &str) -> bool {
    matches!(
        name,
        "contact_changed"
            | "procedure_changed"
            | "mode_flags_changed"
            | "actor_disappearance_occurred"
    )
}

fn sample_manifest_digest(samples: &[MultiTaskSetSample]) -> Result<Digest, TrainableSetError> {
    canonical_digest(
        b"dusklight.native-multitask-sample-dataset/v5\0",
        &samples
            .iter()
            .map(|sample| {
                (
                    sample.input.sample_sha256,
                    sample.post_input.sample_sha256,
                    sample
                        .history
                        .iter()
                        .map(|step| {
                            (
                                step.transition_sha256,
                                step.state.sample_sha256,
                                &step.action_context,
                            )
                        })
                        .collect::<Vec<_>>(),
                    &sample.action_context,
                    &sample.targets,
                    &sample.target_present,
                )
            })
            .collect::<Vec<_>>(),
    )
}

fn sample_model_states(sample: &MultiTaskSetSample) -> impl Iterator<Item = &TypedSetSample> {
    std::iter::once(&sample.input)
        .chain(std::iter::once(&sample.post_input))
        .chain(sample.history.iter().map(|step| step.state.as_ref()))
}

fn hex_128(value: [u8; 16]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[allow(clippy::too_many_arguments)]
fn validate_samples(
    actor_feature_schema_sha256: Digest,
    training_dataset_sha256: Digest,
    held_out_dataset_sha256: Digest,
    target_names: &[String],
    training: &[MultiTaskSetSample],
    held_out: &[MultiTaskSetSample],
    config: TrainableSetConfig,
    temporal: MultiTaskTemporalConfig,
) -> Result<Dimensions, TrainableSetError> {
    temporal.validate()?;
    if actor_feature_schema_sha256 == Digest::ZERO
        || training_dataset_sha256 == Digest::ZERO
        || held_out_dataset_sha256 == Digest::ZERO
        || training_dataset_sha256 == held_out_dataset_sha256
        || training.is_empty()
        || held_out.is_empty()
        || training.len() > MAX_SAMPLES
        || held_out.len() > MAX_SAMPLES
        || target_names.is_empty()
        || target_names.len() > MAX_TARGETS
        || target_names.iter().any(|name| name.is_empty())
        || target_names.iter().collect::<BTreeSet<_>>().len() != target_names.len()
        || config.epochs == 0
        || config.epochs > MAX_EPOCHS
        || config.node_hidden_width == 0
        || config.node_hidden_width > MAX_HIDDEN_WIDTH
        || config.head_hidden_width == 0
        || config.head_hidden_width > MAX_HIDDEN_WIDTH
        || !config.learning_rate.is_finite()
        || config.learning_rate <= 0.0
        || !config.l2_penalty.is_finite()
        || config.l2_penalty < 0.0
        || !config.gradient_clip.is_finite()
        || config.gradient_clip <= 0.0
        || !config.minimum_relative_improvement.is_finite()
        || !(0.0..=1.0).contains(&config.minimum_relative_improvement)
    {
        return Err(TrainableSetError::new(
            "multitask set encoder configuration is invalid",
        ));
    }
    let first_node = training
        .iter()
        .chain(held_out)
        .flat_map(sample_model_states)
        .find_map(|input| input.nodes.first());
    let dimensions = Dimensions {
        categorical: first_node.map_or(0, |node| node.categorical.len()),
        continuous: first_node.map_or(0, |node| node.continuous.len()),
        binary: first_node.map_or(0, |node| node.binary.len()),
        base: training[0].input.base.len(),
    };
    let target_objectives = target_objectives_for_names(target_names);
    let mut identities = BTreeSet::new();
    let mut history_steps = 0_usize;
    for sample in training.iter().chain(held_out) {
        if sample.input.sample_sha256 == Digest::ZERO
            || !identities.insert(sample.input.sample_sha256)
            || sample.post_input.sample_sha256 == Digest::ZERO
            || sample.post_input.sample_sha256 == sample.input.sample_sha256
            || !identities.insert(sample.post_input.sample_sha256)
            || sample.input.actor_feature_schema_sha256 != actor_feature_schema_sha256
            || sample.post_input.actor_feature_schema_sha256 != actor_feature_schema_sha256
            || sample.action_context.len() != ACTION_CONTEXT_WIDTH
            || sample.action_context.iter().any(|value| !value.is_finite())
            || sample.targets.len() != target_names.len()
            || sample.target_present.len() != target_names.len()
            || sample
                .targets
                .iter()
                .zip(&sample.target_present)
                .any(|(target, present)| !target.is_finite() || (!present && *target != 0.0))
            || sample.targets.iter().enumerate().any(|(target, value)| {
                sample.target_present[target]
                    && target_objectives[target] == AuxiliaryHeadObjective::ClassBalancedBernoulli
                    && *value != 0.0
                    && *value != 1.0
            })
        {
            return Err(TrainableSetError::new(
                "multitask sample identity, schema, target, or mask is invalid",
            ));
        }
        let history_valid = match temporal.encoding {
            MultiTaskTemporalEncoding::None => sample.history.is_empty(),
            MultiTaskTemporalEncoding::GatedRecurrent => {
                sample.history.len() <= temporal.history_depth
            }
        };
        let mut transition_identities = BTreeSet::new();
        if !history_valid {
            return Err(TrainableSetError::new(
                "multitask sample history does not match temporal configuration",
            ));
        }
        for step in &sample.history {
            if step.transition_sha256 == Digest::ZERO
                || !transition_identities.insert(step.transition_sha256)
                || step.state.sample_sha256 == Digest::ZERO
                || step.state.actor_feature_schema_sha256 != actor_feature_schema_sha256
                || step.action_context.len() != ACTION_CONTEXT_WIDTH
                || step.action_context.iter().any(|value| !value.is_finite())
            {
                return Err(TrainableSetError::new(
                    "multitask sample history identity, schema, or action is invalid",
                ));
            }
            history_steps += 1;
        }
        for state in sample_model_states(sample) {
            validate_sample_dimensions(state, dimensions)?;
        }
    }
    if temporal.encoding == MultiTaskTemporalEncoding::GatedRecurrent && history_steps == 0 {
        return Err(TrainableSetError::new(
            "multitask recurrent corpus contains no history",
        ));
    }
    if target_support(training, target_names.len()).contains(&0) {
        return Err(TrainableSetError::new(
            "each auxiliary target requires training support",
        ));
    }
    Ok(dimensions)
}

fn target_support(samples: &[MultiTaskSetSample], width: usize) -> Vec<usize> {
    (0..width)
        .map(|target| {
            samples
                .iter()
                .filter(|sample| sample.target_present[target])
                .count()
        })
        .collect()
}

fn target_normalization(
    training: &[MultiTaskSetSample],
    objectives: &[AuxiliaryHeadObjective],
) -> Result<TargetNormalization, TrainableSetError> {
    let width = objectives.len();
    let support = target_support(training, width);
    let mut mean = Vec::with_capacity(width);
    let mut inverse_stddev = Vec::with_capacity(width);
    let mut positive_weight = Vec::with_capacity(width);
    let mut negative_weight = Vec::with_capacity(width);
    for (target, objective) in objectives.iter().copied().enumerate() {
        let values = training
            .iter()
            .filter(|sample| sample.target_present[target])
            .map(|sample| f64::from(sample.targets[target]))
            .collect::<Vec<_>>();
        let target_mean = values.iter().sum::<f64>() / values.len() as f64;
        let variance = values
            .iter()
            .map(|value| (value - target_mean).powi(2))
            .sum::<f64>()
            / values.len() as f64;
        mean.push(target_mean);
        match objective {
            AuxiliaryHeadObjective::NormalizedRegression => {
                inverse_stddev.push(if variance > 1.0e-12 {
                    1.0 / variance.sqrt()
                } else {
                    1.0
                });
                positive_weight.push(1.0);
                negative_weight.push(1.0);
            }
            AuxiliaryHeadObjective::ClassBalancedBernoulli => {
                if values.iter().any(|value| *value != 0.0 && *value != 1.0) {
                    return Err(TrainableSetError::new(
                        "Bernoulli auxiliary target is not binary",
                    ));
                }
                let positives = values.iter().filter(|value| **value == 1.0).count();
                let negatives = values.len() - positives;
                if positives == 0 || negatives == 0 {
                    return Err(TrainableSetError::new(
                        "class-balanced Bernoulli target requires both training classes",
                    ));
                }
                inverse_stddev.push(1.0);
                positive_weight.push(values.len() as f64 / (2.0 * positives as f64));
                negative_weight.push(values.len() as f64 / (2.0 * negatives as f64));
            }
        }
    }
    if mean
        .iter()
        .chain(&inverse_stddev)
        .chain(&positive_weight)
        .chain(&negative_weight)
        .any(|value| !value.is_finite())
    {
        return Err(TrainableSetError::new(
            "multitask target normalization is non-finite",
        ));
    }
    Ok(TargetNormalization {
        mean,
        inverse_stddev,
        positive_weight,
        negative_weight,
        support,
    })
}

fn logistic(value: f64) -> f64 {
    if value >= 0.0 {
        1.0 / (1.0 + (-value).exp())
    } else {
        let exponential = value.exp();
        exponential / (1.0 + exponential)
    }
}

fn calibrated_binary_probability(
    weighted_logit: f64,
    positive_weight: f64,
    negative_weight: f64,
) -> f64 {
    logistic(weighted_logit + (negative_weight / positive_weight).ln())
}

fn select_binary_decision_threshold(rows: &[(bool, f64)]) -> Result<f64, TrainableSetError> {
    let positives = rows.iter().filter(|(expected, _)| *expected).count();
    let negatives = rows.len().saturating_sub(positives);
    if positives == 0
        || negatives == 0
        || rows
            .iter()
            .any(|(_, probability)| !probability.is_finite() || !(0.0..=1.0).contains(probability))
    {
        return Err(TrainableSetError::new(
            "binary threshold calibration requires finite probabilities and both validation classes",
        ));
    }
    let mut thresholds = rows
        .iter()
        .map(|(_, probability)| *probability)
        .collect::<Vec<_>>();
    thresholds.sort_by(|left, right| right.total_cmp(left));
    thresholds.dedup_by(|left, right| left.to_bits() == right.to_bits());

    let mut best = None::<(f64, f64, f64)>;
    for threshold in thresholds {
        let true_positives = rows
            .iter()
            .filter(|(expected, probability)| *expected && *probability >= threshold)
            .count();
        let false_positives = rows
            .iter()
            .filter(|(expected, probability)| !*expected && *probability >= threshold)
            .count();
        let false_negatives = positives - true_positives;
        let true_negatives = negatives - false_positives;
        let f1 = if 2 * true_positives + false_positives + false_negatives == 0 {
            0.0
        } else {
            2.0 * true_positives as f64
                / (2 * true_positives + false_positives + false_negatives) as f64
        };
        let balanced_accuracy = 0.5
            * (true_positives as f64 / positives as f64 + true_negatives as f64 / negatives as f64);
        let candidate = (f1, balanced_accuracy, threshold);
        if best.is_none_or(|current| {
            candidate.0.total_cmp(&current.0).is_gt()
                || (candidate.0.total_cmp(&current.0).is_eq()
                    && (candidate.1.total_cmp(&current.1).is_gt()
                        || (candidate.1.total_cmp(&current.1).is_eq()
                            && candidate.2.total_cmp(&current.2).is_gt())))
        }) {
            best = Some(candidate);
        }
    }
    Ok(best
        .ok_or_else(|| TrainableSetError::new("binary threshold calibration has no candidates"))?
        .2)
}

fn binary_cross_entropy_from_logit(logit: f64, expected: f64) -> f64 {
    logit.max(0.0) - logit * expected + (-logit.abs()).exp().ln_1p()
}

fn binary_cross_entropy_from_probability(probability: f64, expected: f64) -> f64 {
    let probability = probability.clamp(1.0e-12, 1.0 - 1.0e-12);
    -expected * probability.ln() - (1.0 - expected) * (1.0 - probability).ln()
}

fn relative_improvement(baseline: f64, model: f64) -> f64 {
    if baseline > f64::EPSILON {
        (baseline - model) / baseline
    } else {
        0.0
    }
}

fn report_digest(report: &MultiTaskSetEncoderReport) -> Result<Digest, TrainableSetError> {
    let mut canonical = report.clone();
    canonical.report_sha256 = Digest::ZERO;
    canonical_digest(b"dusklight.multitask-set-encoder-report/v12\0", &canonical)
}

fn canonical_digest<T: Serialize>(domain: &[u8], value: &T) -> Result<Digest, TrainableSetError> {
    let bytes =
        serde_json::to_vec(value).map_err(|error| TrainableSetError::new(error.to_string()))?;
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
    Ok(Digest(hasher.finalize().into()))
}
