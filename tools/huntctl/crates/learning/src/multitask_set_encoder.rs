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

pub const MULTITASK_SET_ENCODER_REPORT_SCHEMA_V10: &str =
    "dusklight-multitask-set-encoder-report/v10";
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
    pub held_out_training_mean_loss: f64,
    pub relative_held_out_improvement: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AuxiliaryHeadEvaluation {
    pub name: String,
    pub objective: AuxiliaryHeadObjective,
    pub support: usize,
    pub loss: f64,
    pub training_mean_loss: f64,
    pub relative_improvement: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MultiTaskSetEvaluation {
    pub samples: usize,
    pub objective_loss: f64,
    pub training_mean_objective_loss: f64,
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
    pub target_support_training: Vec<usize>,
    pub target_support_held_out: Vec<usize>,
    pub maximum_training_nodes: usize,
    pub maximum_held_out_nodes: usize,
    pub parameter_count: usize,
    pub optimizer_steps: u64,
    pub training_objective_loss: f64,
    pub held_out_objective_loss: f64,
    pub held_out_training_mean_objective_loss: f64,
    pub relative_held_out_improvement: f64,
    pub heads: Vec<AuxiliaryHeadMetrics>,
    pub held_out_rare_events: Vec<RareEventMetrics>,
    pub held_out_attention: Vec<AttentionHeadDiagnostics>,
    pub decision: MultiTaskEncoderDecision,
    pub model_sha256: Digest,
    pub promotion_authority: bool,
    pub report_sha256: Digest,
}

#[derive(Clone, Debug, Serialize)]
pub struct CompleteSetMultiTaskEncoder {
    actor_feature_schema_sha256: Digest,
    layout: FeatureLayout,
    config: TrainableSetConfig,
    pooling: MultiTaskSetPooling,
    temporal: MultiTaskTemporalConfig,
    target_names: Vec<String>,
    target_conditioning: Vec<AuxiliaryHeadConditioning>,
    target_objectives: Vec<AuxiliaryHeadObjective>,
    target_mean: Vec<f64>,
    target_inverse_stddev: Vec<f64>,
    target_positive_weight: Vec<f64>,
    target_negative_weight: Vec<f64>,
    node_weights: Vec<f64>,
    node_bias: Vec<f64>,
    attention_queries: Vec<f64>,
    history_gru: Option<GatedRecurrent>,
    state_weights: Vec<f64>,
    state_bias: Vec<f64>,
    output_weights: Vec<f64>,
    output_bias: Vec<f64>,
    optimizer_steps: u64,
}

struct StateForward {
    node_inputs: Vec<Vec<f64>>,
    node_hidden: Vec<Vec<f64>>,
    max_indices: Vec<Option<usize>>,
    attention_weights: Vec<Vec<f64>>,
    attention_pools: Vec<Vec<f64>>,
    state_input: Vec<f64>,
    state_hidden: Vec<f64>,
}

struct ConditionedForward {
    pre: StateForward,
    post: StateForward,
    history: HistoryForward,
    head_inputs: Vec<Vec<f64>>,
    predictions: Vec<f64>,
}

struct HistoryForward {
    states: Vec<StateForward>,
    recurrent_steps: Vec<GatedRecurrentStep>,
    hidden: Vec<f64>,
}

struct EncoderGradients {
    node_weights: Vec<f64>,
    node_bias: Vec<f64>,
    attention_queries: Vec<f64>,
    state_weights: Vec<f64>,
    state_bias: Vec<f64>,
}

impl CompleteSetMultiTaskEncoder {
    #[allow(clippy::too_many_arguments)]
    pub fn fit(
        actor_feature_schema_sha256: Digest,
        training_dataset_sha256: Digest,
        held_out_dataset_sha256: Digest,
        target_names: Vec<String>,
        training: &[MultiTaskSetSample],
        held_out: &[MultiTaskSetSample],
        config: TrainableSetConfig,
    ) -> Result<(MultiTaskSetEncoderReport, Self), TrainableSetError> {
        Self::fit_with_pooling(
            actor_feature_schema_sha256,
            training_dataset_sha256,
            held_out_dataset_sha256,
            target_names,
            training,
            held_out,
            config,
            MultiTaskSetPooling::MeanMax,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn fit_with_pooling(
        actor_feature_schema_sha256: Digest,
        training_dataset_sha256: Digest,
        held_out_dataset_sha256: Digest,
        target_names: Vec<String>,
        training: &[MultiTaskSetSample],
        held_out: &[MultiTaskSetSample],
        config: TrainableSetConfig,
        pooling: MultiTaskSetPooling,
    ) -> Result<(MultiTaskSetEncoderReport, Self), TrainableSetError> {
        Self::fit_with_pooling_and_temporal(
            actor_feature_schema_sha256,
            training_dataset_sha256,
            held_out_dataset_sha256,
            target_names,
            training,
            held_out,
            config,
            pooling,
            MultiTaskTemporalConfig::none(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn fit_with_pooling_and_temporal(
        actor_feature_schema_sha256: Digest,
        training_dataset_sha256: Digest,
        held_out_dataset_sha256: Digest,
        target_names: Vec<String>,
        training: &[MultiTaskSetSample],
        held_out: &[MultiTaskSetSample],
        config: TrainableSetConfig,
        pooling: MultiTaskSetPooling,
        temporal: MultiTaskTemporalConfig,
    ) -> Result<(MultiTaskSetEncoderReport, Self), TrainableSetError> {
        let dimensions = validate_samples(
            actor_feature_schema_sha256,
            training_dataset_sha256,
            held_out_dataset_sha256,
            &target_names,
            training,
            held_out,
            config,
            temporal,
        )?;
        let layout = FeatureLayout::fit(training.iter().flat_map(sample_model_states), dimensions)?;
        let target_conditioning = target_conditioning_for_names(&target_names);
        let target_objectives = target_objectives_for_names(&target_names);
        let normalization = target_normalization(training, &target_objectives)?;
        let target_support_training = normalization.support.clone();
        let target_support_held_out = target_support(held_out, target_names.len());
        if target_support_held_out.contains(&0) {
            return Err(TrainableSetError::new(
                "each auxiliary target requires held-out support",
            ));
        }
        let mut model = Self::initialized(
            actor_feature_schema_sha256,
            layout,
            config,
            target_names.clone(),
            target_conditioning.clone(),
            target_objectives.clone(),
            normalization.mean,
            normalization.inverse_stddev,
            normalization.positive_weight.clone(),
            normalization.negative_weight.clone(),
            pooling,
            temporal,
        )?;
        let mut order = (0..training.len()).collect::<Vec<_>>();
        let mut rng = DeterministicRng::new(config.seed ^ 0x4d55_4c54_4954_4153);
        for _ in 0..config.epochs {
            rng.shuffle(&mut order);
            for &index in &order {
                model.train_one(&training[index])?;
            }
        }
        let model_sha256 = model.model_sha256()?;
        let training_objective_loss = model.objective_loss(training)?;
        let held_out_objective_loss = model.objective_loss(held_out)?;
        let held_out_training_mean_objective_loss = model.training_mean_objective_loss(held_out)?;
        let relative_held_out_improvement = relative_improvement(
            held_out_training_mean_objective_loss,
            held_out_objective_loss,
        );
        let heads = model.head_metrics(training, held_out)?;
        let decision = if relative_held_out_improvement >= config.minimum_relative_improvement {
            MultiTaskEncoderDecision::SharedEncoderCandidate
        } else {
            MultiTaskEncoderDecision::RetainTrainingMeanBaseline
        };
        let held_out_rare_events = model.rare_event_metrics(held_out)?;
        let held_out_attention = model.attention_diagnostics(held_out)?;
        let mut report = MultiTaskSetEncoderReport {
            schema: MULTITASK_SET_ENCODER_REPORT_SCHEMA_V10,
            actor_feature_schema_sha256,
            training_dataset_sha256,
            held_out_dataset_sha256,
            config,
            pooling,
            temporal,
            target_names,
            target_conditioning,
            target_objectives,
            target_positive_weights: normalization.positive_weight,
            target_negative_weights: normalization.negative_weight,
            target_support_training,
            target_support_held_out,
            maximum_training_nodes: training
                .iter()
                .flat_map(sample_model_states)
                .map(|state| state.nodes.len())
                .max()
                .unwrap_or(0),
            maximum_held_out_nodes: held_out
                .iter()
                .flat_map(sample_model_states)
                .map(|state| state.nodes.len())
                .max()
                .unwrap_or(0),
            parameter_count: model.parameter_count(),
            optimizer_steps: model.optimizer_steps,
            training_objective_loss,
            held_out_objective_loss,
            held_out_training_mean_objective_loss,
            relative_held_out_improvement,
            heads,
            held_out_rare_events,
            held_out_attention,
            decision,
            model_sha256,
            promotion_authority: false,
            report_sha256: Digest::ZERO,
        };
        report.report_sha256 = report_digest(&report)?;
        Ok((report, model))
    }

    #[allow(clippy::too_many_arguments)]
    fn initialized(
        actor_feature_schema_sha256: Digest,
        layout: FeatureLayout,
        config: TrainableSetConfig,
        target_names: Vec<String>,
        target_conditioning: Vec<AuxiliaryHeadConditioning>,
        target_objectives: Vec<AuxiliaryHeadObjective>,
        target_mean: Vec<f64>,
        target_inverse_stddev: Vec<f64>,
        target_positive_weight: Vec<f64>,
        target_negative_weight: Vec<f64>,
        pooling: MultiTaskSetPooling,
        temporal: MultiTaskTemporalConfig,
    ) -> Result<Self, TrainableSetError> {
        temporal.validate()?;
        let target_width = target_names.len();
        let global_attention_heads = pooling.global_attention_heads();
        let attention_heads = pooling.attention_heads(target_width);
        let state_input_width =
            layout.base_input_width + 2 + config.node_hidden_width * (2 + global_attention_heads);
        if target_conditioning.len() != target_width
            || target_objectives.len() != target_width
            || target_mean.len() != target_width
            || target_inverse_stddev.len() != target_width
            || target_positive_weight.len() != target_width
            || target_negative_weight.len() != target_width
        {
            return Err(TrainableSetError::new(
                "multitask target conditioning width is invalid",
            ));
        }
        let task_attention_width =
            usize::from(pooling.uses_task_attention()) * config.node_hidden_width * 2;
        let head_input_width = config.head_hidden_width * 2
            + ACTION_CONTEXT_WIDTH
            + temporal.hidden_width
            + task_attention_width;
        let recurrent_parameter_count = match temporal.encoding {
            MultiTaskTemporalEncoding::None => 0,
            MultiTaskTemporalEncoding::GatedRecurrent => temporal
                .hidden_width
                .checked_mul(3)
                .and_then(|value| {
                    value.checked_mul(
                        config.head_hidden_width + ACTION_CONTEXT_WIDTH + temporal.hidden_width + 1,
                    )
                })
                .ok_or_else(|| TrainableSetError::new("multitask parameter count overflowed"))?,
        };
        let parameter_count = config
            .node_hidden_width
            .checked_mul(layout.node_input_width + 1)
            .and_then(|value| value.checked_add(attention_heads * config.node_hidden_width))
            .and_then(|value| value.checked_add(config.head_hidden_width * (state_input_width + 1)))
            .and_then(|value| value.checked_add(recurrent_parameter_count))
            .and_then(|value| value.checked_add(target_width * (head_input_width + 1)))
            .ok_or_else(|| TrainableSetError::new("multitask parameter count overflowed"))?;
        if parameter_count > MAX_PARAMETERS {
            return Err(TrainableSetError::new(
                "multitask set encoder exceeds its parameter budget",
            ));
        }
        let mut rng = DeterministicRng::new(config.seed ^ 0x5348_4152_4544_0001);
        let node_weights =
            initialized_weights(config.node_hidden_width, layout.node_input_width, &mut rng);
        let attention_queries =
            initialized_weights(attention_heads, config.node_hidden_width, &mut rng);
        let state_weights =
            initialized_weights(config.head_hidden_width, state_input_width, &mut rng);
        let output_weights = initialized_weights(target_width, head_input_width, &mut rng);
        let history_gru = match temporal.encoding {
            MultiTaskTemporalEncoding::None => None,
            MultiTaskTemporalEncoding::GatedRecurrent => Some(GatedRecurrent::initialized(
                config.head_hidden_width + ACTION_CONTEXT_WIDTH,
                temporal.hidden_width,
                &mut rng,
            )?),
        };
        let model = Self {
            actor_feature_schema_sha256,
            pooling,
            temporal,
            node_weights,
            node_bias: vec![0.0; config.node_hidden_width],
            attention_queries,
            history_gru,
            state_weights,
            state_bias: vec![0.0; config.head_hidden_width],
            output_weights,
            output_bias: vec![0.0; target_width],
            layout,
            config,
            target_names,
            target_conditioning,
            target_objectives,
            target_mean,
            target_inverse_stddev,
            target_positive_weight,
            target_negative_weight,
            optimizer_steps: 0,
        };
        debug_assert_eq!(model.parameter_count(), parameter_count);
        Ok(model)
    }

    pub fn encode(&self, sample: &TypedSetSample) -> Result<Vec<f32>, TrainableSetError> {
        self.validate_input(sample)?;
        Ok(self
            .state_forward(sample)
            .state_hidden
            .into_iter()
            .map(|value| value as f32)
            .collect())
    }

    pub fn predict(&self, sample: &MultiTaskSetSample) -> Result<Vec<f32>, TrainableSetError> {
        self.validate_transition(sample)?;
        Ok(self
            .conditioned_forward(sample)?
            .predictions
            .iter()
            .enumerate()
            .map(|(target, prediction)| self.prediction_value(target, *prediction) as f32)
            .collect())
    }

    fn prediction_value(&self, target: usize, raw_prediction: f64) -> f64 {
        match self.target_objectives[target] {
            AuxiliaryHeadObjective::NormalizedRegression => {
                raw_prediction / self.target_inverse_stddev[target] + self.target_mean[target]
            }
            AuxiliaryHeadObjective::ClassBalancedBernoulli => logistic(raw_prediction),
        }
    }

    fn target_loss(&self, target: usize, raw_prediction: f64, expected: f64) -> f64 {
        match self.target_objectives[target] {
            AuxiliaryHeadObjective::NormalizedRegression => {
                let normalized =
                    (expected - self.target_mean[target]) * self.target_inverse_stddev[target];
                (raw_prediction - normalized).powi(2)
            }
            AuxiliaryHeadObjective::ClassBalancedBernoulli => {
                self.binary_weight(target, expected)
                    * binary_cross_entropy_from_logit(raw_prediction, expected)
            }
        }
    }

    fn training_mean_loss(&self, target: usize, expected: f64) -> f64 {
        match self.target_objectives[target] {
            AuxiliaryHeadObjective::NormalizedRegression => {
                ((expected - self.target_mean[target]) * self.target_inverse_stddev[target]).powi(2)
            }
            AuxiliaryHeadObjective::ClassBalancedBernoulli => {
                self.binary_weight(target, expected)
                    * binary_cross_entropy_from_probability(self.target_mean[target], expected)
            }
        }
    }

    fn binary_weight(&self, target: usize, expected: f64) -> f64 {
        if expected > 0.5 {
            self.target_positive_weight[target]
        } else {
            self.target_negative_weight[target]
        }
    }

    pub fn model_sha256(&self) -> Result<Digest, TrainableSetError> {
        canonical_digest(b"dusklight.complete-set-multitask-encoder/v8\0", self)
    }

    fn attention_head_count(&self) -> usize {
        self.pooling.attention_heads(self.target_names.len())
    }

    fn head_input_width(&self) -> usize {
        self.config.head_hidden_width * 2
            + ACTION_CONTEXT_WIDTH
            + self.temporal.hidden_width
            + usize::from(self.pooling.uses_task_attention()) * self.config.node_hidden_width * 2
    }

    pub fn parameter_count(&self) -> usize {
        self.node_weights.len()
            + self.node_bias.len()
            + self.attention_queries.len()
            + self
                .history_gru
                .as_ref()
                .map_or(0, GatedRecurrent::parameter_count)
            + self.state_weights.len()
            + self.state_bias.len()
            + self.output_weights.len()
            + self.output_bias.len()
    }

    pub fn evaluate(
        &self,
        samples: &[MultiTaskSetSample],
    ) -> Result<MultiTaskSetEvaluation, TrainableSetError> {
        if samples.is_empty() {
            return Err(TrainableSetError::new(
                "multitask evaluation requires samples",
            ));
        }
        let objective_loss = self.objective_loss(samples)?;
        let training_mean_objective_loss = self.training_mean_objective_loss(samples)?;
        let mut target_loss = vec![0.0; self.target_names.len()];
        let mut baseline_loss = vec![0.0; self.target_names.len()];
        let mut support = vec![0_usize; self.target_names.len()];
        for sample in samples {
            self.validate_transition(sample)?;
            let raw_predictions = self.conditioned_forward(sample)?.predictions;
            for target in 0..self.target_names.len() {
                if sample.target_present[target] {
                    let expected = f64::from(sample.targets[target]);
                    target_loss[target] +=
                        self.target_loss(target, raw_predictions[target], expected);
                    baseline_loss[target] += self.training_mean_loss(target, expected);
                    support[target] += 1;
                }
            }
        }
        let heads = (0..self.target_names.len())
            .map(|target| {
                if support[target] == 0 {
                    return Err(TrainableSetError::new(
                        "multitask evaluation target has no support",
                    ));
                }
                let loss = target_loss[target] / support[target] as f64;
                let training_mean_loss = baseline_loss[target] / support[target] as f64;
                Ok(AuxiliaryHeadEvaluation {
                    name: self.target_names[target].clone(),
                    objective: self.target_objectives[target],
                    support: support[target],
                    loss,
                    training_mean_loss,
                    relative_improvement: relative_improvement(training_mean_loss, loss),
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let rare_events = self.rare_event_metrics(samples)?;
        Ok(MultiTaskSetEvaluation {
            samples: samples.len(),
            objective_loss,
            training_mean_objective_loss,
            relative_improvement: relative_improvement(
                training_mean_objective_loss,
                objective_loss,
            ),
            heads,
            rare_events,
        })
    }

    fn rare_event_metrics(
        &self,
        samples: &[MultiTaskSetSample],
    ) -> Result<Vec<RareEventMetrics>, TrainableSetError> {
        let targets = self
            .target_names
            .iter()
            .enumerate()
            .filter(|(_, name)| rare_event_target(name))
            .map(|(index, name)| (index, name.clone()))
            .collect::<Vec<_>>();
        let mut model = vec![BinaryEventAccumulator::default(); targets.len()];
        let mut baseline = vec![BinaryEventAccumulator::default(); targets.len()];
        for sample in samples {
            let predictions = self.predict(sample)?;
            for (metric, (target, _)) in targets.iter().enumerate() {
                if sample.target_present[*target] {
                    let expected = sample.targets[*target] > 0.5;
                    model[metric].observe(expected, f64::from(predictions[*target]));
                    baseline[metric].observe(expected, self.target_mean[*target]);
                }
            }
        }
        targets
            .into_iter()
            .enumerate()
            .map(|(metric, (_, name))| {
                Ok(RareEventMetrics {
                    name,
                    threshold: 0.5,
                    model: model[metric].finish()?,
                    training_mean_baseline: baseline[metric].finish()?,
                })
            })
            .collect()
    }

    fn attention_diagnostics(
        &self,
        samples: &[MultiTaskSetSample],
    ) -> Result<Vec<AttentionHeadDiagnostics>, TrainableSetError> {
        let heads = self.attention_head_count();
        if heads == 0 {
            return Ok(Vec::new());
        }
        let mut entropy_sum = vec![0.0; heads];
        let mut maximum_sum = vec![0.0; heads];
        let mut support = vec![0_usize; heads];
        for sample in samples {
            self.validate_transition(sample)?;
            let pre = self.state_forward(&sample.input);
            let post = self
                .pooling
                .uses_task_attention()
                .then(|| self.state_forward(&sample.post_input));
            for head in 0..heads {
                if self.pooling.uses_task_attention() && !sample.target_present[head] {
                    continue;
                }
                accumulate_attention_distribution(
                    &pre.attention_weights[head],
                    &mut entropy_sum[head],
                    &mut maximum_sum[head],
                    &mut support[head],
                );
                if self.pooling.uses_task_attention()
                    && self.target_conditioning[head] == AuxiliaryHeadConditioning::PreAndPostState
                {
                    accumulate_attention_distribution(
                        &post
                            .as_ref()
                            .expect("task attention computes post state")
                            .attention_weights[head],
                        &mut entropy_sum[head],
                        &mut maximum_sum[head],
                        &mut support[head],
                    );
                }
            }
        }
        (0..heads)
            .map(|head| {
                if support[head] == 0 {
                    return Err(TrainableSetError::new(
                        "learned attention diagnostics have no actor support",
                    ));
                }
                let query = &self.attention_queries[head * self.config.node_hidden_width
                    ..(head + 1) * self.config.node_hidden_width];
                Ok(AttentionHeadDiagnostics {
                    head,
                    target: self
                        .pooling
                        .uses_task_attention()
                        .then(|| self.target_names[head].clone()),
                    conditioning: self
                        .pooling
                        .uses_task_attention()
                        .then(|| self.target_conditioning[head]),
                    observation_support: support[head],
                    query_l2_norm: query.iter().map(|value| value * value).sum::<f64>().sqrt(),
                    mean_normalized_entropy: entropy_sum[head] / support[head] as f64,
                    mean_maximum_weight: maximum_sum[head] / support[head] as f64,
                })
            })
            .collect()
    }

    fn validate_input(&self, sample: &TypedSetSample) -> Result<(), TrainableSetError> {
        if sample.actor_feature_schema_sha256 != self.actor_feature_schema_sha256 {
            return Err(TrainableSetError::new(
                "multitask sample actor schema does not match model",
            ));
        }
        validate_sample_dimensions(sample, self.layout.dimensions())
    }

    fn validate_transition(&self, sample: &MultiTaskSetSample) -> Result<(), TrainableSetError> {
        self.validate_input(&sample.input)?;
        self.validate_input(&sample.post_input)?;
        if sample.action_context.len() != ACTION_CONTEXT_WIDTH
            || sample.action_context.iter().any(|value| !value.is_finite())
        {
            return Err(TrainableSetError::new(
                "multitask action context is invalid",
            ));
        }
        let history_valid = match self.temporal.encoding {
            MultiTaskTemporalEncoding::None => sample.history.is_empty(),
            MultiTaskTemporalEncoding::GatedRecurrent => {
                sample.history.len() <= self.temporal.history_depth
            }
        };
        let mut history_identities = BTreeSet::new();
        if !history_valid {
            return Err(TrainableSetError::new(
                "multitask history does not match the temporal model",
            ));
        }
        for step in &sample.history {
            if step.transition_sha256 == Digest::ZERO
                || !history_identities.insert(step.transition_sha256)
                || step.action_context.len() != ACTION_CONTEXT_WIDTH
                || step.action_context.iter().any(|value| !value.is_finite())
            {
                return Err(TrainableSetError::new(
                    "multitask history identity or action is invalid",
                ));
            }
            self.validate_input(&step.state)?;
        }
        Ok(())
    }

    fn state_forward(&self, sample: &TypedSetSample) -> StateForward {
        let nodes = ordered_nodes(&sample.nodes);
        let node_inputs = nodes
            .iter()
            .map(|node| self.layout.node_input(node))
            .collect::<Vec<_>>();
        let node_hidden = node_inputs
            .iter()
            .map(|input| {
                dense_tanh(
                    input,
                    &self.node_weights,
                    &self.node_bias,
                    self.config.node_hidden_width,
                )
            })
            .collect::<Vec<_>>();
        let mut mean_pool = vec![0.0; self.config.node_hidden_width];
        let mut max_pool = vec![0.0; self.config.node_hidden_width];
        let mut max_indices = vec![None; self.config.node_hidden_width];
        if !node_hidden.is_empty() {
            max_pool.fill(f64::NEG_INFINITY);
            for (node_index, hidden) in node_hidden.iter().enumerate() {
                for feature in 0..hidden.len() {
                    mean_pool[feature] += hidden[feature];
                    if hidden[feature] > max_pool[feature] {
                        max_pool[feature] = hidden[feature];
                        max_indices[feature] = Some(node_index);
                    }
                }
            }
            for value in &mut mean_pool {
                *value /= node_hidden.len() as f64;
            }
        }
        let attention_heads = self.attention_head_count();
        let mut attention_weights = Vec::with_capacity(attention_heads);
        let mut attention_pools = Vec::with_capacity(attention_heads);
        for head in 0..attention_heads {
            if node_hidden.is_empty() {
                attention_weights.push(Vec::new());
                attention_pools.push(vec![0.0; self.config.node_hidden_width]);
                continue;
            }
            let query = &self.attention_queries
                [head * self.config.node_hidden_width..(head + 1) * self.config.node_hidden_width];
            let logits = node_hidden
                .iter()
                .map(|hidden| dot(hidden, query))
                .collect::<Vec<_>>();
            let maximum = logits.iter().copied().max_by(f64::total_cmp).unwrap_or(0.0);
            let mut weights = logits
                .iter()
                .map(|logit| (logit - maximum).exp())
                .collect::<Vec<_>>();
            let denominator = weights.iter().sum::<f64>();
            for weight in &mut weights {
                *weight /= denominator;
            }
            let mut pool = vec![0.0; self.config.node_hidden_width];
            for (hidden, weight) in node_hidden.iter().zip(&weights) {
                for (pooled, value) in pool.iter_mut().zip(hidden) {
                    *pooled += weight * value;
                }
            }
            attention_weights.push(weights);
            attention_pools.push(pool);
        }
        let mut state_input = self.layout.base_input(sample);
        state_input.push(f64::from(!sample.nodes.is_empty()));
        state_input.push((sample.nodes.len() as f64).ln_1p() / (u16::MAX as f64).ln_1p());
        state_input.extend(mean_pool);
        state_input.extend(max_pool);
        state_input.extend(
            attention_pools[..self.pooling.global_attention_heads()]
                .iter()
                .flatten()
                .copied(),
        );
        let state_hidden = dense_tanh(
            &state_input,
            &self.state_weights,
            &self.state_bias,
            self.config.head_hidden_width,
        );
        StateForward {
            node_inputs,
            node_hidden,
            max_indices,
            attention_weights,
            attention_pools,
            state_input,
            state_hidden,
        }
    }

    fn history_forward(
        &self,
        sample: &MultiTaskSetSample,
    ) -> Result<HistoryForward, TrainableSetError> {
        let Some(recurrent) = &self.history_gru else {
            return Ok(HistoryForward {
                states: Vec::new(),
                recurrent_steps: Vec::new(),
                hidden: Vec::new(),
            });
        };
        let states = sample
            .history
            .iter()
            .map(|step| self.state_forward(&step.state))
            .collect::<Vec<_>>();
        let inputs = states
            .iter()
            .zip(&sample.history)
            .map(|(state, step)| {
                let mut input = Vec::with_capacity(recurrent.input_width());
                input.extend(&state.state_hidden);
                input.extend(step.action_context.iter().map(|value| f64::from(*value)));
                input
            })
            .collect::<Vec<_>>();
        let recurrent_steps = recurrent.forward_sequence(&inputs)?;
        let hidden = recurrent_steps.last().map_or_else(
            || vec![0.0; recurrent.hidden_width()],
            |step| step.hidden.clone(),
        );
        Ok(HistoryForward {
            states,
            recurrent_steps,
            hidden,
        })
    }

    fn conditioned_forward(
        &self,
        sample: &MultiTaskSetSample,
    ) -> Result<ConditionedForward, TrainableSetError> {
        let pre = self.state_forward(&sample.input);
        let post = self.state_forward(&sample.post_input);
        let history = self.history_forward(sample)?;
        let head_input_width = self.head_input_width();
        let head_inputs = self
            .target_conditioning
            .iter()
            .enumerate()
            .map(|(target, conditioning)| {
                let mut input = Vec::with_capacity(head_input_width);
                input.extend(&pre.state_hidden);
                match conditioning {
                    AuxiliaryHeadConditioning::PreStateAndAction => {
                        input.extend(std::iter::repeat_n(0.0, self.config.head_hidden_width));
                        input.extend(sample.action_context.iter().map(|value| f64::from(*value)));
                    }
                    AuxiliaryHeadConditioning::PreAndPostState => {
                        input.extend(&post.state_hidden);
                        input.extend(std::iter::repeat_n(0.0, ACTION_CONTEXT_WIDTH));
                    }
                }
                input.extend(&history.hidden);
                if self.pooling.uses_task_attention() {
                    input.extend(&pre.attention_pools[target]);
                    match conditioning {
                        AuxiliaryHeadConditioning::PreStateAndAction => {
                            input.extend(std::iter::repeat_n(0.0, self.config.node_hidden_width))
                        }
                        AuxiliaryHeadConditioning::PreAndPostState => {
                            input.extend(&post.attention_pools[target]);
                        }
                    }
                }
                input
            })
            .collect::<Vec<_>>();
        let predictions = head_inputs
            .iter()
            .enumerate()
            .map(|(target, input)| {
                dot(
                    input,
                    &self.output_weights
                        [target * head_input_width..(target + 1) * head_input_width],
                ) + self.output_bias[target]
            })
            .collect();
        Ok(ConditionedForward {
            pre,
            post,
            history,
            head_inputs,
            predictions,
        })
    }

    fn train_one(&mut self, sample: &MultiTaskSetSample) -> Result<(), TrainableSetError> {
        self.validate_transition(sample)?;
        let forward = self.conditioned_forward(sample)?;
        let output_before = self.output_weights.clone();
        let state_before = self.state_weights.clone();
        let attention_before = self.attention_queries.clone();
        let head_input_width = self.head_input_width();
        let present_count = sample
            .target_present
            .iter()
            .filter(|present| **present)
            .count();
        let mut d_outputs = vec![0.0; self.target_names.len()];
        for (target, d_output) in d_outputs.iter_mut().enumerate() {
            if !sample.target_present[target] {
                continue;
            }
            let expected = f64::from(sample.targets[target]);
            let gradient = match self.target_objectives[target] {
                AuxiliaryHeadObjective::NormalizedRegression => {
                    let normalized =
                        (expected - self.target_mean[target]) * self.target_inverse_stddev[target];
                    2.0 * (forward.predictions[target] - normalized)
                }
                AuxiliaryHeadObjective::ClassBalancedBernoulli => {
                    self.binary_weight(target, expected)
                        * (logistic(forward.predictions[target]) - expected)
                }
            };
            *d_output = clip(gradient / present_count as f64, self.config.gradient_clip);
            for input in 0..head_input_width {
                let parameter = target * head_input_width + input;
                let gradient = *d_output * forward.head_inputs[target][input]
                    + self.config.l2_penalty * self.output_weights[parameter];
                self.output_weights[parameter] -=
                    self.config.learning_rate * clip(gradient, self.config.gradient_clip);
            }
            self.output_bias[target] -= self.config.learning_rate * *d_output;
        }
        let mut d_pre_hidden = vec![0.0; self.config.head_hidden_width];
        let mut d_post_hidden = vec![0.0; self.config.head_hidden_width];
        let mut d_history_hidden = vec![0.0; self.temporal.hidden_width];
        let mut d_pre_attention =
            vec![vec![0.0; self.config.node_hidden_width]; self.attention_head_count()];
        let mut d_post_attention =
            vec![vec![0.0; self.config.node_hidden_width]; self.attention_head_count()];
        for hidden in 0..self.config.head_hidden_width {
            for target in 0..self.target_names.len() {
                d_pre_hidden[hidden] +=
                    d_outputs[target] * output_before[target * head_input_width + hidden];
                if self.target_conditioning[target] == AuxiliaryHeadConditioning::PreAndPostState {
                    d_post_hidden[hidden] += d_outputs[target]
                        * output_before
                            [target * head_input_width + self.config.head_hidden_width + hidden];
                }
            }
        }
        let history_offset = self.config.head_hidden_width * 2 + ACTION_CONTEXT_WIDTH;
        for (hidden, gradient) in d_history_hidden.iter_mut().enumerate() {
            for target in 0..self.target_names.len() {
                *gradient += d_outputs[target]
                    * output_before[target * head_input_width + history_offset + hidden];
            }
        }
        if self.pooling.uses_task_attention() {
            let pre_offset = history_offset + self.temporal.hidden_width;
            let post_offset = pre_offset + self.config.node_hidden_width;
            for target in 0..self.target_names.len() {
                for hidden in 0..self.config.node_hidden_width {
                    d_pre_attention[target][hidden] += d_outputs[target]
                        * output_before[target * head_input_width + pre_offset + hidden];
                    if self.target_conditioning[target]
                        == AuxiliaryHeadConditioning::PreAndPostState
                    {
                        d_post_attention[target][hidden] += d_outputs[target]
                            * output_before[target * head_input_width + post_offset + hidden];
                    }
                }
            }
        }
        let mut gradients = EncoderGradients {
            node_weights: vec![0.0; self.node_weights.len()],
            node_bias: vec![0.0; self.node_bias.len()],
            attention_queries: vec![0.0; self.attention_queries.len()],
            state_weights: vec![0.0; self.state_weights.len()],
            state_bias: vec![0.0; self.state_bias.len()],
        };
        self.accumulate_encoder_gradients(
            &forward.pre,
            &d_pre_hidden,
            &d_pre_attention,
            &state_before,
            &attention_before,
            &mut gradients,
        );
        let recurrent_gradients = if let Some(recurrent) = &self.history_gru {
            let (recurrent_gradients, history_input_gradients) =
                recurrent.backward_sequence(&forward.history.recurrent_steps, &d_history_hidden)?;
            if history_input_gradients.len() != forward.history.states.len() {
                return Err(TrainableSetError::new(
                    "multitask recurrent history gradient count is invalid",
                ));
            }
            let no_direct_attention =
                vec![vec![0.0; self.config.node_hidden_width]; self.attention_head_count()];
            for (state, input_gradient) in
                forward.history.states.iter().zip(&history_input_gradients)
            {
                self.accumulate_encoder_gradients(
                    state,
                    &input_gradient[..self.config.head_hidden_width],
                    &no_direct_attention,
                    &state_before,
                    &attention_before,
                    &mut gradients,
                );
            }
            Some(recurrent_gradients)
        } else {
            None
        };
        self.accumulate_encoder_gradients(
            &forward.post,
            &d_post_hidden,
            &d_post_attention,
            &state_before,
            &attention_before,
            &mut gradients,
        );
        for (weight, gradient) in self.state_weights.iter_mut().zip(gradients.state_weights) {
            let gradient = gradient + self.config.l2_penalty * *weight;
            *weight -= self.config.learning_rate * clip(gradient, self.config.gradient_clip);
        }
        for (bias, gradient) in self.state_bias.iter_mut().zip(gradients.state_bias) {
            *bias -= self.config.learning_rate * clip(gradient, self.config.gradient_clip);
        }
        for (weight, gradient) in self.node_weights.iter_mut().zip(gradients.node_weights) {
            let gradient = gradient + self.config.l2_penalty * *weight;
            *weight -= self.config.learning_rate * clip(gradient, self.config.gradient_clip);
        }
        for (bias, gradient) in self.node_bias.iter_mut().zip(gradients.node_bias) {
            *bias -= self.config.learning_rate * clip(gradient, self.config.gradient_clip);
        }
        for (weight, gradient) in self
            .attention_queries
            .iter_mut()
            .zip(gradients.attention_queries)
        {
            let gradient = gradient + self.config.l2_penalty * *weight;
            *weight -= self.config.learning_rate * clip(gradient, self.config.gradient_clip);
        }
        if let (Some(recurrent), Some(recurrent_gradients)) =
            (&mut self.history_gru, recurrent_gradients)
        {
            recurrent.apply_gradients(
                recurrent_gradients,
                self.config.learning_rate,
                self.config.l2_penalty,
                self.config.gradient_clip,
            );
        }
        self.optimizer_steps += 1;
        if self
            .node_weights
            .iter()
            .chain(&self.node_bias)
            .chain(&self.attention_queries)
            .chain(&self.state_weights)
            .chain(&self.state_bias)
            .chain(&self.output_weights)
            .chain(&self.output_bias)
            .any(|value| !value.is_finite())
            || self
                .history_gru
                .as_ref()
                .is_some_and(|recurrent| !recurrent.all_finite())
        {
            return Err(TrainableSetError::new(
                "multitask set encoder parameters became non-finite",
            ));
        }
        Ok(())
    }

    fn accumulate_encoder_gradients(
        &self,
        forward: &StateForward,
        d_hidden: &[f64],
        direct_attention: &[Vec<f64>],
        state_before: &[f64],
        attention_before: &[f64],
        gradients: &mut EncoderGradients,
    ) {
        let d_state_pre = d_hidden
            .iter()
            .zip(&forward.state_hidden)
            .map(|(gradient, hidden)| gradient * (1.0 - hidden.powi(2)))
            .collect::<Vec<_>>();
        let mut d_state_input = vec![0.0; forward.state_input.len()];
        for (output, delta) in d_state_pre.iter().copied().enumerate() {
            for (input, d_input) in d_state_input.iter_mut().enumerate() {
                let parameter = output * forward.state_input.len() + input;
                *d_input += state_before[parameter] * delta;
                gradients.state_weights[parameter] += delta * forward.state_input[input];
            }
            gradients.state_bias[output] += delta;
        }
        let pool_offset = self.layout.base_input_width + 2;
        let d_mean = &d_state_input[pool_offset..pool_offset + self.config.node_hidden_width];
        let d_max = &d_state_input[pool_offset + self.config.node_hidden_width
            ..pool_offset + self.config.node_hidden_width * 2];
        let attention_offset = pool_offset + self.config.node_hidden_width * 2;
        let attention_heads = self.attention_head_count();
        let mut d_attention = vec![vec![0.0; self.config.node_hidden_width]; attention_heads];
        for (head, d_pool) in d_attention
            .iter_mut()
            .take(self.pooling.global_attention_heads())
            .enumerate()
        {
            d_pool.copy_from_slice(
                &d_state_input[attention_offset + head * self.config.node_hidden_width
                    ..attention_offset + (head + 1) * self.config.node_hidden_width],
            );
        }
        for (d_pool, direct) in d_attention.iter_mut().zip(direct_attention) {
            for (gradient, additional) in d_pool.iter_mut().zip(direct) {
                *gradient += additional;
            }
        }
        let node_count = forward.node_hidden.len();
        for node_index in 0..node_count {
            for hidden in 0..self.config.node_hidden_width {
                let mut gradient = d_mean[hidden] / node_count as f64;
                if forward.max_indices[hidden] == Some(node_index) {
                    gradient += d_max[hidden];
                }
                for (head, d_pool) in d_attention.iter().enumerate() {
                    let weight = forward.attention_weights[head][node_index];
                    let score_gradient = weight
                        * d_pool
                            .iter()
                            .enumerate()
                            .map(|(feature, d_value)| {
                                d_value
                                    * (forward.node_hidden[node_index][feature]
                                        - forward.attention_pools[head][feature])
                            })
                            .sum::<f64>();
                    gradient += weight * d_pool[hidden]
                        + score_gradient
                            * attention_before[head * self.config.node_hidden_width + hidden];
                    gradients.attention_queries[head * self.config.node_hidden_width + hidden] +=
                        score_gradient * forward.node_hidden[node_index][hidden];
                }
                let delta = gradient * (1.0 - forward.node_hidden[node_index][hidden].powi(2));
                for input in 0..self.layout.node_input_width {
                    gradients.node_weights[hidden * self.layout.node_input_width + input] +=
                        delta * forward.node_inputs[node_index][input];
                }
                gradients.node_bias[hidden] += delta;
            }
        }
    }

    fn objective_loss(&self, samples: &[MultiTaskSetSample]) -> Result<f64, TrainableSetError> {
        let mut loss = 0.0;
        let mut count = 0_usize;
        for sample in samples {
            self.validate_transition(sample)?;
            let prediction = self.conditioned_forward(sample)?.predictions;
            for (target, predicted) in prediction.iter().enumerate() {
                if sample.target_present[target] {
                    loss += self.target_loss(target, *predicted, f64::from(sample.targets[target]));
                    count += 1;
                }
            }
        }
        Ok(loss / count as f64)
    }

    fn training_mean_objective_loss(
        &self,
        samples: &[MultiTaskSetSample],
    ) -> Result<f64, TrainableSetError> {
        let mut loss = 0.0;
        let mut count = 0_usize;
        for sample in samples {
            for target in 0..self.target_names.len() {
                if sample.target_present[target] {
                    loss += self.training_mean_loss(target, f64::from(sample.targets[target]));
                    count += 1;
                }
            }
        }
        if count == 0 {
            return Err(TrainableSetError::new(
                "multitask baseline has no supported targets",
            ));
        }
        Ok(loss / count as f64)
    }

    fn head_metrics(
        &self,
        training: &[MultiTaskSetSample],
        held_out: &[MultiTaskSetSample],
    ) -> Result<Vec<AuxiliaryHeadMetrics>, TrainableSetError> {
        let collect = |samples: &[MultiTaskSetSample]| {
            let mut target_loss = vec![0.0; self.target_names.len()];
            let mut baseline_loss = vec![0.0; self.target_names.len()];
            let mut support = vec![0_usize; self.target_names.len()];
            for sample in samples {
                self.validate_transition(sample)?;
                let raw_predictions = self.conditioned_forward(sample)?.predictions;
                for target in 0..self.target_names.len() {
                    if sample.target_present[target] {
                        let expected = f64::from(sample.targets[target]);
                        target_loss[target] +=
                            self.target_loss(target, raw_predictions[target], expected);
                        baseline_loss[target] += self.training_mean_loss(target, expected);
                        support[target] += 1;
                    }
                }
            }
            Ok::<_, TrainableSetError>((support, target_loss, baseline_loss))
        };
        let (training_support, training_error, _) = collect(training)?;
        let (held_out_support, held_out_error, held_out_baseline_error) = collect(held_out)?;
        Ok((0..self.target_names.len())
            .map(|target| {
                let training_loss = training_error[target] / training_support[target] as f64;
                let held_out_loss = held_out_error[target] / held_out_support[target] as f64;
                let held_out_training_mean_loss =
                    held_out_baseline_error[target] / held_out_support[target] as f64;
                AuxiliaryHeadMetrics {
                    name: self.target_names[target].clone(),
                    objective: self.target_objectives[target],
                    training_support: training_support[target],
                    held_out_support: held_out_support[target],
                    training_loss,
                    held_out_loss,
                    held_out_training_mean_loss,
                    relative_held_out_improvement: relative_improvement(
                        held_out_training_mean_loss,
                        held_out_loss,
                    ),
                }
            })
            .collect())
    }
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
    fn observe(&mut self, expected: bool, score: f64) {
        let probability = score.clamp(0.0, 1.0);
        let predicted = probability >= 0.5;
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

fn native_actor_feature_schema(
    spec: &NativeEncoderFeatureSpec,
) -> Result<Digest, TrainableSetError> {
    canonical_digest(
        b"dusklight.native-direct-actor-features/v10\0",
        &(
            spec,
            "signed-log-presence-unit-rms-reservoir/v1",
            HISTORY_RESERVOIR_SEED,
            selected_feature_names(
                native_base_feature_names(),
                &native_base_feature_families(),
                spec,
            ),
            selected_feature_names(
                native_actor_categorical_names(),
                &native_actor_categorical_families(),
                spec,
            ),
            selected_feature_names(
                native_actor_continuous_names(),
                &native_actor_continuous_families(),
                spec,
            ),
            selected_feature_names(
                native_actor_binary_names(),
                &native_actor_binary_families(),
                spec,
            ),
            native_history_feature_names(spec),
        ),
    )
}

fn native_history_feature_names(spec: &NativeEncoderFeatureSpec) -> Vec<String> {
    if matches!(
        spec.history_encoding,
        NativeEncoderHistoryEncoding::None | NativeEncoderHistoryEncoding::TrainableGru
    ) {
        return Vec::new();
    }
    if spec.history_encoding == NativeEncoderHistoryEncoding::RecurrentReservoir {
        let mut names = vec![
            "history_recurrent_available".into(),
            "history_recurrent_fill".into(),
        ];
        names.extend(
            (0..spec.history_recurrent_width)
                .map(|index| format!("history_recurrent_hidden_{index}")),
        );
        return names;
    }
    let core_names = selected_feature_names(
        native_base_feature_names(),
        &native_base_feature_families(),
        spec,
    );
    let action_names = [
        "stick_x",
        "stick_y",
        "substick_x",
        "substick_y",
        "trigger_left",
        "trigger_right",
        "analog_a",
        "analog_b",
    ];
    let mut names = Vec::new();
    for slot in 0..spec.history_depth {
        names.push(format!("history_{slot}_present"));
        names.extend(
            action_names
                .iter()
                .map(|name| format!("history_{slot}_action_{name}")),
        );
        names.extend((0..16).map(|bit| format!("history_{slot}_action_button_{bit}")));
        names.extend(
            core_names
                .iter()
                .map(|name| format!("history_{slot}_state_{name}")),
        );
    }
    names
}

fn native_base_feature_names() -> Vec<String> {
    let mut names = Vec::new();
    for prefix in [
        "player_position",
        "player_velocity",
        "player_current_angle_s16",
        "player_shape_angle_s16",
    ] {
        extend_vec3_feature_names(&mut names, prefix);
    }
    names.insert(6, "player_forward_speed".into());
    names.push("player_procedure".into());
    names.extend((0..32).map(|bit| format!("player_mode_flag_{bit}")));
    names.extend((0..8).map(|bit| format!("player_contact_bit_{bit}")));
    names.extend(
        [
            "event_running",
            "event_id",
            "event_mode",
            "event_status",
            "event_map_tool_id",
            "room",
            "layer",
            "point",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    names.extend(
        [
            "event_transition_data_loaded",
            "event_transition_camera_play",
            "event_transition_current_event_id",
            "event_transition_current_event_type",
            "event_transition_current_event_room",
            "event_transition_goal_x",
            "event_transition_goal_y",
            "event_transition_goal_z",
            "event_transition_pending_stage",
            "event_transition_pending_room",
            "event_transition_pending_layer",
            "event_transition_pending_point",
            "event_transition_pending_wipe",
            "event_transition_pending_wipe_speed",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    names.extend(
        [
            "clock_framework_frames",
            "clock_gameplay_frames",
            "clock_global_pause",
            "clock_scene_paused",
            "clock_scene_pause_timer",
            "clock_scene_next_pause_timer",
            "clock_overlap_request_active",
            "clock_overlap_fadeout_peek",
            "clock_demo_present",
            "clock_demo_mode",
            "clock_demo_frame",
            "clock_demo_frame_no_message",
            "clock_demo_flags",
            "clock_timer_present",
            "clock_timer_mode",
            "clock_timer_now_ms",
            "clock_timer_limit_ms",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    names.extend(
        [
            "warp_request_kind",
            "warp_selection_present",
            "warp_selection_position_x",
            "warp_selection_position_y",
            "warp_selection_position_z",
            "warp_selection_angle",
            "warp_selection_room",
            "warp_selection_parameter",
            "warp_selection_player",
            "warp_selection_stage_matches_current",
            "warp_return_mark_present",
            "warp_return_position_x",
            "warp_return_position_y",
            "warp_return_position_z",
            "warp_return_angle",
            "warp_return_room",
            "warp_return_accept_stage",
            "warp_return_stage_matches_current",
            "warp_target_point_present",
            "warp_target_point",
            "warp_selected_point_present",
            "warp_selected_point",
            "warp_transport_match",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    names.extend(
        [
            "previous_stick_x",
            "previous_stick_y",
            "previous_substick_x",
            "previous_substick_y",
            "previous_trigger_left",
            "previous_trigger_right",
            "previous_analog_a",
            "previous_analog_b",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    names.extend((0..16).map(|bit| format!("previous_button_bit_{bit}")));
    names.extend(
        [
            "camera_yaw_radians",
            "camera_view_yaw_s16",
            "camera_controlled_yaw_s16",
            "camera_bank_s16",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    extend_vec3_feature_names(&mut names, "camera_eye");
    extend_vec3_feature_names(&mut names, "camera_center");
    names.extend(
        [
            "player_ground_height",
            "player_roof_height",
            "player_water_height",
            "collision_correction_x",
            "collision_correction_z",
            "scene_exit_signed_distance",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    extend_vec3_feature_names(&mut names, "scene_exit_player_local_position");
    extend_vec3_feature_names(&mut names, "scene_exit_volume_extent");
    for stream in 0..2 {
        names.push(format!("rng_{stream}_id"));
        for state in 0..3 {
            names.push(format!("rng_{stream}_state_{state}"));
        }
        names.push(format!("rng_{stream}_call_count"));
    }
    names.extend(["rng_stream_count".into(), "rng_stream_overflow".into()]);
    names.extend(
        [
            "goal_requested_count",
            "goal_hit_count",
            "goal_stable_ticks",
            "goal_consecutive_ticks",
            "goal_sequence_steps",
            "goal_sequence_next_step",
            "goal_sequence_within_ticks",
            "goal_sequence_elapsed_ticks",
            "goal_reached",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    names.extend((0..32).map(|bit| format!("attention_player_flag_{bit}")));
    names.extend(
        [
            "attention_status",
            "attention_block_timer",
            "attention_lock_count",
            "attention_lock_offset",
            "attention_action_count",
            "attention_action_offset",
            "attention_check_count",
            "attention_check_offset",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    for prefix in ["temporal_player_position", "temporal_player_velocity"] {
        extend_vec3_feature_names(&mut names, prefix);
    }
    names.extend(
        [
            "temporal_player_forward_speed_delta",
            "temporal_camera_yaw_delta",
            "temporal_ground_height_delta",
            "temporal_roof_height_delta",
            "temporal_previous_state_available",
            "temporal_player_comparable",
            "temporal_procedure_changed",
            "temporal_mode_changed",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    names.extend((0..8).map(|bit| format!("temporal_contact_changed_bit_{bit}")));
    names.extend(
        [
            "temporal_event_running_changed",
            "temporal_event_id_changed",
            "temporal_context_changed",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    names
}

fn native_actor_categorical_names() -> Vec<String> {
    [
        "parameters",
        "status",
        "actor_name",
        "profile_name",
        "set_id",
        "home_room",
        "current_room",
        "group",
        "argument",
        "health",
        "actor_type",
        "process_subtype",
        "condition",
        "old_room",
        "pause_flag",
        "process_init_state",
        "process_create_phase",
        "cull_type",
        "demo_actor_id",
        "carry_type",
        "attention_flags",
        "attention_distance_0",
        "attention_distance_1",
        "attention_distance_2",
        "attention_distance_3",
        "attention_distance_4",
        "attention_distance_5",
        "attention_distance_6",
        "attention_distance_7",
        "attention_distance_8",
        "attention_auxiliary",
        "event_command",
        "event_condition",
        "event_id",
        "event_map_tool_id",
        "event_index",
        "return_save_room",
        "return_save_point",
        "return_switch_room",
        "return_required_event_set",
        "return_required_event_unset",
        "return_required_switch_set",
        "return_required_switch_unset",
        "enemy_flags",
        "enemy_throw_mode",
        "trigger_kind",
        "trigger_shape",
        "trigger_behavior",
        "door20_kind",
        "door20_model",
        "door20_front_option",
        "door20_back_option",
        "door20_front_room",
        "door20_back_room",
        "door20_exit_number",
        "door20_front_switch",
        "door20_back_switch",
        "door20_unlock_effect_switch",
        "door20_front_event",
        "door20_back_event",
        "door20_message_number",
        "door20_action",
        "door20_active_side",
        "door20_event_variant",
        "door20_key_type",
        "door20_enemy_clear_debounce",
        "door20_stopper_side",
        "door20_front_stopper_status",
        "door20_back_stopper_status",
        "attention_lock_type",
        "attention_lock_rank",
        "attention_action_type",
        "attention_action_rank",
        "attention_check_type",
        "attention_check_rank",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn native_actor_continuous_names() -> Vec<String> {
    let mut names = Vec::new();
    for prefix in [
        "absolute_position",
        "absolute_home_position",
        "absolute_old_position",
        "absolute_velocity",
    ] {
        extend_vec3_feature_names(&mut names, prefix);
    }
    names.push("forward_speed".into());
    extend_vec3_feature_names(&mut names, "scale");
    names.extend(["gravity".into(), "max_fall_speed".into()]);
    extend_vec3_feature_names(&mut names, "absolute_eye_position");
    for prefix in [
        "home_angle_s16",
        "old_angle_s16",
        "current_angle_s16",
        "shape_angle_s16",
        "link_relative_position",
        "link_relative_home_position",
        "link_relative_velocity",
    ] {
        extend_vec3_feature_names(&mut names, prefix);
    }
    names.push("link_distance".into());
    extend_vec3_feature_names(&mut names, "parent_relative_position");
    extend_vec3_feature_names(&mut names, "parent_relative_velocity");
    extend_vec3_feature_names(&mut names, "attention_absolute_position");
    extend_vec3_feature_names(&mut names, "attention_link_relative_position");
    extend_vec3_feature_names(&mut names, "enemy_absolute_down_position");
    extend_vec3_feature_names(&mut names, "enemy_absolute_head_lock_position");
    extend_vec3_feature_names(&mut names, "trigger_absolute_center");
    extend_vec3_feature_names(&mut names, "trigger_half_extent");
    extend_vec3_feature_names(&mut names, "trigger_link_relative_center");
    names.extend([
        "trigger_yaw_relative_to_link_sin".into(),
        "trigger_yaw_relative_to_link_cos".into(),
    ]);
    names.push("door20_angle_s16".into());
    for prefix in ["attention_lock", "attention_action", "attention_check"] {
        names.extend([
            format!("{prefix}_weight"),
            format!("{prefix}_distance"),
            format!("{prefix}_angle_s16"),
        ]);
    }
    extend_vec3_feature_names(&mut names, "temporal_position_delta");
    extend_vec3_feature_names(&mut names, "temporal_velocity_delta");
    names.push("temporal_forward_speed_delta".into());
    extend_vec3_feature_names(&mut names, "temporal_current_angle_delta_s16");
    extend_vec3_feature_names(&mut names, "temporal_shape_angle_delta_s16");
    extend_vec3_feature_names(&mut names, "temporal_attention_position_delta");
    names
}

fn native_actor_binary_names() -> Vec<String> {
    let mut names = [
        "base_state_available",
        "heap_present",
        "model_present",
        "joint_collision_present",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    names.extend((0..32).map(|bit| format!("status_bit_{bit}")));
    names.extend(
        [
            "attention_present",
            "event_participation_present",
            "return_place_writer_present",
            "enemy_base_present",
            "return_no_telop_clear",
            "return_event_set_satisfied",
            "return_event_unset_satisfied",
            "return_switch_set_satisfied",
            "return_switch_unset_satisfied",
            "return_eligible",
            "player_targeted_actor",
            "player_ride_actor",
            "player_held_item_actor",
            "player_grabbed_actor",
            "player_thrown_boomerang_actor",
            "player_copy_rod_actor",
            "player_hookshot_roof_wait_actor",
            "player_chain_grab_actor",
            "player_attention_hint_actor",
            "player_attention_catch_actor",
            "player_attention_look_actor",
            "trigger_volume_present",
            "trigger_enabled",
            "trigger_vertical_unbounded",
            "door20_present",
            "door20_message_door",
            "door20_front_switch_set",
            "door20_back_switch_set",
            "door20_unlock_effect_switch_set",
            "door20_locked",
            "door20_background_collision_released",
            "door20_unlock_effect_triggered",
            "door20_opening_active",
            "door20_closing_active",
            "attention_lock_candidate",
            "attention_action_candidate",
            "attention_check_candidate",
            "temporal_previous_actor_present",
            "temporal_base_state_changed",
            "temporal_actor_type_changed",
            "temporal_process_subtype_changed",
            "temporal_parameters_changed",
            "temporal_status_changed",
            "temporal_condition_changed",
            "temporal_home_room_changed",
            "temporal_old_room_changed",
            "temporal_current_room_changed",
            "temporal_group_changed",
            "temporal_argument_changed",
            "temporal_pause_flag_changed",
            "temporal_process_init_state_changed",
            "temporal_process_create_phase_changed",
            "temporal_cull_type_changed",
            "temporal_demo_actor_id_changed",
            "temporal_carry_type_changed",
            "temporal_health_changed",
            "temporal_heap_present_changed",
            "temporal_model_present_changed",
            "temporal_joint_collision_present_changed",
            "temporal_attention_presence_changed",
            "temporal_event_presence_changed",
            "temporal_enemy_presence_changed",
            "temporal_trigger_presence_changed",
            "temporal_door20_presence_changed",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    names
}

fn native_base_feature_families() -> Vec<NativeEncoderChannelFamily> {
    use NativeEncoderChannelFamily as Family;
    let mut families = Vec::new();
    extend_family(&mut families, Family::CorePlayerMotion, 13);
    extend_family(&mut families, Family::CoreActionPhase, 41);
    extend_family(&mut families, Family::CoreEventContext, 8);
    extend_family(&mut families, Family::CoreEventTransition, 14);
    extend_family(&mut families, Family::CoreClockDomains, 17);
    extend_family(&mut families, Family::CoreWarpSession, 23);
    extend_family(&mut families, Family::CorePreviousInput, 24);
    extend_family(&mut families, Family::CoreCameraCollisionWorld, 22);
    extend_family(&mut families, Family::CoreRng, 12);
    extend_family(&mut families, Family::CoreGoal, 9);
    extend_family(&mut families, Family::CoreAttentionCandidates, 40);
    extend_family(&mut families, Family::CoreTemporalDelta, 25);
    families
}

fn native_actor_categorical_families() -> Vec<NativeEncoderChannelFamily> {
    use NativeEncoderChannelFamily as Family;
    let mut families = Vec::new();
    extend_family(&mut families, Family::ActorIdentity, 10);
    extend_family(&mut families, Family::ActorLifecyclePhysics, 10);
    extend_family(&mut families, Family::ActorAttention, 11);
    extend_family(&mut families, Family::ActorEventParticipation, 5);
    extend_family(&mut families, Family::ActorReturnWriter, 7);
    extend_family(&mut families, Family::ActorEnemyBase, 2);
    extend_family(&mut families, Family::ActorTriggerVolume, 3);
    extend_family(&mut families, Family::ActorDoor20, 21);
    extend_family(&mut families, Family::ActorAttentionCandidates, 6);
    families
}

fn native_actor_continuous_families() -> Vec<NativeEncoderChannelFamily> {
    use NativeEncoderChannelFamily as Family;
    let mut families = Vec::new();
    extend_family(&mut families, Family::ActorMotion, 6);
    extend_family(&mut families, Family::ActorLifecyclePhysics, 3);
    extend_family(&mut families, Family::ActorMotion, 4);
    extend_family(&mut families, Family::ActorLifecyclePhysics, 14);
    extend_family(&mut families, Family::ActorMotion, 6);
    extend_family(&mut families, Family::ActorLinkRelative, 10);
    extend_family(&mut families, Family::ActorParentRelative, 6);
    extend_family(&mut families, Family::ActorAttention, 6);
    extend_family(&mut families, Family::ActorEnemyBase, 6);
    extend_family(&mut families, Family::ActorTriggerVolume, 11);
    extend_family(&mut families, Family::ActorDoor20, 1);
    extend_family(&mut families, Family::ActorAttentionCandidates, 9);
    extend_family(&mut families, Family::ActorTemporalDelta, 16);
    families
}

fn native_actor_binary_families() -> Vec<NativeEncoderChannelFamily> {
    use NativeEncoderChannelFamily as Family;
    let mut families = Vec::new();
    extend_family(&mut families, Family::ActorLifecyclePhysics, 4);
    extend_family(&mut families, Family::ActorIdentity, 32);
    extend_family(&mut families, Family::ActorAttention, 1);
    extend_family(&mut families, Family::ActorEventParticipation, 1);
    extend_family(&mut families, Family::ActorReturnWriter, 1);
    extend_family(&mut families, Family::ActorEnemyBase, 1);
    extend_family(&mut families, Family::ActorReturnWriter, 6);
    extend_family(&mut families, Family::ActorPlayerRelationships, 11);
    extend_family(&mut families, Family::ActorTriggerVolume, 3);
    extend_family(&mut families, Family::ActorDoor20, 10);
    extend_family(&mut families, Family::ActorAttentionCandidates, 3);
    extend_family(&mut families, Family::ActorTemporalDelta, 27);
    families
}

fn extend_family(
    families: &mut Vec<NativeEncoderChannelFamily>,
    family: NativeEncoderChannelFamily,
    count: usize,
) {
    families.extend(std::iter::repeat_n(family, count));
}

fn selected_feature_names(
    names: Vec<String>,
    families: &[NativeEncoderChannelFamily],
    spec: &NativeEncoderFeatureSpec,
) -> Vec<String> {
    debug_assert_eq!(names.len(), families.len());
    names
        .into_iter()
        .zip(families)
        .filter_map(|(name, family)| spec.contains(*family).then_some(name))
        .collect()
}

fn retain_feature_families<T>(
    values: &mut Vec<T>,
    present: &mut Vec<bool>,
    families: &[NativeEncoderChannelFamily],
    spec: &NativeEncoderFeatureSpec,
) {
    debug_assert_eq!(values.len(), present.len());
    debug_assert_eq!(values.len(), families.len());
    let retained = families
        .iter()
        .map(|family| spec.contains(*family))
        .collect::<Vec<_>>();
    *values = std::mem::take(values)
        .into_iter()
        .zip(&retained)
        .filter_map(|(value, retained)| retained.then_some(value))
        .collect();
    *present = std::mem::take(present)
        .into_iter()
        .zip(retained)
        .filter_map(|(value, retained)| retained.then_some(value))
        .collect();
}

fn suppress_base_family(
    values: &mut [f32],
    present: &mut [bool],
    suppressed: NativeEncoderChannelFamily,
) {
    debug_assert_eq!(values.len(), present.len());
    debug_assert_eq!(values.len(), native_base_feature_families().len());
    for ((value, available), family) in values
        .iter_mut()
        .zip(present)
        .zip(native_base_feature_families())
    {
        if family == suppressed {
            *value = 0.0;
            *available = false;
        }
    }
}

fn retain_node_feature_families(node: &mut TypedSetNode, spec: &NativeEncoderFeatureSpec) {
    retain_feature_families(
        &mut node.categorical,
        &mut node.categorical_present,
        &native_actor_categorical_families(),
        spec,
    );
    retain_feature_families(
        &mut node.continuous,
        &mut node.continuous_present,
        &native_actor_continuous_families(),
        spec,
    );
    retain_feature_families(
        &mut node.binary,
        &mut node.binary_present,
        &native_actor_binary_families(),
        spec,
    );
}

fn extend_vec3_feature_names(names: &mut Vec<String>, prefix: &str) {
    names.extend(["x", "y", "z"].map(|axis| format!("{prefix}_{axis}")));
}

fn native_actor_nodes(
    observation: &NativeLearningObservation,
    previous: Option<&NativeLearningObservation>,
) -> Vec<TypedSetNode> {
    let actors_by_generation = observation
        .actors
        .iter()
        .map(|actor| (actor.runtime_generation, actor))
        .collect::<BTreeMap<_, _>>();
    let previous_comparable = previous.is_some_and(|previous| {
        observation.stage == previous.stage
            && observation.room == previous.room
            && observation.layer == previous.layer
    });
    let previous_actors_by_generation = previous
        .filter(|_| previous_comparable)
        .map(|observation| {
            observation
                .actors
                .iter()
                .map(|actor| (actor.runtime_generation, actor))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    observation
        .actors
        .iter()
        .map(|actor| {
            native_actor_node(
                observation,
                actor,
                &actors_by_generation,
                previous_actors_by_generation
                    .get(&actor.runtime_generation)
                    .copied(),
                previous_comparable,
            )
        })
        .collect()
}

fn attention_candidate_for(
    candidates: &[NativeAttentionCandidateObservation],
    runtime_generation: u64,
) -> Option<(usize, &NativeAttentionCandidateObservation)> {
    candidates.iter().enumerate().find(|(_, candidate)| {
        candidate.actor.actor.as_ref().is_some_and(|identity| {
            identity.present && u64::from(identity.runtime_generation) == runtime_generation
        })
    })
}

fn native_actor_node(
    observation: &NativeLearningObservation,
    actor: &NativeActorObservation,
    actors_by_generation: &BTreeMap<u64, &NativeActorObservation>,
    previous_actor: Option<&NativeActorObservation>,
    previous_observation_available: bool,
) -> TypedSetNode {
    let attention_available =
        observation.attention_candidates_status == NativeChannelStatus::Present;
    let attention = observation.attention_candidates.as_ref();
    let lock_candidate = attention.and_then(|value| {
        attention_candidate_for(&value.lock_candidates, actor.runtime_generation)
    });
    let action_candidate = attention.and_then(|value| {
        attention_candidate_for(&value.action_candidates, actor.runtime_generation)
    });
    let check_candidate = attention.and_then(|value| {
        attention_candidate_for(&value.check_candidates, actor.runtime_generation)
    });

    let mut categorical = Vec::new();
    let mut categorical_present = Vec::new();
    let mut category = |value: i64, available: bool| {
        categorical.push(if available { value } else { 0 });
        categorical_present.push(available);
    };
    for value in [
        i64::from(actor.parameters),
        i64::from(actor.status),
        i64::from(actor.actor_name),
        i64::from(actor.profile_name),
        i64::from(actor.set_id),
        i64::from(actor.home_room),
        i64::from(actor.current_room),
        i64::from(actor.group),
        i64::from(actor.argument),
        i64::from(actor.health),
    ] {
        category(value, true);
    }
    for value in [
        i64::from(actor.actor_type),
        i64::from(actor.process_subtype),
        i64::from(actor.condition),
        i64::from(actor.old_room),
        i64::from(actor.pause_flag),
        i64::from(actor.process_init_state),
        i64::from(actor.process_create_phase),
        i64::from(actor.cull_type),
        i64::from(actor.demo_actor_id),
        i64::from(actor.carry_type),
    ] {
        category(value, actor.base_state_available);
    }
    if let Some(attention) = &actor.attention {
        category(i64::from(attention.flags), true);
        for value in attention.distance_indices {
            category(i64::from(value), true);
        }
        category(i64::from(attention.auxiliary), true);
    } else {
        for _ in 0..11 {
            category(0, false);
        }
    }
    if let Some(event) = &actor.event_participation {
        for value in [
            i64::from(event.command),
            i64::from(event.condition),
            i64::from(event.event_id),
            i64::from(event.map_tool_id),
            i64::from(event.index),
        ] {
            category(value, true);
        }
    } else {
        for _ in 0..5 {
            category(0, false);
        }
    }
    if let Some(writer) = &actor.return_place_writer {
        for value in [
            i64::from(writer.save_room),
            i64::from(writer.save_point),
            i64::from(writer.switch_room),
            i64::from(writer.required_event_set),
            i64::from(writer.required_event_unset),
            i64::from(writer.required_switch_set),
            i64::from(writer.required_switch_unset),
        ] {
            category(value, true);
        }
    } else {
        for _ in 0..7 {
            category(0, false);
        }
    }
    if let Some(enemy) = &actor.enemy_base {
        category(i64::from(enemy.flags), true);
        category(i64::from(enemy.throw_mode), true);
    } else {
        category(0, false);
        category(0, false);
    }
    if let Some(trigger) = &actor.trigger_volume {
        use dusklight_evidence::native_episode_shard::{
            NativeTriggerVolumeKind as Kind, NativeTriggerVolumeShape as Shape,
        };
        category(
            match trigger.kind {
                Kind::SceneExit => 0,
                Kind::SceneExitCylinder => 1,
                Kind::EventArea => 2,
                Kind::ScriptedEvent => 3,
                Kind::MappedEvent => 4,
            },
            true,
        );
        category(
            match trigger.shape {
                Shape::Box => 0,
                Shape::EllipticCylinder => 1,
            },
            true,
        );
        category(i64::from(trigger.behavior), true);
    } else {
        for _ in 0..3 {
            category(0, false);
        }
    }
    if let Some(door) = &actor.door20 {
        for value in [
            door.kind,
            door.door_model,
            door.front_option,
            door.back_option,
            door.front_room,
            door.back_room,
            door.exit_number,
        ] {
            category(i64::from(value), true);
        }
        for switch in [
            door.front_switch,
            door.back_switch,
            door.unlock_effect_switch,
        ] {
            category(i64::from(switch), switch != u8::MAX);
        }
        for value in [
            i64::from(door.front_event),
            i64::from(door.back_event),
            i64::from(door.message_number),
            door.action as u8 as i64,
            door.active_side as u8 as i64,
            i64::from(door.event_variant),
            i64::from(door.key_type),
            i64::from(door.enemy_clear_debounce),
            door.stopper_side as u8 as i64,
            door.front_stopper_status as u8 as i64,
            door.back_stopper_status as u8 as i64,
        ] {
            category(value, true);
        }
    } else {
        for _ in 0..21 {
            category(0, false);
        }
    }
    for candidate in [lock_candidate, action_candidate, check_candidate] {
        category(
            candidate.map_or(0, |(_, value)| i64::from(value.attention_type)),
            candidate.is_some(),
        );
        category(
            candidate.map_or(0, |(rank, _)| rank as i64),
            candidate.is_some(),
        );
    }

    let mut continuous = Vec::new();
    let mut continuous_present = Vec::new();
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        actor.position,
        true,
    );
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        actor.home_position,
        true,
    );
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        actor.old_position,
        actor.base_state_available,
    );
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        actor.velocity,
        true,
    );
    push_continuous(
        &mut continuous,
        &mut continuous_present,
        actor.forward_speed,
        true,
    );
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        actor.scale,
        actor.base_state_available,
    );
    push_continuous(
        &mut continuous,
        &mut continuous_present,
        actor.gravity,
        actor.base_state_available,
    );
    push_continuous(
        &mut continuous,
        &mut continuous_present,
        actor.max_fall_speed,
        actor.base_state_available,
    );
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        actor.eye_position,
        actor.base_state_available,
    );
    for (index, angles) in [
        actor.home_angle,
        actor.old_angle,
        actor.current_angle,
        actor.shape_angle,
    ]
    .into_iter()
    .enumerate()
    {
        let available = actor.base_state_available || index >= 2;
        push_continuous3(
            &mut continuous,
            &mut continuous_present,
            angles.map(f32::from),
            available,
        );
    }
    let player_available = observation.player_present;
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        subtract3(actor.position, observation.player_position),
        player_available,
    );
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        subtract3(actor.home_position, observation.player_position),
        player_available,
    );
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        subtract3(actor.velocity, observation.player_velocity),
        player_available,
    );
    push_continuous(
        &mut continuous,
        &mut continuous_present,
        length3(subtract3(actor.position, observation.player_position)),
        player_available,
    );
    let parent = actors_by_generation.get(&u64::from(actor.parent_runtime_generation));
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        parent.map_or([0.0; 3], |parent| {
            subtract3(actor.position, parent.position)
        }),
        parent.is_some(),
    );
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        parent.map_or([0.0; 3], |parent| {
            subtract3(actor.velocity, parent.velocity)
        }),
        parent.is_some(),
    );
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        actor
            .attention
            .as_ref()
            .map_or([0.0; 3], |value| value.position),
        actor.attention.is_some(),
    );
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        actor.attention.as_ref().map_or([0.0; 3], |value| {
            subtract3(value.position, observation.player_position)
        }),
        actor.attention.is_some() && player_available,
    );
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        actor
            .enemy_base
            .as_ref()
            .map_or([0.0; 3], |value| value.down_position),
        actor.enemy_base.is_some(),
    );
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        actor
            .enemy_base
            .as_ref()
            .map_or([0.0; 3], |value| value.head_lock_position),
        actor.enemy_base.is_some(),
    );
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        actor
            .trigger_volume
            .as_ref()
            .map_or([0.0; 3], |value| value.center),
        actor.trigger_volume.is_some(),
    );
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        actor
            .trigger_volume
            .as_ref()
            .map_or([0.0; 3], |value| value.half_extent),
        actor.trigger_volume.is_some(),
    );
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        actor.trigger_volume.as_ref().map_or([0.0; 3], |value| {
            direction_yaw3(
                subtract3(value.center, observation.player_position),
                observation.player_shape_angle[1],
            )
        }),
        actor.trigger_volume.is_some() && player_available,
    );
    let trigger_yaw = actor
        .trigger_volume
        .as_ref()
        .map(|value| angle_pair(value.yaw.wrapping_sub(observation.player_shape_angle[1])));
    for component in trigger_yaw.unwrap_or([0.0; 2]) {
        push_continuous(
            &mut continuous,
            &mut continuous_present,
            component,
            trigger_yaw.is_some() && player_available,
        );
    }
    push_continuous(
        &mut continuous,
        &mut continuous_present,
        actor
            .door20
            .as_ref()
            .map_or(0.0, |door| f32::from(door.door_angle)),
        actor.door20.is_some(),
    );
    for candidate in [lock_candidate, action_candidate, check_candidate] {
        push_continuous(
            &mut continuous,
            &mut continuous_present,
            candidate.map_or(0.0, |(_, value)| value.weight),
            candidate.is_some(),
        );
        push_continuous(
            &mut continuous,
            &mut continuous_present,
            candidate.map_or(0.0, |(_, value)| value.distance),
            candidate.is_some(),
        );
        push_continuous(
            &mut continuous,
            &mut continuous_present,
            candidate.map_or(0.0, |(_, value)| f32::from(value.angle)),
            candidate.is_some(),
        );
    }
    let actor_comparable = previous_actor.is_some();
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        previous_actor.map_or([0.0; 3], |previous| {
            subtract3(actor.position, previous.position)
        }),
        actor_comparable,
    );
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        previous_actor.map_or([0.0; 3], |previous| {
            subtract3(actor.velocity, previous.velocity)
        }),
        actor_comparable,
    );
    push_continuous(
        &mut continuous,
        &mut continuous_present,
        previous_actor.map_or(0.0, |previous| actor.forward_speed - previous.forward_speed),
        actor_comparable,
    );
    for (current, previous) in [
        (
            actor.current_angle,
            previous_actor.map(|actor| actor.current_angle),
        ),
        (
            actor.shape_angle,
            previous_actor.map(|actor| actor.shape_angle),
        ),
    ] {
        push_continuous3(
            &mut continuous,
            &mut continuous_present,
            previous.map_or([0.0; 3], |previous| {
                std::array::from_fn(|index| f32::from(current[index].wrapping_sub(previous[index])))
            }),
            previous.is_some(),
        );
    }
    let attention_pair = actor
        .attention
        .as_ref()
        .zip(previous_actor.and_then(|previous| previous.attention.as_ref()));
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        attention_pair.map_or([0.0; 3], |(current, previous)| {
            subtract3(current.position, previous.position)
        }),
        attention_pair.is_some(),
    );

    let mut binary = Vec::new();
    let mut binary_present = Vec::new();
    let mut boolean = |value: bool, available: bool| {
        binary.push(value && available);
        binary_present.push(available);
    };
    boolean(actor.base_state_available, true);
    boolean(actor.heap_present, actor.base_state_available);
    boolean(actor.model_present, actor.base_state_available);
    boolean(actor.joint_collision_present, actor.base_state_available);
    for bit in 0..32 {
        boolean(actor.status & (1_u32 << bit) != 0, true);
    }
    boolean(actor.attention.is_some(), true);
    boolean(actor.event_participation.is_some(), true);
    boolean(actor.return_place_writer.is_some(), true);
    boolean(actor.enemy_base.is_some(), true);
    if let Some(writer) = &actor.return_place_writer {
        for value in [
            writer.no_telop_clear,
            writer.event_set_satisfied,
            writer.event_unset_satisfied,
            writer.switch_set_satisfied,
            writer.switch_unset_satisfied,
            writer.eligible,
        ] {
            boolean(value, true);
        }
    } else {
        for _ in 0..6 {
            boolean(false, false);
        }
    }
    let relationships_available =
        observation.player_relationships_status == NativeChannelStatus::Present;
    let relationships = observation.player_relationships.as_ref();
    let related = |identity: Option<
        &dusklight_evidence::native_episode_shard::NativeActorIdentity,
    >| {
        identity.is_some_and(|identity| {
            identity.present && u64::from(identity.runtime_generation) == actor.runtime_generation
        })
    };
    for value in [
        relationships.and_then(|value| value.targeted_actor.as_ref()),
        relationships.and_then(|value| value.ride_actor.as_ref()),
        relationships.and_then(|value| value.held_item_actor.as_ref()),
        relationships.and_then(|value| value.grabbed_actor.as_ref()),
        relationships.and_then(|value| value.thrown_boomerang_actor.as_ref()),
        relationships.and_then(|value| value.copy_rod_actor.as_ref()),
        relationships.and_then(|value| value.hookshot_roof_wait_actor.as_ref()),
        relationships.and_then(|value| value.chain_grab_actor.as_ref()),
        relationships.and_then(|value| value.attention_hint_actor.as_ref()),
        relationships.and_then(|value| value.attention_catch_actor.as_ref()),
        relationships.and_then(|value| value.attention_look_actor.as_ref()),
    ] {
        boolean(related(value), relationships_available);
    }
    boolean(actor.trigger_volume.is_some(), true);
    boolean(
        actor
            .trigger_volume
            .as_ref()
            .is_some_and(|value| value.enabled),
        actor.trigger_volume.is_some(),
    );
    boolean(
        actor
            .trigger_volume
            .as_ref()
            .is_some_and(|value| value.vertical_unbounded),
        actor.trigger_volume.is_some(),
    );
    boolean(actor.door20.is_some(), true);
    if let Some(door) = &actor.door20 {
        boolean(door.message_door, true);
        for (switch, set) in [
            (door.front_switch, door.front_switch_set),
            (door.back_switch, door.back_switch_set),
            (door.unlock_effect_switch, door.unlock_effect_switch_set),
        ] {
            boolean(set, switch != u8::MAX);
        }
        for value in [
            door.locked,
            door.background_collision_released,
            door.unlock_effect_triggered,
            door.opening_active,
            door.closing_active,
        ] {
            boolean(value, true);
        }
    } else {
        for _ in 0..9 {
            boolean(false, false);
        }
    }
    for candidate in [lock_candidate, action_candidate, check_candidate] {
        boolean(candidate.is_some(), attention_available);
    }
    boolean(previous_actor.is_some(), previous_observation_available);
    let previous_available = previous_actor.is_some();
    let changed = previous_actor.map(|previous| {
        [
            actor.base_state_available != previous.base_state_available,
            actor.actor_type != previous.actor_type,
            actor.process_subtype != previous.process_subtype,
            actor.parameters != previous.parameters,
            actor.status != previous.status,
            actor.condition != previous.condition,
            actor.home_room != previous.home_room,
            actor.old_room != previous.old_room,
            actor.current_room != previous.current_room,
            actor.group != previous.group,
            actor.argument != previous.argument,
            actor.pause_flag != previous.pause_flag,
            actor.process_init_state != previous.process_init_state,
            actor.process_create_phase != previous.process_create_phase,
            actor.cull_type != previous.cull_type,
            actor.demo_actor_id != previous.demo_actor_id,
            actor.carry_type != previous.carry_type,
            actor.health != previous.health,
            actor.heap_present != previous.heap_present,
            actor.model_present != previous.model_present,
            actor.joint_collision_present != previous.joint_collision_present,
            actor.attention.is_some() != previous.attention.is_some(),
            actor.event_participation.is_some() != previous.event_participation.is_some(),
            actor.enemy_base.is_some() != previous.enemy_base.is_some(),
            actor.trigger_volume.is_some() != previous.trigger_volume.is_some(),
            actor.door20.is_some() != previous.door20.is_some(),
        ]
    });
    for value in changed.unwrap_or([false; 26]) {
        boolean(value, previous_available);
    }
    debug_assert_eq!(categorical.len(), native_actor_categorical_names().len());
    debug_assert_eq!(continuous.len(), native_actor_continuous_names().len());
    debug_assert_eq!(binary.len(), native_actor_binary_names().len());
    TypedSetNode {
        stable_id: actor.runtime_generation,
        categorical,
        categorical_present,
        continuous,
        continuous_present,
        binary,
        binary_present,
    }
}

fn subtract3(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [left[0] - right[0], left[1] - right[1], left[2] - right[2]]
}

fn push_continuous(values: &mut Vec<f32>, present: &mut Vec<bool>, value: f32, available: bool) {
    values.push(if available { value } else { 0.0 });
    present.push(available);
}

fn push_continuous3(
    values: &mut Vec<f32>,
    present: &mut Vec<bool>,
    value: [f32; 3],
    available: bool,
) {
    for component in value {
        push_continuous(values, present, component, available);
    }
}

fn length3(value: [f32; 3]) -> f32 {
    value
        .iter()
        .map(|component| component * component)
        .sum::<f32>()
        .sqrt()
}

fn direction_yaw3(direction: [f32; 3], yaw: i16) -> [f32; 3] {
    let radians = f32::from(yaw) * std::f32::consts::PI / 32768.0;
    let (sin, cos) = radians.sin_cos();
    [
        cos * direction[0] - sin * direction[2],
        direction[1],
        sin * direction[0] + cos * direction[2],
    ]
}

fn angle_pair(angle: i16) -> [f32; 2] {
    let radians = f32::from(angle) * std::f32::consts::PI / 32768.0;
    [radians.sin(), radians.cos()]
}

fn native_target_names() -> Vec<String> {
    [
        "player_position_delta_x",
        "player_position_delta_y",
        "player_position_delta_z",
        "player_velocity_delta_x",
        "player_velocity_delta_y",
        "player_velocity_delta_z",
        "player_forward_speed_delta",
        "contact_changed",
        "procedure_changed",
        "mode_flags_changed",
        "actor_disappearance_occurred",
        "actor_disappearance_count",
        "inverse_stick_x",
        "inverse_stick_y",
        "inverse_button_0x0100",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn native_target_conditioning() -> Vec<AuxiliaryHeadConditioning> {
    let mut conditioning = vec![AuxiliaryHeadConditioning::PreStateAndAction; 12];
    conditioning.extend([AuxiliaryHeadConditioning::PreAndPostState; 3]);
    conditioning
}

fn target_objectives_for_names(names: &[String]) -> Vec<AuxiliaryHeadObjective> {
    names
        .iter()
        .map(|name| {
            if matches!(
                name.as_str(),
                "contact_changed"
                    | "procedure_changed"
                    | "mode_flags_changed"
                    | "actor_disappearance_occurred"
                    | "inverse_button_0x0100"
            ) {
                AuxiliaryHeadObjective::ClassBalancedBernoulli
            } else {
                AuxiliaryHeadObjective::NormalizedRegression
            }
        })
        .collect()
}

fn target_conditioning_for_names(names: &[String]) -> Vec<AuxiliaryHeadConditioning> {
    names
        .iter()
        .map(|name| {
            if name.starts_with("inverse_") {
                AuxiliaryHeadConditioning::PreAndPostState
            } else {
                AuxiliaryHeadConditioning::PreStateAndAction
            }
        })
        .collect()
}

fn native_action_context(example: &NativeAuxiliaryExample) -> Vec<f32> {
    let action = example.targets.inverse_action;
    pad_action_context(
        action.buttons,
        action.stick_x,
        action.stick_y,
        action.substick_x,
        action.substick_y,
        action.trigger_left,
        action.trigger_right,
        action.analog_a,
        action.analog_b,
    )
}

fn episode_history_action_context(action: &EpisodeHistoryPad) -> Vec<f32> {
    pad_action_context(
        action.buttons,
        action.stick_x,
        action.stick_y,
        action.substick_x,
        action.substick_y,
        action.trigger_left,
        action.trigger_right,
        action.analog_a,
        action.analog_b,
    )
}

#[allow(clippy::too_many_arguments)]
fn pad_action_context(
    buttons: u16,
    stick_x: i8,
    stick_y: i8,
    substick_x: i8,
    substick_y: i8,
    trigger_left: u8,
    trigger_right: u8,
    analog_a: u8,
    analog_b: u8,
) -> Vec<f32> {
    let mut context = [stick_x, stick_y, substick_x, substick_y]
        .map(|value| f32::from(value) / 128.0)
        .to_vec();
    context.extend(
        [trigger_left, trigger_right, analog_a, analog_b].map(|value| f32::from(value) / 255.0),
    );
    context.extend((0..16).map(|bit| f32::from(buttons & (1_u16 << bit) != 0)));
    debug_assert_eq!(context.len(), ACTION_CONTEXT_WIDTH);
    context
}

fn append_episode_history_features(
    values: &mut Vec<f32>,
    present: &mut Vec<bool>,
    episode: &NativeEpisode,
    completed: &[&EpisodeHistoryTransition],
    spec: &NativeEncoderFeatureSpec,
) -> Result<(), TrainableSetError> {
    if completed.len() > spec.history_depth {
        return Err(TrainableSetError::new(
            "native episode history exceeds the declared feature depth",
        ));
    }
    let core_width = selected_feature_names(
        native_base_feature_names(),
        &native_base_feature_families(),
        spec,
    )
    .len();
    let missing = spec.history_depth - completed.len();
    for _ in 0..missing {
        values.push(0.0);
        present.push(true);
        values.extend(std::iter::repeat_n(0.0, ACTION_CONTEXT_WIDTH + core_width));
        present.extend(std::iter::repeat_n(
            false,
            ACTION_CONTEXT_WIDTH + core_width,
        ));
    }
    for transition in completed {
        if transition.episode_id != episode.id {
            return Err(TrainableSetError::new(
                "native episode history crosses an episode boundary",
            ));
        }
        let step = episode
            .steps
            .get(transition.step_index as usize)
            .ok_or_else(|| TrainableSetError::new("native episode history step is absent"))?;
        values.push(1.0);
        present.push(true);
        values.extend(episode_history_action_context(&transition.consumed_pad));
        present.extend(std::iter::repeat_n(true, ACTION_CONTEXT_WIDTH));

        let (mut state, mut state_present) = broad_base(&step.post_simulation);
        append_core_temporal_features(
            &mut state,
            &mut state_present,
            &step.post_simulation,
            Some(&step.pre_input),
        );
        retain_feature_families(
            &mut state,
            &mut state_present,
            &native_base_feature_families(),
            spec,
        );
        if state.len() != core_width || state_present.len() != core_width {
            return Err(TrainableSetError::new(
                "native episode history core width is inconsistent",
            ));
        }
        values.extend(state);
        present.extend(state_present);
    }
    Ok(())
}

fn trainable_episode_history_steps(
    episode: &NativeEpisode,
    completed: &[&EpisodeHistoryTransition],
    spec: &NativeEncoderFeatureSpec,
    actor_feature_schema_sha256: Digest,
    states: &mut BTreeMap<(String, u32), Arc<TypedSetSample>>,
) -> Result<Vec<MultiTaskHistoryStep>, TrainableSetError> {
    if spec.history_encoding != NativeEncoderHistoryEncoding::TrainableGru {
        return Ok(Vec::new());
    }
    if completed.len() > spec.history_depth {
        return Err(TrainableSetError::new(
            "native trainable history exceeds the declared feature depth",
        ));
    }
    completed
        .iter()
        .map(|transition| {
            if transition.episode_id != episode.id {
                return Err(TrainableSetError::new(
                    "native trainable history crosses an episode boundary",
                ));
            }
            let transition_sha256 = canonical_digest(
                b"dusklight.native-trainable-history-transition/v1\0",
                transition,
            )?;
            let key = (transition.episode_id.clone(), transition.step_index);
            let state = if let Some(state) = states.get(&key) {
                Arc::clone(state)
            } else {
                let step = episode
                    .steps
                    .get(transition.step_index as usize)
                    .ok_or_else(|| {
                        TrainableSetError::new("native trainable history step is absent")
                    })?;
                let (mut base, mut base_present) = broad_base(&step.post_simulation);
                append_core_temporal_features(
                    &mut base,
                    &mut base_present,
                    &step.post_simulation,
                    Some(&step.pre_input),
                );
                suppress_base_family(
                    &mut base,
                    &mut base_present,
                    NativeEncoderChannelFamily::CorePreviousInput,
                );
                retain_feature_families(
                    &mut base,
                    &mut base_present,
                    &native_base_feature_families(),
                    spec,
                );
                let mut nodes = if spec.contains(NativeEncoderChannelFamily::ActorPopulation) {
                    native_actor_nodes(&step.post_simulation, Some(&step.pre_input))
                } else {
                    Vec::new()
                };
                for node in &mut nodes {
                    retain_node_feature_families(node, spec);
                }
                let sample_sha256 = canonical_digest(
                    b"dusklight.native-trainable-history-state/v1\0",
                    &(
                        transition_sha256,
                        hex_128(step.post_simulation.state_identity),
                        actor_feature_schema_sha256,
                    ),
                )?;
                let state = Arc::new(TypedSetSample {
                    sample_sha256,
                    actor_feature_schema_sha256,
                    base,
                    base_present,
                    nodes,
                    target: 0.0,
                });
                states.insert(key, Arc::clone(&state));
                state
            };
            Ok(MultiTaskHistoryStep {
                transition_sha256,
                state,
                action_context: episode_history_action_context(&transition.consumed_pad),
            })
        })
        .collect()
}

fn append_encoded_episode_history_features(
    values: &mut Vec<f32>,
    present: &mut Vec<bool>,
    episode: &NativeEpisode,
    completed: &[&EpisodeHistoryTransition],
    spec: &NativeEncoderFeatureSpec,
    recurrent_reservoir: Option<&Reservoir>,
) -> Result<(), TrainableSetError> {
    match spec.history_encoding {
        NativeEncoderHistoryEncoding::None => {
            if !completed.is_empty() {
                return Err(TrainableSetError::new(
                    "native episode history is present for a history-free feature spec",
                ));
            }
            Ok(())
        }
        NativeEncoderHistoryEncoding::Stacked => {
            append_episode_history_features(values, present, episode, completed, spec)
        }
        NativeEncoderHistoryEncoding::RecurrentReservoir => {
            append_recurrent_episode_history_features(
                values,
                present,
                episode,
                completed,
                spec,
                recurrent_reservoir.ok_or_else(|| {
                    TrainableSetError::new("native recurrent history reservoir is absent")
                })?,
            )
        }
        NativeEncoderHistoryEncoding::TrainableGru => Ok(()),
    }
}

fn native_recurrent_history_input_width(
    spec: &NativeEncoderFeatureSpec,
) -> Result<usize, TrainableSetError> {
    let core_width = selected_feature_names(
        native_base_feature_names(),
        &native_base_feature_families(),
        spec,
    )
    .len();
    ACTION_CONTEXT_WIDTH
        .checked_add(core_width.checked_mul(2).ok_or_else(|| {
            TrainableSetError::new("native recurrent history input width overflowed")
        })?)
        .ok_or_else(|| TrainableSetError::new("native recurrent history input width overflowed"))
}

fn native_recurrent_history_reservoir(
    spec: &NativeEncoderFeatureSpec,
) -> Result<Option<Reservoir>, TrainableSetError> {
    if spec.history_encoding != NativeEncoderHistoryEncoding::RecurrentReservoir {
        return Ok(None);
    }
    Ok(Some(Reservoir::new(
        native_recurrent_history_input_width(spec)?,
        spec.history_recurrent_width,
        HISTORY_RESERVOIR_SEED,
    )))
}

fn append_recurrent_episode_history_features(
    values: &mut Vec<f32>,
    present: &mut Vec<bool>,
    episode: &NativeEpisode,
    completed: &[&EpisodeHistoryTransition],
    spec: &NativeEncoderFeatureSpec,
    reservoir: &Reservoir,
) -> Result<(), TrainableSetError> {
    if completed.len() > spec.history_depth {
        return Err(TrainableSetError::new(
            "native episode history exceeds the declared feature depth",
        ));
    }
    let core_width = selected_feature_names(
        native_base_feature_names(),
        &native_base_feature_families(),
        spec,
    )
    .len();
    let input_width = native_recurrent_history_input_width(spec)?;
    let mut hidden = vec![0.0; spec.history_recurrent_width];
    for transition in completed {
        if transition.episode_id != episode.id {
            return Err(TrainableSetError::new(
                "native episode history crosses an episode boundary",
            ));
        }
        let step = episode
            .steps
            .get(transition.step_index as usize)
            .ok_or_else(|| TrainableSetError::new("native episode history step is absent"))?;
        let mut input = episode_history_action_context(&transition.consumed_pad);
        let (mut state, mut state_present) = broad_base(&step.post_simulation);
        append_core_temporal_features(
            &mut state,
            &mut state_present,
            &step.post_simulation,
            Some(&step.pre_input),
        );
        retain_feature_families(
            &mut state,
            &mut state_present,
            &native_base_feature_families(),
            spec,
        );
        if state.len() != core_width || state_present.len() != core_width {
            return Err(TrainableSetError::new(
                "native recurrent history core width is inconsistent",
            ));
        }
        input.extend(state.iter().zip(&state_present).map(|(value, available)| {
            if *available {
                (value.signum() * value.abs().ln_1p() / 32.0).clamp(-1.0, 1.0)
            } else {
                0.0
            }
        }));
        input.extend(state_present.iter().map(|available| f32::from(*available)));
        if input.len() != input_width {
            return Err(TrainableSetError::new(
                "native recurrent history observation width is inconsistent",
            ));
        }
        let input_scale = (input_width as f32).sqrt().recip();
        input.iter_mut().for_each(|value| *value *= input_scale);
        hidden = reservoir.step(&input, &hidden);
    }

    values.push(f32::from(!completed.is_empty()));
    values.push(completed.len() as f32 / spec.history_depth as f32);
    values.extend(hidden.into_iter().map(|value| value as f32));
    present.extend(std::iter::repeat_n(true, 2 + spec.history_recurrent_width));
    Ok(())
}

fn native_targets(example: &NativeAuxiliaryExample) -> (Vec<f32>, Vec<bool>) {
    let mut targets = vec![0.0; 15];
    let mut present = vec![false; 15];
    if let Some(dynamics) = &example.targets.player_dynamics {
        targets[..3].copy_from_slice(&dynamics.position_delta);
        targets[3..6].copy_from_slice(&dynamics.velocity_delta);
        targets[6] = dynamics.forward_speed_delta;
        present[..7].fill(true);
    }
    if let Some(contacts) = &example.targets.contacts {
        targets[7] = f32::from(contacts.activated != 0 || contacts.cleared != 0);
        present[7] = true;
    }
    if let Some(action) = &example.targets.action_phase {
        targets[8] = f32::from(action.procedure_before != action.procedure_after);
        targets[9] = f32::from(action.mode_flags_activated != 0 || action.mode_flags_cleared != 0);
        present[8..10].fill(true);
    }
    if let Some(lifecycle) = &example.targets.actor_lifecycle {
        let count = lifecycle.disappeared_runtime_generations.len();
        targets[10] = f32::from(count != 0);
        targets[11] = count as f32;
        present[10..12].fill(true);
    }
    targets[12] = f32::from(example.targets.inverse_action.stick_x);
    targets[13] = f32::from(example.targets.inverse_action.stick_y);
    targets[14] = f32::from(example.targets.inverse_action.buttons & 0x0100 != 0);
    present[12..].fill(true);
    (targets, present)
}

fn broad_base(observation: &NativeLearningObservation) -> (Vec<f32>, Vec<bool>) {
    let mut values = Vec::new();
    let mut present = Vec::new();
    let mut push = |value: f32, available: bool| {
        values.push(if available { value } else { 0.0 });
        present.push(available);
    };
    for value in observation.player_position {
        push(value, observation.player_present);
    }
    for value in observation.player_velocity {
        push(value, observation.player_present);
    }
    push(observation.player_forward_speed, observation.player_present);
    for angle in observation
        .player_current_angle
        .into_iter()
        .chain(observation.player_shape_angle)
    {
        push(f32::from(angle), observation.player_present);
    }
    push(
        f32::from(observation.player_procedure),
        observation.player_present,
    );
    for bit in 0..32 {
        push(
            f32::from(observation.player_mode_flags & (1_u32 << bit) != 0),
            observation.player_present,
        );
    }
    for bit in 0..8 {
        push(
            f32::from(observation.player_contacts & (1_u8 << bit) != 0),
            observation.player_present,
        );
    }
    push(f32::from(observation.event_running), true);
    push(f32::from(observation.event_id), true);
    push(f32::from(observation.event_mode), true);
    push(f32::from(observation.event_status), true);
    push(f32::from(observation.event_map_tool_id), true);
    push(f32::from(observation.room), true);
    push(f32::from(observation.layer), true);
    push(f32::from(observation.point), true);
    let transition = observation.event_transition.as_ref();
    push(
        transition.map_or(0.0, |value| f32::from(value.event_data_loaded)),
        transition.is_some(),
    );
    push(
        transition.map_or(0.0, |value| value.camera_play as f32),
        transition.is_some(),
    );
    let current_event = transition.and_then(|value| value.current_event.as_ref());
    push(
        current_event.map_or(0.0, |value| f32::from(value.event_id)),
        current_event.is_some(),
    );
    push(
        current_event.map_or(0.0, |value| value.event_type as f32),
        current_event.is_some(),
    );
    push(
        current_event.map_or(0.0, |value| value.room as f32),
        current_event.is_some(),
    );
    for index in 0..3 {
        push(
            current_event.map_or(0.0, |value| value.goal[index]),
            current_event.is_some(),
        );
    }
    let pending_stage = transition.and_then(|value| value.pending_stage.as_ref());
    push(f32::from(pending_stage.is_some()), transition.is_some());
    push(
        pending_stage.map_or(0.0, |value| f32::from(value.room)),
        pending_stage.is_some(),
    );
    push(
        pending_stage.map_or(0.0, |value| f32::from(value.layer)),
        pending_stage.is_some(),
    );
    push(
        pending_stage.map_or(0.0, |value| f32::from(value.point)),
        pending_stage.is_some(),
    );
    push(
        pending_stage.map_or(0.0, |value| f32::from(value.wipe)),
        pending_stage.is_some(),
    );
    push(
        pending_stage.map_or(0.0, |value| f32::from(value.wipe_speed)),
        pending_stage.is_some(),
    );
    let clocks = observation.clock_domains.as_ref();
    let clocks_present = clocks.is_some();
    push(
        clocks.map_or(0.0, |value| value.framework_frames as f32),
        clocks_present,
    );
    push(
        clocks.map_or(0.0, |value| value.gameplay_frames as f32),
        clocks_present,
    );
    push(
        clocks.map_or(0.0, |value| f32::from(value.global_pause)),
        clocks_present,
    );
    push(
        clocks.map_or(0.0, |value| f32::from(value.scene_paused)),
        clocks_present,
    );
    push(
        clocks.map_or(0.0, |value| value.scene_pause_timer as f32),
        clocks_present,
    );
    push(
        clocks.map_or(0.0, |value| value.scene_next_pause_timer as f32),
        clocks_present,
    );
    push(
        clocks.map_or(0.0, |value| f32::from(value.overlap_request_active)),
        clocks_present,
    );
    push(
        clocks.map_or(0.0, |value| f32::from(value.overlap_fadeout_peek)),
        clocks_present,
    );
    let demo_present =
        clocks.is_some_and(|value| value.demo_status == NativeChannelStatus::Present);
    push(f32::from(demo_present), clocks_present);
    push(
        clocks.map_or(0.0, |value| value.demo_mode as f32),
        demo_present,
    );
    push(
        clocks.map_or(0.0, |value| value.demo_frame as f32),
        demo_present,
    );
    push(
        clocks.map_or(0.0, |value| value.demo_frame_no_message as f32),
        demo_present,
    );
    push(
        clocks.map_or(0.0, |value| value.demo_flags as f32),
        demo_present,
    );
    let timer_present =
        clocks.is_some_and(|value| value.timer_status == NativeChannelStatus::Present);
    push(f32::from(timer_present), clocks_present);
    push(
        clocks.map_or(0.0, |value| value.timer_mode as f32),
        timer_present,
    );
    push(
        clocks.map_or(0.0, |value| value.timer_now_ms as f32),
        timer_present,
    );
    push(
        clocks.map_or(0.0, |value| value.timer_limit_ms as f32),
        timer_present,
    );
    let warp = observation.warp_session.as_ref();
    let warp_present = warp.is_some();
    push(
        warp.map_or(0.0, |value| f32::from(value.request_kind)),
        warp_present,
    );
    let selection = warp.and_then(|value| value.selection.as_ref());
    push(f32::from(selection.is_some()), warp_present);
    for index in 0..3 {
        push(
            selection.map_or(0.0, |value| value.position[index]),
            selection.is_some(),
        );
    }
    push(
        selection.map_or(0.0, |value| f32::from(value.angle)),
        selection.is_some(),
    );
    push(
        selection.map_or(0.0, |value| f32::from(value.room)),
        selection.is_some(),
    );
    push(
        selection.map_or(0.0, |value| f32::from(value.parameter)),
        selection.is_some(),
    );
    push(
        selection.map_or(0.0, |value| f32::from(value.player)),
        selection.is_some(),
    );
    push(
        selection.map_or(0.0, |value| f32::from(value.stage == observation.stage)),
        selection.is_some(),
    );
    let return_mark = warp.and_then(|value| value.return_mark.as_ref());
    push(f32::from(return_mark.is_some()), warp_present);
    for index in 0..3 {
        push(
            return_mark.map_or(0.0, |value| value.position[index]),
            return_mark.is_some(),
        );
    }
    push(
        return_mark.map_or(0.0, |value| f32::from(value.angle)),
        return_mark.is_some(),
    );
    push(
        return_mark.map_or(0.0, |value| f32::from(value.room)),
        return_mark.is_some(),
    );
    push(
        return_mark.map_or(0.0, |value| f32::from(value.accept_stage)),
        return_mark.is_some(),
    );
    push(
        return_mark.map_or(0.0, |value| f32::from(value.stage == observation.stage)),
        return_mark.is_some(),
    );
    let target_point = warp.and_then(|value| value.target_point);
    push(f32::from(target_point.is_some()), warp_present);
    push(target_point.map_or(0.0, f32::from), target_point.is_some());
    let selected_point = warp.and_then(|value| value.selected_point);
    push(f32::from(selected_point.is_some()), warp_present);
    push(
        selected_point.map_or(0.0, f32::from),
        selected_point.is_some(),
    );
    push(
        warp.map_or(0.0, |value| f32::from(value.transport_match)),
        warp_present,
    );
    for value in [
        observation.previous_input.stick_x,
        observation.previous_input.stick_y,
        observation.previous_input.substick_x,
        observation.previous_input.substick_y,
    ] {
        push(f32::from(value), true);
    }
    for value in [
        observation.previous_input.trigger_left,
        observation.previous_input.trigger_right,
        observation.previous_input.analog_a,
        observation.previous_input.analog_b,
    ] {
        push(f32::from(value), true);
    }
    for bit in 0..16 {
        push(
            f32::from(observation.previous_input.buttons & (1_u16 << bit) != 0),
            true,
        );
    }
    push(
        observation.camera_yaw_radians.unwrap_or(0.0),
        observation.camera_yaw_radians.is_some(),
    );
    for index in 0..9 {
        let camera_value = observation.camera.as_ref().map(|camera| match index {
            0 => f32::from(camera.view_yaw),
            1 => f32::from(camera.controlled_yaw),
            2 => f32::from(camera.bank),
            3..=5 => camera.eye[index - 3],
            _ => camera.center[index - 6],
        });
        push(camera_value.unwrap_or(0.0), camera_value.is_some());
    }
    push(
        observation.player_ground_height.unwrap_or(0.0),
        observation.player_ground_height.is_some(),
    );
    push(
        observation.player_roof_height.unwrap_or(0.0),
        observation.player_roof_height.is_some(),
    );
    let water_height = observation
        .player_background_collision
        .as_ref()
        .map(|collision| collision.water_height);
    push(water_height.unwrap_or(0.0), water_height.is_some());
    for index in 0..2 {
        let correction = observation.collision_correction.map(|value| value[index]);
        push(correction.unwrap_or(0.0), correction.is_some());
    }
    for index in 0..7 {
        let scene = observation.scene_exit.as_ref().map(|exit| match index {
            0 => exit.signed_distance_to_volume,
            1..=3 => exit.player_local_position[index - 1],
            _ => exit.volume_extent[index - 4],
        });
        push(scene.unwrap_or(0.0), scene.is_some());
    }
    for stream_index in 0..2 {
        let stream = observation.rng_streams.get(stream_index);
        push(
            stream.map_or(0.0, |value| f32::from(value.id)),
            stream.is_some(),
        );
        for state_index in 0..3 {
            push(
                stream.map_or(0.0, |value| value.state[state_index] as f32),
                stream.is_some(),
            );
        }
        push(
            stream.map_or(0.0, |value| value.call_count as f32),
            stream.is_some(),
        );
    }
    push(observation.rng_streams.len() as f32, true);
    push(observation.rng_streams.len().saturating_sub(2) as f32, true);
    for value in [
        observation.goal.requested_count,
        observation.goal.hit_count,
        observation.goal.stable_ticks,
        observation.goal.consecutive_ticks,
        u16::from(observation.goal.sequence_steps),
        u16::from(observation.goal.sequence_next_step),
        observation.goal.sequence_within_ticks,
        observation.goal.sequence_elapsed_ticks,
    ] {
        push(f32::from(value), observation.goal.configured);
    }
    push(
        f32::from(observation.goal.reached),
        observation.goal.configured,
    );
    let attention_available =
        observation.attention_candidates_status == NativeChannelStatus::Present;
    let attention = observation.attention_candidates.as_ref();
    for bit in 0..32 {
        push(
            attention.map_or(0.0, |value| {
                f32::from(value.player_attention_flags & (1_u32 << bit) != 0)
            }),
            attention_available,
        );
    }
    for value in [
        attention.map_or(0.0, |value| f32::from(value.attention_status)),
        attention.map_or(0.0, |value| value.attention_block_timer as f32),
        attention.map_or(0.0, |value| value.lock_candidates.len() as f32),
        attention.map_or(0.0, |value| f32::from(value.lock_offset)),
        attention.map_or(0.0, |value| value.action_candidates.len() as f32),
        attention.map_or(0.0, |value| f32::from(value.action_offset)),
        attention.map_or(0.0, |value| value.check_candidates.len() as f32),
        attention.map_or(0.0, |value| f32::from(value.check_offset)),
    ] {
        push(value, attention_available);
    }
    (values, present)
}

fn append_core_temporal_features(
    values: &mut Vec<f32>,
    present: &mut Vec<bool>,
    current: &NativeLearningObservation,
    previous: Option<&NativeLearningObservation>,
) {
    let comparable = previous.is_some_and(|previous| {
        current.player_present
            && previous.player_present
            && current.player_is_link == previous.player_is_link
            && current.stage == previous.stage
            && current.room == previous.room
            && current.layer == previous.layer
    });
    let player_delta = |current: [f32; 3], select: fn(&NativeLearningObservation) -> [f32; 3]| {
        previous
            .filter(|_| comparable)
            .map_or([0.0; 3], |observation| {
                subtract3(current, select(observation))
            })
    };
    push_continuous3(
        values,
        present,
        player_delta(current.player_position, |value| value.player_position),
        comparable,
    );
    push_continuous3(
        values,
        present,
        player_delta(current.player_velocity, |value| value.player_velocity),
        comparable,
    );
    push_continuous(
        values,
        present,
        previous.map_or(0.0, |previous| {
            current.player_forward_speed - previous.player_forward_speed
        }),
        comparable,
    );
    let camera_pair = current
        .camera_yaw_radians
        .zip(previous.and_then(|previous| previous.camera_yaw_radians));
    push_continuous(
        values,
        present,
        camera_pair.map_or(0.0, |(current, previous)| current - previous),
        camera_pair.is_some() && comparable,
    );
    for (current_height, previous_height) in [
        (
            current.player_ground_height,
            previous.and_then(|value| value.player_ground_height),
        ),
        (
            current.player_roof_height,
            previous.and_then(|value| value.player_roof_height),
        ),
    ] {
        let pair = current_height.zip(previous_height);
        push_continuous(
            values,
            present,
            pair.map_or(0.0, |(current, previous)| current - previous),
            pair.is_some() && comparable,
        );
    }
    push_continuous(values, present, f32::from(previous.is_some()), true);
    push_continuous(values, present, f32::from(comparable), true);
    push_continuous(
        values,
        present,
        previous.map_or(0.0, |previous| {
            f32::from(current.player_procedure != previous.player_procedure)
        }),
        comparable,
    );
    push_continuous(
        values,
        present,
        previous.map_or(0.0, |previous| {
            f32::from(current.player_mode_flags != previous.player_mode_flags)
        }),
        comparable,
    );
    for bit in 0..8 {
        push_continuous(
            values,
            present,
            previous.map_or(0.0, |previous| {
                f32::from((current.player_contacts ^ previous.player_contacts) & (1_u8 << bit) != 0)
            }),
            comparable,
        );
    }
    push_continuous(
        values,
        present,
        previous.map_or(0.0, |previous| {
            f32::from(current.event_running != previous.event_running)
        }),
        previous.is_some(),
    );
    push_continuous(
        values,
        present,
        previous.map_or(0.0, |previous| {
            f32::from(current.event_id != previous.event_id)
        }),
        previous.is_some(),
    );
    push_continuous(
        values,
        present,
        previous.map_or(0.0, |previous| {
            f32::from(
                current.stage != previous.stage
                    || current.room != previous.room
                    || current.layer != previous.layer
                    || current.point != previous.point,
            )
        }),
        previous.is_some(),
    );
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
    canonical_digest(b"dusklight.multitask-set-encoder-report/v10\0", &canonical)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trainable_set_encoder::TypedSetNode;

    fn sample(identity: u8, first: f32, second: f32, reverse: bool) -> MultiTaskSetSample {
        let mut nodes = vec![
            TypedSetNode {
                stable_id: 1,
                categorical: vec![10],
                categorical_present: vec![true],
                continuous: vec![first],
                continuous_present: vec![true],
                binary: vec![first > 0.0],
                binary_present: vec![true],
            },
            TypedSetNode {
                stable_id: 2,
                categorical: vec![20],
                categorical_present: vec![true],
                continuous: vec![second],
                continuous_present: vec![true],
                binary: vec![second > 0.0],
                binary_present: vec![true],
            },
        ];
        if reverse {
            nodes.reverse();
        }
        let second_present = !identity.is_multiple_of(5);
        let mut post_nodes = nodes.clone();
        for node in &mut post_nodes {
            node.continuous[0] += first - second;
        }
        let post_sample_sha256 =
            canonical_digest(b"dusklight.synthetic-multitask-post/v1\0", &identity).unwrap();
        let mut action_context = vec![0.0; ACTION_CONTEXT_WIDTH];
        action_context[0] = first;
        action_context[1] = second;
        MultiTaskSetSample {
            input: TypedSetSample {
                sample_sha256: Digest([identity; 32]),
                actor_feature_schema_sha256: Digest([7; 32]),
                base: vec![first - second],
                base_present: vec![true],
                nodes,
                target: 0.0,
            },
            post_input: TypedSetSample {
                sample_sha256: post_sample_sha256,
                actor_feature_schema_sha256: Digest([7; 32]),
                base: vec![first + second],
                base_present: vec![true],
                nodes: post_nodes,
                target: 0.0,
            },
            history: Vec::new(),
            action_context,
            targets: vec![
                first + second,
                if second_present { first - second } else { 0.0 },
            ],
            target_present: vec![true, second_present],
        }
    }

    fn corpus(start: u8, count: usize) -> Vec<MultiTaskSetSample> {
        (0..count)
            .map(|index| {
                let first = ((index * 17 % 41) as f32 - 20.0) / 10.0;
                let second = ((index * 29 % 37) as f32 - 18.0) / 10.0;
                sample(start + index as u8, first, second, index % 2 == 0)
            })
            .collect()
    }

    #[test]
    fn direct_native_adapter_exposes_generic_event_transition_with_masks() {
        let shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v22.dseps"
        ))
        .unwrap();
        let observation = &shard.episodes[0].steps[0].pre_input;
        let (base, present) = broad_base(observation);
        assert_eq!(
            &base[62..76],
            &[
                1.0, 2.0, 291.0, 1.0, 0.0, 10.0, 20.0, 30.0, 1.0, 2.0, 1.0, 3.0, 5.0, 2.0
            ]
        );
        assert!(present[62..76].iter().all(|value| *value));

        let mut base = base;
        let mut present = present;
        append_core_temporal_features(&mut base, &mut present, observation, None);

        let reduced =
            NativeEncoderFeatureSpec::excluding([NativeEncoderChannelFamily::CoreEventTransition])
                .unwrap();
        let mut reduced_values = base;
        let mut reduced_present = present;
        retain_feature_families(
            &mut reduced_values,
            &mut reduced_present,
            &native_base_feature_families(),
            &reduced,
        );
        assert_eq!(reduced_values.len(), 234);
        assert_eq!(reduced_present.len(), 234);
    }

    #[test]
    fn direct_native_adapter_exposes_generic_clock_domains_with_masks() {
        let shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v23.dseps"
        ))
        .unwrap();
        let observation = &shard.episodes[0].steps[0].pre_input;
        let (base, present) = broad_base(observation);
        assert_eq!(
            &base[76..93],
            &[
                1000.0, 900.0, 0.0, 1.0, 1.0, 2.0, 1.0, 0.0, 1.0, 1.0, 40.0, 35.0, 3.0, 1.0, 4.0,
                1234.0, 5000.0,
            ]
        );
        assert!(present[76..93].iter().all(|value| *value));

        let mut base = base;
        let mut present = present;
        append_core_temporal_features(&mut base, &mut present, observation, None);
        let reduced =
            NativeEncoderFeatureSpec::excluding([NativeEncoderChannelFamily::CoreClockDomains])
                .unwrap();
        retain_feature_families(
            &mut base,
            &mut present,
            &native_base_feature_families(),
            &reduced,
        );
        assert_eq!(base.len(), 231);
        assert_eq!(present.len(), 231);
    }

    #[test]
    fn direct_native_adapter_exposes_generic_warp_session_with_masks() {
        let shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v25.dseps"
        ))
        .unwrap();
        let observation = &shard.episodes[0].steps[0].pre_input;
        let (base, present) = broad_base(observation);
        assert_eq!(
            &base[93..116],
            &[
                3.0, 1.0, 100.0, 200.0, -300.0, 4608.0, 2.0, 4.0, 1.0, 0.0, 1.0, 10.0, 20.0, 30.0,
                -4608.0, 5.0, 3.0, 0.0, 1.0, 9.0, 1.0, 6.0, 0.0,
            ]
        );
        assert!(present[93..116].iter().all(|value| *value));

        let legacy = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v24.dseps"
        ))
        .unwrap();
        let (legacy_base, legacy_present) = broad_base(&legacy.episodes[0].steps[0].pre_input);
        assert!(legacy_base[93..116].iter().all(|value| *value == 0.0));
        assert!(legacy_present[93..116].iter().all(|value| !*value));

        let mut base = base;
        let mut present = present;
        append_core_temporal_features(&mut base, &mut present, observation, None);
        let reduced =
            NativeEncoderFeatureSpec::excluding([NativeEncoderChannelFamily::CoreWarpSession])
                .unwrap();
        retain_feature_families(
            &mut base,
            &mut present,
            &native_base_feature_families(),
            &reduced,
        );
        assert_eq!(base.len(), 225);
        assert_eq!(present.len(), 225);
    }

    #[test]
    fn direct_native_adapter_keeps_the_complete_typed_actor_population() {
        let shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v15.dseps"
        ))
        .unwrap();
        let observation = &shard.episodes[0].steps[0].pre_input;
        assert!(!observation.actors_truncated);
        let nodes = native_actor_nodes(observation, None);
        assert_eq!(nodes.len(), observation.actors.len());
        assert_eq!(
            nodes.iter().map(|node| node.stable_id).collect::<Vec<_>>(),
            observation
                .actors
                .iter()
                .map(|actor| actor.runtime_generation)
                .collect::<Vec<_>>()
        );
        assert!(nodes.iter().all(|node| {
            node.categorical.len() == native_actor_categorical_names().len()
                && node.categorical.len() == node.categorical_present.len()
                && node.continuous.len() == native_actor_continuous_names().len()
                && node.continuous.len() == node.continuous_present.len()
                && node.binary.len() == native_actor_binary_names().len()
                && node.binary.len() == node.binary_present.len()
        }));
        let lock_membership = native_actor_binary_names()
            .iter()
            .position(|name| name == "attention_lock_candidate")
            .unwrap();
        assert!(
            nodes
                .iter()
                .all(|node| !node.binary[lock_membership] && !node.binary_present[lock_membership])
        );
        let (base, present) = broad_base(observation);
        assert_eq!(base.len(), 223);
        assert_eq!(present.len(), 223);
        assert!(base[62..93].iter().all(|value| *value == 0.0));
        assert!(present[62..93].iter().all(|value| !*value));
        assert!(base[93..116].iter().all(|value| *value == 0.0));
        assert!(present[93..116].iter().all(|value| !*value));
        assert!(base[183..].iter().all(|value| *value == 0.0));
        assert!(present[183..].iter().all(|value| !*value));
        let mut temporal_base = base.clone();
        let mut temporal_present = present.clone();
        append_core_temporal_features(&mut temporal_base, &mut temporal_present, observation, None);
        let all = NativeEncoderFeatureSpec::all();
        assert_eq!(temporal_base.len(), 248);
        assert_eq!(temporal_present.len(), 248);
        assert_eq!(native_base_feature_names().len(), 248);
        assert_eq!(native_base_feature_families().len(), 248);
        let previous_available = native_base_feature_names()
            .iter()
            .position(|name| name == "temporal_previous_state_available")
            .unwrap();
        assert_eq!(temporal_base[previous_available], 0.0);
        assert!(temporal_present[previous_available]);
        let actor_previous_available = native_actor_binary_names()
            .iter()
            .position(|name| name == "temporal_previous_actor_present")
            .unwrap();
        assert!(!nodes[0].binary[actor_previous_available]);
        assert!(!nodes[0].binary_present[actor_previous_available]);
        let mut post_base = temporal_base.clone();
        let mut post_present = temporal_present.clone();
        suppress_base_family(
            &mut post_base,
            &mut post_present,
            NativeEncoderChannelFamily::CorePreviousInput,
        );
        for (index, family) in native_base_feature_families().into_iter().enumerate() {
            if family == NativeEncoderChannelFamily::CorePreviousInput {
                assert_eq!(post_base[index], 0.0);
                assert!(!post_present[index]);
            } else {
                assert_eq!(post_base[index], temporal_base[index]);
                assert_eq!(post_present[index], temporal_present[index]);
            }
        }
        assert_ne!(native_actor_feature_schema(&all).unwrap(), Digest::ZERO);
        let reduced = NativeEncoderFeatureSpec::excluding([
            NativeEncoderChannelFamily::CoreAttentionCandidates,
            NativeEncoderChannelFamily::ActorAttention,
            NativeEncoderChannelFamily::ActorAttentionCandidates,
            NativeEncoderChannelFamily::ActorEventParticipation,
            NativeEncoderChannelFamily::ActorEnemyBase,
            NativeEncoderChannelFamily::ActorTriggerVolume,
            NativeEncoderChannelFamily::ActorDoor20,
        ])
        .unwrap();
        let mut reduced_node = nodes[0].clone();
        retain_node_feature_families(&mut reduced_node, &reduced);
        assert!(reduced_node.categorical.len() < nodes[0].categorical.len());
        assert!(reduced_node.continuous.len() < nodes[0].continuous.len());
        assert!(reduced_node.binary.len() < nodes[0].binary.len());
        assert_ne!(
            native_actor_feature_schema(&reduced).unwrap(),
            native_actor_feature_schema(&all).unwrap()
        );
    }

    #[test]
    fn direct_native_adapter_exposes_ablatable_door20_with_legacy_masks() {
        let shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v27.dseps"
        ))
        .unwrap();
        let observation = &shard.episodes[0].steps[0].pre_input;
        let mut nodes = native_actor_nodes(observation, None);
        let categorical_names = native_actor_categorical_names();
        let continuous_names = native_actor_continuous_names();
        let binary_names = native_actor_binary_names();
        let kind = categorical_names
            .iter()
            .position(|name| name == "door20_kind")
            .unwrap();
        let front_switch = categorical_names
            .iter()
            .position(|name| name == "door20_front_switch")
            .unwrap();
        let action = categorical_names
            .iter()
            .position(|name| name == "door20_action")
            .unwrap();
        let angle = continuous_names
            .iter()
            .position(|name| name == "door20_angle_s16")
            .unwrap();
        let present = binary_names
            .iter()
            .position(|name| name == "door20_present")
            .unwrap();
        let front_switch_set = binary_names
            .iter()
            .position(|name| name == "door20_front_switch_set")
            .unwrap();
        let opening = binary_names
            .iter()
            .position(|name| name == "door20_opening_active")
            .unwrap();
        let door = nodes
            .iter()
            .find(|node| node.binary[present])
            .expect("direct DOOR20 node");
        assert!(door.binary_present[present]);
        assert_eq!(door.categorical[kind], 9);
        assert_eq!(door.categorical[front_switch], 0x11);
        assert!(door.categorical_present[front_switch]);
        assert_eq!(door.categorical[action], 3);
        assert_eq!(door.continuous[angle], -1234.0);
        assert!(door.continuous_present[angle]);
        assert!(door.binary[front_switch_set]);
        assert!(door.binary[opening]);

        let spec = NativeEncoderFeatureSpec::new([
            NativeEncoderChannelFamily::ActorPopulation,
            NativeEncoderChannelFamily::ActorDoor20,
        ])
        .unwrap();
        for node in &mut nodes {
            retain_node_feature_families(node, &spec);
            assert_eq!(node.categorical.len(), 21);
            assert_eq!(node.continuous.len(), 1);
            assert_eq!(node.binary.len(), 10);
        }
        assert_eq!(
            selected_feature_names(
                native_actor_categorical_names(),
                &native_actor_categorical_families(),
                &spec,
            )
            .len(),
            21
        );

        let legacy = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v26.dseps"
        ))
        .unwrap();
        let legacy = native_actor_nodes(&legacy.episodes[0].steps[0].pre_input, None);
        assert!(legacy.iter().all(|node| {
            !node.binary[present]
                && node.binary_present[present]
                && node.categorical[kind] == 0
                && !node.categorical_present[kind]
                && node.continuous[angle] == 0.0
                && !node.continuous_present[angle]
                && !node.binary[opening]
                && !node.binary_present[opening]
        }));

        let all = NativeEncoderFeatureSpec::all();
        let without_door =
            NativeEncoderFeatureSpec::excluding([NativeEncoderChannelFamily::ActorDoor20]).unwrap();
        for (names, families, removed) in [
            (
                native_actor_categorical_names(),
                native_actor_categorical_families(),
                21,
            ),
            (
                native_actor_continuous_names(),
                native_actor_continuous_families(),
                1,
            ),
            (
                native_actor_binary_names(),
                native_actor_binary_families(),
                10,
            ),
        ] {
            assert_eq!(
                selected_feature_names(names.clone(), &families, &all).len()
                    - selected_feature_names(names, &families, &without_door).len(),
                removed
            );
        }
        assert_ne!(
            native_actor_feature_schema(&all).unwrap(),
            native_actor_feature_schema(&without_door).unwrap()
        );
    }

    #[test]
    fn temporal_features_are_past_only_and_join_actors_by_runtime_generation() {
        let shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v15.dseps"
        ))
        .unwrap();
        let previous = shard.episodes[0].steps[0].pre_input.clone();
        let mut current = previous.clone();
        current.player_position[0] += 1.25;
        current.player_velocity[2] -= 0.5;
        let actor = current.actors.first_mut().unwrap();
        let stable_id = actor.runtime_generation;
        actor.position[0] += 3.5;
        actor.velocity[1] -= 2.0;
        actor.status ^= 1;

        let (mut base, mut present) = broad_base(&current);
        append_core_temporal_features(&mut base, &mut present, &current, Some(&previous));
        let base_names = native_base_feature_names();
        let player_delta_x = base_names
            .iter()
            .position(|name| name == "temporal_player_position_x")
            .unwrap();
        assert_eq!(base[player_delta_x], 1.25);
        assert!(present[player_delta_x]);

        let node = native_actor_nodes(&current, Some(&previous))
            .into_iter()
            .find(|node| node.stable_id == stable_id)
            .unwrap();
        let continuous_names = native_actor_continuous_names();
        let position_delta_x = continuous_names
            .iter()
            .position(|name| name == "temporal_position_delta_x")
            .unwrap();
        let velocity_delta_y = continuous_names
            .iter()
            .position(|name| name == "temporal_velocity_delta_y")
            .unwrap();
        assert_eq!(node.continuous[position_delta_x], 3.5);
        assert!(node.continuous_present[position_delta_x]);
        assert_eq!(node.continuous[velocity_delta_y], -2.0);
        assert!(node.continuous_present[velocity_delta_y]);

        let binary_names = native_actor_binary_names();
        let previous_actor_present = binary_names
            .iter()
            .position(|name| name == "temporal_previous_actor_present")
            .unwrap();
        let status_changed = binary_names
            .iter()
            .position(|name| name == "temporal_status_changed")
            .unwrap();
        assert!(node.binary[previous_actor_present]);
        assert!(node.binary_present[previous_actor_present]);
        assert!(node.binary[status_changed]);
        assert!(node.binary_present[status_changed]);

        current.room = current.room.wrapping_add(1);
        let context_changed_node = native_actor_nodes(&current, Some(&previous))
            .into_iter()
            .find(|node| node.stable_id == stable_id)
            .unwrap();
        assert!(!context_changed_node.binary[previous_actor_present]);
        assert!(!context_changed_node.binary_present[previous_actor_present]);
        assert!(!context_changed_node.continuous_present[position_delta_x]);
    }

    #[test]
    fn rare_event_metrics_report_recall_and_probability_error() {
        let mut accumulator = BinaryEventAccumulator::default();
        for (expected, score) in [(true, 0.9), (true, 0.2), (false, 0.8), (false, 0.1)] {
            accumulator.observe(expected, score);
        }
        let metrics = accumulator.finish().unwrap();
        assert_eq!(metrics.positives, 2);
        assert_eq!(metrics.negatives, 2);
        assert_eq!(metrics.true_positives, 1);
        assert_eq!(metrics.false_positives, 1);
        assert_eq!(metrics.true_negatives, 1);
        assert_eq!(metrics.false_negatives, 1);
        assert_eq!(metrics.precision, Some(0.5));
        assert_eq!(metrics.recall, Some(0.5));
        assert_eq!(metrics.specificity, Some(0.5));
        assert_eq!(metrics.balanced_accuracy, Some(0.5));
        assert_eq!(metrics.f1, Some(0.5));
        assert!((metrics.brier_score - 0.325).abs() < 1.0e-12);
    }

    #[test]
    fn bernoulli_loss_is_stable_and_its_gradient_matches_finite_difference() {
        let logit = 0.37;
        let expected = 1.0;
        let weight = 3.25;
        let epsilon = 1.0e-6;
        let numeric = weight
            * (binary_cross_entropy_from_logit(logit + epsilon, expected)
                - binary_cross_entropy_from_logit(logit - epsilon, expected))
            / (2.0 * epsilon);
        let analytic = weight * (logistic(logit) - expected);
        assert!((numeric - analytic).abs() < 1.0e-9);
        assert!(binary_cross_entropy_from_logit(1_000.0, 0.0).is_finite());
        assert!(binary_cross_entropy_from_logit(-1_000.0, 1.0).is_finite());
    }

    #[test]
    fn bernoulli_normalization_balances_classes_without_changing_regression() {
        let mut samples = corpus(1, 4);
        for (index, sample) in samples.iter_mut().enumerate() {
            sample.targets = vec![f32::from(index == 0), index as f32];
            sample.target_present = vec![true, true];
        }
        let objectives = vec![
            AuxiliaryHeadObjective::ClassBalancedBernoulli,
            AuxiliaryHeadObjective::NormalizedRegression,
        ];
        let normalization = target_normalization(&samples, &objectives).unwrap();
        assert_eq!(normalization.mean, vec![0.25, 1.5]);
        assert_eq!(normalization.inverse_stddev[0], 1.0);
        assert_eq!(normalization.positive_weight[0], 2.0);
        assert!((normalization.negative_weight[0] - 2.0 / 3.0).abs() < 1.0e-12);
        assert_eq!(normalization.positive_weight[1], 1.0);
        assert_eq!(normalization.negative_weight[1], 1.0);

        samples[0].targets[0] = 0.25;
        assert!(target_normalization(&samples, &objectives).is_err());
    }

    #[test]
    fn typed_multitask_fit_binds_balanced_binary_heads_and_probabilities() {
        let mut training = corpus(1, 40);
        let mut held_out = corpus(101, 20);
        for (index, sample) in training.iter_mut().enumerate() {
            sample.targets[0] = f32::from(index % 10 == 0);
        }
        for (index, sample) in held_out.iter_mut().enumerate() {
            sample.targets[0] = f32::from(index % 5 == 0);
        }
        let training_digest = sample_manifest_digest(&training).unwrap();
        let held_out_digest = sample_manifest_digest(&held_out).unwrap();
        let config = TrainableSetConfig {
            epochs: 4,
            node_hidden_width: 4,
            head_hidden_width: 4,
            ..TrainableSetConfig::default()
        };
        let (report, model) = CompleteSetMultiTaskEncoder::fit(
            Digest([7; 32]),
            training_digest,
            held_out_digest,
            vec!["contact_changed".into(), "inverse_difference".into()],
            &training,
            &held_out,
            config,
        )
        .unwrap();
        assert_eq!(
            report.target_objectives,
            vec![
                AuxiliaryHeadObjective::ClassBalancedBernoulli,
                AuxiliaryHeadObjective::NormalizedRegression,
            ]
        );
        assert_eq!(report.target_positive_weights, vec![5.0, 1.0]);
        assert!((report.target_negative_weights[0] - 5.0 / 9.0).abs() < 1.0e-12);
        assert_eq!(report.target_negative_weights[1], 1.0);
        assert!(report.training_objective_loss.is_finite());
        assert!(report.held_out_objective_loss.is_finite());
        let probability = model.predict(&held_out[0]).unwrap()[0];
        assert!((0.0..=1.0).contains(&probability));

        let control = fit_shuffled_auxiliary_control(
            Digest([7; 32]),
            vec!["contact_changed".into(), "inverse_difference".into()],
            training,
            held_out_digest,
            &held_out,
            &held_out,
            config,
        )
        .unwrap();
        assert_eq!(control.report.target_objectives, report.target_objectives);
        assert_eq!(
            control.report.target_positive_weights,
            report.target_positive_weights
        );
    }

    #[test]
    fn feature_family_names_round_trip_and_actor_columns_require_population() {
        let target_names = native_target_names();
        assert_eq!(target_names.len(), 15);
        assert_eq!(
            target_conditioning_for_names(&target_names),
            native_target_conditioning()
        );
        let objectives = target_objectives_for_names(&target_names);
        assert_eq!(
            objectives[target_names
                .iter()
                .position(|name| name == "actor_disappearance_occurred")
                .unwrap()],
            AuxiliaryHeadObjective::ClassBalancedBernoulli
        );
        assert_eq!(
            objectives[target_names
                .iter()
                .position(|name| name == "actor_disappearance_count")
                .unwrap()],
            AuxiliaryHeadObjective::NormalizedRegression
        );
        assert_eq!(
            MultiTaskSetPooling::parse("mean-max"),
            Some(MultiTaskSetPooling::MeanMax)
        );
        assert_eq!(
            MultiTaskSetPooling::parse("mean-max-learned-attention"),
            Some(MultiTaskSetPooling::MeanMaxLearnedAttention)
        );
        assert_eq!(
            MultiTaskSetPooling::parse("mean-max-task-attention"),
            Some(MultiTaskSetPooling::MeanMaxTaskAttention)
        );
        assert_eq!(MultiTaskSetPooling::parse("nearest-actor"), None);
        for family in NativeEncoderChannelFamily::ALL {
            assert_eq!(
                NativeEncoderChannelFamily::parse(family.name()),
                Some(family)
            );
        }
        assert!(NativeEncoderChannelFamily::parse("nearest_actor_magic").is_none());
        assert!(NativeEncoderFeatureSpec::new([NativeEncoderChannelFamily::ActorMotion]).is_err());
        assert!(
            NativeEncoderFeatureSpec::new([NativeEncoderChannelFamily::CorePreviousInput]).is_ok()
        );
        assert!(
            NativeEncoderFeatureSpec::all()
                .with_history_depth(MAX_EPISODE_HISTORY_DEPTH + 1)
                .is_err()
        );
        assert!(
            NativeEncoderFeatureSpec::all()
                .with_recurrent_history(0, DEFAULT_HISTORY_RECURRENT_WIDTH)
                .is_err()
        );
        assert!(
            NativeEncoderFeatureSpec::all()
                .with_recurrent_history(2, 0)
                .is_err()
        );
        assert!(
            NativeEncoderFeatureSpec::all()
                .with_recurrent_history(2, MAX_HISTORY_RECURRENT_WIDTH + 1)
                .is_err()
        );
        assert!(
            NativeEncoderFeatureSpec::all()
                .with_trainable_history(0, DEFAULT_HISTORY_RECURRENT_WIDTH)
                .is_err()
        );
        assert_ne!(
            native_actor_feature_schema(&NativeEncoderFeatureSpec::all()).unwrap(),
            native_actor_feature_schema(
                &NativeEncoderFeatureSpec::all()
                    .with_history_depth(2)
                    .unwrap()
            )
            .unwrap()
        );
        assert_ne!(
            native_actor_feature_schema(
                &NativeEncoderFeatureSpec::all()
                    .with_recurrent_history(2, DEFAULT_HISTORY_RECURRENT_WIDTH)
                    .unwrap()
            )
            .unwrap(),
            native_actor_feature_schema(
                &NativeEncoderFeatureSpec::all()
                    .with_trainable_history(2, DEFAULT_HISTORY_RECURRENT_WIDTH)
                    .unwrap()
            )
            .unwrap()
        );
        assert_ne!(
            native_actor_feature_schema(
                &NativeEncoderFeatureSpec::all()
                    .with_history_depth(2)
                    .unwrap()
            )
            .unwrap(),
            native_actor_feature_schema(
                &NativeEncoderFeatureSpec::all()
                    .with_recurrent_history(2, DEFAULT_HISTORY_RECURRENT_WIDTH)
                    .unwrap()
            )
            .unwrap()
        );
        assert!(
            NativeEncoderFeatureSpec {
                families: vec![
                    NativeEncoderChannelFamily::CoreGoal,
                    NativeEncoderChannelFamily::CorePlayerMotion,
                ],
                history_depth: 0,
                history_encoding: NativeEncoderHistoryEncoding::None,
                history_recurrent_width: 0,
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn stacked_history_is_past_only_right_aligned_and_masked_at_episode_start() {
        let mut shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v14.dseps"
        ))
        .unwrap();
        let prototype = shard.episodes[0].steps[0].clone();
        shard.episodes[0].steps = vec![prototype; 3];
        let episode = &shard.episodes[0];
        let history = NativeEpisodeHistoryView::build(&shard, 2).unwrap();
        let spec = NativeEncoderFeatureSpec::all()
            .with_history_depth(2)
            .unwrap();
        let names = native_history_feature_names(&spec);
        assert_eq!(names.len() % 2, 0);
        let slot_width = names.len() / 2;

        let mut start_values = Vec::new();
        let mut start_present = Vec::new();
        append_episode_history_features(&mut start_values, &mut start_present, episode, &[], &spec)
            .unwrap();
        assert_eq!(start_values.len(), names.len());
        assert_eq!(start_values[0], 0.0);
        assert_eq!(start_values[slot_width], 0.0);
        assert!(start_present[0] && start_present[slot_width]);
        assert!(start_present[1..slot_width].iter().all(|present| !*present));
        assert!(
            start_present[slot_width + 1..]
                .iter()
                .all(|present| !*present)
        );

        let decision = &history.decisions[2];
        assert_eq!(decision.episode_id, episode.id);
        assert_eq!(decision.step_index, 2);
        let completed = decision
            .completed_transition_indices
            .iter()
            .map(|index| &history.transitions[*index as usize])
            .collect::<Vec<_>>();
        let mut values = Vec::new();
        let mut present = Vec::new();
        append_episode_history_features(&mut values, &mut present, episode, &completed, &spec)
            .unwrap();
        assert_eq!(values[0], 1.0);
        assert_eq!(values[slot_width], 1.0);
        assert!(present[1..].iter().any(|present| *present));

        let mut changed_current = episode.clone();
        changed_current.steps[2].consumed_pad.buttons ^= 0xffff;
        changed_current.steps[2].post_simulation.player_position[0] += 10_000.0;
        let mut unchanged_values = Vec::new();
        let mut unchanged_present = Vec::new();
        append_episode_history_features(
            &mut unchanged_values,
            &mut unchanged_present,
            &changed_current,
            &completed,
            &spec,
        )
        .unwrap();
        assert_eq!(unchanged_values, values);
        assert_eq!(unchanged_present, present);
    }

    #[test]
    fn recurrent_history_is_bounded_deterministic_and_past_only() {
        let mut shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v14.dseps"
        ))
        .unwrap();
        let prototype = shard.episodes[0].steps[0].clone();
        shard.episodes[0].steps = vec![prototype; 3];
        let episode = &shard.episodes[0];
        let history = NativeEpisodeHistoryView::build(&shard, 2).unwrap();
        let spec = NativeEncoderFeatureSpec::all()
            .with_recurrent_history(2, 4)
            .unwrap();
        let reservoir = native_recurrent_history_reservoir(&spec).unwrap().unwrap();
        assert_eq!(native_history_feature_names(&spec).len(), 6);

        let mut start_values = Vec::new();
        let mut start_present = Vec::new();
        append_encoded_episode_history_features(
            &mut start_values,
            &mut start_present,
            episode,
            &[],
            &spec,
            Some(&reservoir),
        )
        .unwrap();
        assert_eq!(start_values, vec![0.0; 6]);
        assert_eq!(start_present, vec![true; 6]);

        let decision = &history.decisions[2];
        let completed = decision
            .completed_transition_indices
            .iter()
            .map(|index| &history.transitions[*index as usize])
            .collect::<Vec<_>>();
        let mut values = Vec::new();
        let mut present = Vec::new();
        append_encoded_episode_history_features(
            &mut values,
            &mut present,
            episode,
            &completed,
            &spec,
            Some(&reservoir),
        )
        .unwrap();
        assert_eq!(values.len(), 6);
        assert_eq!(values[0], 1.0);
        assert_eq!(values[1], 1.0);
        assert!(values[2..].iter().all(|value| value.is_finite()));
        assert!(values[2..].iter().any(|value| *value != 0.0));
        assert_eq!(present, vec![true; 6]);

        let mut repeated_values = Vec::new();
        let mut repeated_present = Vec::new();
        append_encoded_episode_history_features(
            &mut repeated_values,
            &mut repeated_present,
            episode,
            &completed,
            &spec,
            Some(&reservoir),
        )
        .unwrap();
        assert_eq!(repeated_values, values);
        assert_eq!(repeated_present, present);

        let mut changed_current = episode.clone();
        changed_current.steps[2].consumed_pad.buttons ^= 0xffff;
        changed_current.steps[2].post_simulation.player_position[0] += 10_000.0;
        let mut unchanged_values = Vec::new();
        let mut unchanged_present = Vec::new();
        append_encoded_episode_history_features(
            &mut unchanged_values,
            &mut unchanged_present,
            &changed_current,
            &completed,
            &spec,
            Some(&reservoir),
        )
        .unwrap();
        assert_eq!(unchanged_values, values);
        assert_eq!(unchanged_present, present);
    }

    #[test]
    fn trainable_history_shares_complete_states_and_excludes_the_current_transition() {
        let mut shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v14.dseps"
        ))
        .unwrap();
        let prototype = shard.episodes[0].steps[0].clone();
        shard.episodes[0].steps = vec![prototype; 3];
        let episode = &shard.episodes[0];
        let history = NativeEpisodeHistoryView::build(&shard, 2).unwrap();
        let spec = NativeEncoderFeatureSpec::all()
            .with_trainable_history(2, 4)
            .unwrap();
        let schema = native_actor_feature_schema(&spec).unwrap();
        let mut states = BTreeMap::new();

        let first_decision = &history.decisions[1];
        let first_completed = first_decision
            .completed_transition_indices
            .iter()
            .map(|index| &history.transitions[*index as usize])
            .collect::<Vec<_>>();
        let first =
            trainable_episode_history_steps(episode, &first_completed, &spec, schema, &mut states)
                .unwrap();
        assert_eq!(first.len(), 1);
        assert!(!first[0].state.nodes.is_empty());

        let decision = &history.decisions[2];
        let completed = decision
            .completed_transition_indices
            .iter()
            .map(|index| &history.transitions[*index as usize])
            .collect::<Vec<_>>();
        let second =
            trainable_episode_history_steps(episode, &completed, &spec, schema, &mut states)
                .unwrap();
        assert_eq!(second.len(), 2);
        assert!(Arc::ptr_eq(&first[0].state, &second[0].state));
        assert_eq!(states.len(), 2);

        let mut changed_current = episode.clone();
        changed_current.steps[2].consumed_pad.buttons ^= 0xffff;
        changed_current.steps[2].post_simulation.player_position[0] += 10_000.0;
        let mut changed_states = BTreeMap::new();
        let unchanged = trainable_episode_history_steps(
            &changed_current,
            &completed,
            &spec,
            schema,
            &mut changed_states,
        )
        .unwrap();
        assert_eq!(
            unchanged
                .iter()
                .map(|step| (step.transition_sha256, step.state.sample_sha256))
                .collect::<Vec<_>>(),
            second
                .iter()
                .map(|step| (step.transition_sha256, step.state.sample_sha256))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            unchanged
                .iter()
                .map(|step| &step.action_context)
                .collect::<Vec<_>>(),
            second
                .iter()
                .map(|step| &step.action_context)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn historical_actor_state_receives_gradient_through_the_gru() {
        let mut sample = sample(201, 1.25, -0.5, false);
        let mut history_state = sample.input.clone();
        history_state.sample_sha256 = Digest([202; 32]);
        sample.input.nodes.clear();
        sample.post_input.nodes.clear();
        sample.history = vec![MultiTaskHistoryStep {
            transition_sha256: Digest([203; 32]),
            state: Arc::new(history_state),
            action_context: vec![0.0; ACTION_CONTEXT_WIDTH],
        }];
        sample.targets = vec![1.0, 0.0];
        sample.target_present = vec![true, false];
        let dimensions = Dimensions {
            categorical: 1,
            continuous: 1,
            binary: 1,
            base: 1,
        };
        let layout = FeatureLayout::fit(sample_model_states(&sample), dimensions).unwrap();
        let config = TrainableSetConfig {
            epochs: 1,
            node_hidden_width: 4,
            head_hidden_width: 4,
            l2_penalty: 0.0,
            ..TrainableSetConfig::default()
        };
        let temporal = MultiTaskTemporalConfig::gated_recurrent(2, 4);
        let mut model = CompleteSetMultiTaskEncoder::initialized(
            Digest([7; 32]),
            layout,
            config,
            vec![
                "actor_disappearance_occurred".into(),
                "inverse_stick_x".into(),
            ],
            vec![
                AuxiliaryHeadConditioning::PreStateAndAction,
                AuxiliaryHeadConditioning::PreAndPostState,
            ],
            vec![
                AuxiliaryHeadObjective::ClassBalancedBernoulli,
                AuxiliaryHeadObjective::NormalizedRegression,
            ],
            vec![0.1, 0.0],
            vec![1.0; 2],
            vec![2.0, 1.0],
            vec![0.5, 1.0],
            MultiTaskSetPooling::MeanMax,
            temporal,
        )
        .unwrap();
        model.output_weights.fill(0.0);
        let history_offset = config.head_hidden_width * 2 + ACTION_CONTEXT_WIDTH;
        model.output_weights[history_offset] = 1.0;
        let before = model.node_weights.clone();
        model.train_one(&sample).unwrap();
        let gradient_l1 = model
            .node_weights
            .iter()
            .zip(before)
            .map(|(after, before)| (after - before).abs())
            .sum::<f64>();
        assert!(gradient_l1 > 0.0);
    }

    #[test]
    fn trainable_history_refits_and_actor_permutations_are_exact() {
        let attach_history = |samples: &mut [MultiTaskSetSample]| {
            for sample in samples {
                let mut state = sample.input.clone();
                state.sample_sha256 = canonical_digest(
                    b"dusklight.synthetic-history-state/v1\0",
                    &sample.input.sample_sha256,
                )
                .unwrap();
                sample.history = vec![MultiTaskHistoryStep {
                    transition_sha256: canonical_digest(
                        b"dusklight.synthetic-history-transition/v1\0",
                        &sample.input.sample_sha256,
                    )
                    .unwrap(),
                    state: Arc::new(state),
                    action_context: sample.action_context.clone(),
                }];
            }
        };
        let mut training = corpus(1, 48);
        let mut held_out = corpus(101, 16);
        attach_history(&mut training);
        attach_history(&mut held_out);
        let training_digest = sample_manifest_digest(&training).unwrap();
        let held_out_digest = sample_manifest_digest(&held_out).unwrap();
        let config = TrainableSetConfig {
            epochs: 4,
            node_hidden_width: 4,
            head_hidden_width: 4,
            ..TrainableSetConfig::default()
        };
        let temporal = MultiTaskTemporalConfig::gated_recurrent(2, 4);
        let fit = || {
            CompleteSetMultiTaskEncoder::fit_with_pooling_and_temporal(
                Digest([7; 32]),
                training_digest,
                held_out_digest,
                vec!["sum".into(), "inverse_difference".into()],
                &training,
                &held_out,
                config,
                MultiTaskSetPooling::MeanMax,
                temporal,
            )
            .unwrap()
        };
        let (first_report, first) = fit();
        let (second_report, second) = fit();
        assert_eq!(first_report.temporal, temporal);
        assert_eq!(first_report.report_sha256, second_report.report_sha256);
        assert_eq!(
            first.model_sha256().unwrap(),
            second.model_sha256().unwrap()
        );

        let original = first.predict(&held_out[0]).unwrap();
        let mut permuted = held_out[0].clone();
        permuted.input.nodes.reverse();
        permuted.post_input.nodes.reverse();
        let mut history_state = (*permuted.history[0].state).clone();
        history_state.nodes.reverse();
        permuted.history[0].state = Arc::new(history_state);
        assert_eq!(first.predict(&permuted).unwrap(), original);
    }

    #[test]
    fn direct_native_adapter_joins_attention_candidates_without_selecting_one() {
        let shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v20.dseps"
        ))
        .unwrap();
        let observation = &shard.episodes[0].steps[0].pre_input;
        let node = native_actor_nodes(observation, None)
            .into_iter()
            .find(|node| node.stable_id == 7)
            .unwrap();
        let categorical_names = native_actor_categorical_names();
        let continuous_names = native_actor_continuous_names();
        let binary_names = native_actor_binary_names();
        let categorical = |name: &str| {
            let index = categorical_names
                .iter()
                .position(|value| value == name)
                .unwrap();
            (node.categorical[index], node.categorical_present[index])
        };
        let continuous = |name: &str| {
            let index = continuous_names
                .iter()
                .position(|value| value == name)
                .unwrap();
            (node.continuous[index], node.continuous_present[index])
        };
        let binary = |name: &str| {
            let index = binary_names.iter().position(|value| value == name).unwrap();
            (node.binary[index], node.binary_present[index])
        };

        assert_eq!(categorical("attention_lock_type"), (1, true));
        assert_eq!(categorical("attention_lock_rank"), (0, true));
        assert_eq!(categorical("attention_action_type"), (6, true));
        assert_eq!(categorical("attention_action_rank"), (0, true));
        assert_eq!(categorical("attention_check_type"), (0, false));
        assert_eq!(continuous("attention_lock_weight"), (0.25, true));
        assert_eq!(continuous("attention_lock_distance"), (80.0, true));
        assert_eq!(continuous("attention_lock_angle_s16"), (-256.0, true));
        assert_eq!(continuous("attention_action_weight"), (0.5, true));
        assert_eq!(continuous("attention_action_distance"), (90.0, true));
        assert_eq!(continuous("attention_action_angle_s16"), (512.0, true));
        assert_eq!(continuous("attention_check_weight"), (0.0, false));
        assert_eq!(binary("attention_lock_candidate"), (true, true));
        assert_eq!(binary("attention_action_candidate"), (true, true));
        assert_eq!(binary("attention_check_candidate"), (false, true));
        let (base, base_present) = broad_base(observation);
        assert_eq!(base.len(), 223);
        assert!(base_present[62..93].iter().all(|value| !*value));
        assert!(base_present[93..116].iter().all(|value| !*value));
        assert!(base_present[183..].iter().all(|value| *value));
        assert_eq!(base[183 + 2], 1.0);
        assert_eq!(base[183 + 4], 1.0);
        assert_eq!(base[215], 2.0);
        assert_eq!(base[216], 3.0);
        assert_eq!(base[217], 1.0);
        assert_eq!(base[219], 1.0);
        assert_eq!(base[221], 0.0);
    }

    #[test]
    fn shuffled_control_rebinds_targets_without_changing_support() {
        let training = corpus(1, 96);
        let validation = corpus(130, 32);
        let test = corpus(170, 32);
        let original_digest = sample_manifest_digest(&training).unwrap();
        let config = TrainableSetConfig {
            epochs: 2,
            node_hidden_width: 8,
            head_hidden_width: 8,
            minimum_relative_improvement: 1.0,
            ..TrainableSetConfig::default()
        };
        let control = fit_shuffled_auxiliary_control(
            Digest([7; 32]),
            vec!["forward_sum".into(), "inverse_difference".into()],
            training,
            sample_manifest_digest(&validation).unwrap(),
            &validation,
            &test,
            config,
        )
        .unwrap();
        assert_eq!(control.schema, SHUFFLED_AUXILIARY_CONTROL_SCHEMA_V1);
        assert_ne!(control.shuffled_training_dataset_sha256, original_digest);
        assert_eq!(control.report.target_support_training, vec![96, 77]);
        assert_eq!(control.test_evaluation.samples, 32);
        assert_eq!(
            control.report.decision,
            MultiTaskEncoderDecision::RetainTrainingMeanBaseline
        );
    }

    #[test]
    fn actorless_control_does_not_leak_set_cardinality() {
        let mut training = corpus(1, 32);
        let mut held_out = corpus(80, 16);
        for sample in training.iter_mut().chain(&mut held_out) {
            sample.input.actor_feature_schema_sha256 = Digest([6; 32]);
            sample.input.nodes.clear();
            sample.post_input.actor_feature_schema_sha256 = Digest([6; 32]);
            sample.post_input.nodes.clear();
        }
        let (report, model) = CompleteSetMultiTaskEncoder::fit(
            Digest([6; 32]),
            Digest([8; 32]),
            Digest([9; 32]),
            vec!["forward_sum".into(), "inverse_difference".into()],
            &training,
            &held_out,
            TrainableSetConfig {
                epochs: 2,
                node_hidden_width: 8,
                head_hidden_width: 8,
                ..TrainableSetConfig::default()
            },
        )
        .unwrap();
        assert_eq!(report.maximum_training_nodes, 0);
        assert_eq!(report.maximum_held_out_nodes, 0);
        assert_eq!(model.encode(&held_out[0].input).unwrap().len(), 8);
    }

    #[test]
    fn shared_complete_set_encoder_learns_masked_heads_on_held_out_rows() {
        let training = corpus(1, 96);
        let held_out = corpus(130, 32);
        let config = TrainableSetConfig {
            epochs: 180,
            node_hidden_width: 12,
            head_hidden_width: 16,
            learning_rate: 0.003,
            minimum_relative_improvement: 0.25,
            ..TrainableSetConfig::default()
        };
        let (report, model) = CompleteSetMultiTaskEncoder::fit(
            Digest([7; 32]),
            Digest([8; 32]),
            Digest([9; 32]),
            vec!["forward_sum".into(), "inverse_difference".into()],
            &training,
            &held_out,
            config,
        )
        .unwrap();
        assert_eq!(report.target_support_training, vec![96, 77]);
        assert_eq!(report.target_support_held_out, vec![32, 25]);
        assert_eq!(
            report.target_conditioning,
            vec![
                AuxiliaryHeadConditioning::PreStateAndAction,
                AuxiliaryHeadConditioning::PreAndPostState,
            ]
        );
        assert!(report.relative_held_out_improvement > 0.25);
        assert_eq!(
            report.decision,
            MultiTaskEncoderDecision::SharedEncoderCandidate
        );
        assert_eq!(model.encode(&held_out[0].input).unwrap().len(), 16);
        assert_eq!(model.predict(&held_out[0]).unwrap().len(), 2);
        let baseline = model.predict(&held_out[0]).unwrap();
        let mut changed_post = held_out[0].clone();
        changed_post.post_input.base[0] += 1000.0;
        changed_post.post_input.nodes[0].continuous[0] -= 1000.0;
        let post_prediction = model.predict(&changed_post).unwrap();
        assert_eq!(baseline[0], post_prediction[0]);
        let mut changed_action = held_out[0].clone();
        changed_action.action_context.fill(0.75);
        let action_prediction = model.predict(&changed_action).unwrap();
        assert_eq!(baseline[1], action_prediction[1]);
        let evaluation = model.evaluate(&held_out).unwrap();
        assert_eq!(evaluation.samples, 32);
        assert!(evaluation.relative_improvement > 0.25);
        assert!(!report.promotion_authority);
        assert_ne!(report.report_sha256, Digest::ZERO);
    }

    #[test]
    fn learned_attention_pooling_is_seeded_trainable_and_permutation_invariant() {
        let training = corpus(1, 48);
        let held_out = corpus(100, 16);
        let config = TrainableSetConfig {
            epochs: 4,
            node_hidden_width: 8,
            head_hidden_width: 8,
            ..TrainableSetConfig::default()
        };
        let fit = || {
            CompleteSetMultiTaskEncoder::fit_with_pooling(
                Digest([7; 32]),
                Digest([8; 32]),
                Digest([9; 32]),
                vec!["forward_sum".into(), "inverse_difference".into()],
                &training,
                &held_out,
                config,
                MultiTaskSetPooling::MeanMaxLearnedAttention,
            )
            .unwrap()
        };
        let (report, mut model) = fit();
        let (_, repeated) = fit();
        assert_eq!(report.pooling, MultiTaskSetPooling::MeanMaxLearnedAttention);
        assert_eq!(report.held_out_attention.len(), LEARNED_ATTENTION_HEADS);
        assert_eq!(
            model.model_sha256().unwrap(),
            repeated.model_sha256().unwrap()
        );
        for head in &report.held_out_attention {
            assert_eq!(head.target, None);
            assert_eq!(head.conditioning, None);
            assert_eq!(head.observation_support, held_out.len());
            assert!(head.query_l2_norm.is_finite() && head.query_l2_norm > 0.0);
            assert!((0.0..=1.0).contains(&head.mean_normalized_entropy));
            assert!((0.0..=1.0).contains(&head.mean_maximum_weight));
        }

        let baseline = model.predict(&held_out[0]).unwrap();
        let mut permuted = held_out[0].clone();
        permuted.input.nodes.reverse();
        permuted.post_input.nodes.reverse();
        assert_eq!(baseline, model.predict(&permuted).unwrap());

        let queries_before = model.attention_queries.clone();
        model.train_one(&training[0]).unwrap();
        assert_ne!(queries_before, model.attention_queries);
    }

    #[test]
    fn task_attention_is_target_bound_phase_correct_and_permutation_invariant() {
        let training = corpus(1, 48);
        let held_out = corpus(100, 16);
        let config = TrainableSetConfig {
            epochs: 4,
            node_hidden_width: 8,
            head_hidden_width: 8,
            ..TrainableSetConfig::default()
        };
        let fit = || {
            CompleteSetMultiTaskEncoder::fit_with_pooling(
                Digest([7; 32]),
                Digest([8; 32]),
                Digest([9; 32]),
                vec!["forward_sum".into(), "inverse_difference".into()],
                &training,
                &held_out,
                config,
                MultiTaskSetPooling::MeanMaxTaskAttention,
            )
            .unwrap()
        };
        let (report, mut model) = fit();
        let (_, repeated) = fit();
        assert_eq!(report.pooling, MultiTaskSetPooling::MeanMaxTaskAttention);
        assert_eq!(report.held_out_attention.len(), 2);
        assert_eq!(
            model.model_sha256().unwrap(),
            repeated.model_sha256().unwrap()
        );
        for (target, head) in report.held_out_attention.iter().enumerate() {
            assert_eq!(
                head.target.as_deref(),
                Some(report.target_names[target].as_str())
            );
            assert_eq!(head.conditioning, Some(report.target_conditioning[target]));
            let phase_multiplier = usize::from(
                report.target_conditioning[target] == AuxiliaryHeadConditioning::PreAndPostState,
            ) + 1;
            assert_eq!(
                head.observation_support,
                report.target_support_held_out[target] * phase_multiplier
            );
        }

        let baseline = model.predict(&held_out[0]).unwrap();
        let mut permuted = held_out[0].clone();
        permuted.input.nodes.reverse();
        permuted.post_input.nodes.reverse();
        assert_eq!(baseline, model.predict(&permuted).unwrap());

        let mut changed_post = held_out[0].clone();
        changed_post.post_input.nodes[0].continuous[0] += 1000.0;
        assert_eq!(baseline[0], model.predict(&changed_post).unwrap()[0]);
        let mut changed_action = held_out[0].clone();
        changed_action.action_context.fill(0.75);
        assert_eq!(baseline[1], model.predict(&changed_action).unwrap()[1]);

        let queries_before = model.attention_queries.clone();
        model.train_one(&training[0]).unwrap();
        assert_ne!(queries_before, model.attention_queries);
    }

    #[test]
    fn rejects_cross_split_identity_and_unsupported_target() {
        let training = corpus(1, 8);
        let mut held_out = corpus(40, 4);
        held_out[0].input.sample_sha256 = training[0].input.sample_sha256;
        assert!(
            CompleteSetMultiTaskEncoder::fit(
                Digest([7; 32]),
                Digest([8; 32]),
                Digest([9; 32]),
                vec!["forward_sum".into(), "inverse_difference".into()],
                &training,
                &held_out,
                TrainableSetConfig::default(),
            )
            .is_err()
        );
        let mut unsupported = corpus(40, 4);
        for sample in &mut unsupported {
            sample.target_present[1] = false;
            sample.targets[1] = 0.0;
        }
        assert!(
            CompleteSetMultiTaskEncoder::fit(
                Digest([7; 32]),
                Digest([8; 32]),
                Digest([9; 32]),
                vec!["forward_sum".into(), "inverse_difference".into()],
                &training,
                &unsupported,
                TrainableSetConfig::default(),
            )
            .is_err()
        );
    }
}
