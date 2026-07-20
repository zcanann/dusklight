//! Evidence-gated variable actor-set encoders.

use crate::artifact::Digest;
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;

pub const ACTOR_SET_READINESS_SCHEMA_V1: &str = "dusklight-actor-set-readiness/v1";
pub const ACTOR_SET_ENCODING_SCHEMA_V2: &str = "dusklight-actor-set-encoding/v2";
/// Matches the authenticated native episode format. This is a serialization
/// limit, not a learner-side selection policy.
pub const ACTOR_SET_MAXIMUM_ACTORS: usize = u16::MAX as usize;
const MAX_ACTOR_WIDTH: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct ActorSetReadinessConfig {
    pub minimum_unique_episodes: usize,
    pub minimum_effective_decisions: u64,
    pub minimum_overflow_decisions: u64,
    pub maximum_fixed_slot_held_out_mse: f64,
    pub minimum_overflow_error_ratio: f64,
}

impl Default for ActorSetReadinessConfig {
    fn default() -> Self {
        Self {
            minimum_unique_episodes: 128,
            minimum_effective_decisions: 4096,
            minimum_overflow_decisions: 256,
            maximum_fixed_slot_held_out_mse: 0.05,
            minimum_overflow_error_ratio: 1.25,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct FixedSlotFailureEvidence {
    pub report_sha256: Digest,
    pub representation_sha256: Digest,
    pub held_out_mse: f64,
    pub overflow_subset_mse: f64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorSetReadinessDisposition {
    RetainFixedSlots,
    InsufficientCorpus,
    ReadyForSetEncoderComparison,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ActorSetReadinessReport {
    pub schema: &'static str,
    pub dataset_sha256: Digest,
    pub fixed_slot_failure: FixedSlotFailureEvidence,
    pub config: ActorSetReadinessConfig,
    pub unique_episodes: usize,
    pub effective_decisions: u64,
    pub actor_overflow_decisions: u64,
    pub content_disjoint_evaluation: bool,
    pub fixed_slot_failed: bool,
    pub overflow_error_ratio: f64,
    pub disposition: ActorSetReadinessDisposition,
    pub set_encoders_enabled: bool,
    pub promotion_authority: bool,
    pub readiness_sha256: Digest,
}

impl ActorSetReadinessReport {
    #[allow(clippy::too_many_arguments)]
    pub fn assess(
        dataset_sha256: Digest,
        fixed_slot_failure: FixedSlotFailureEvidence,
        unique_episodes: usize,
        effective_decisions: u64,
        actor_overflow_decisions: u64,
        content_disjoint_evaluation: bool,
        config: ActorSetReadinessConfig,
    ) -> Result<Self, ActorSetRepresentationError> {
        validate_readiness_inputs(dataset_sha256, fixed_slot_failure, config)?;
        let fixed_slot_failed =
            fixed_slot_failure.held_out_mse > config.maximum_fixed_slot_held_out_mse;
        let overflow_error_ratio = fixed_slot_failure.overflow_subset_mse
            / fixed_slot_failure.held_out_mse.max(f64::EPSILON);
        let corpus_ready = unique_episodes >= config.minimum_unique_episodes
            && effective_decisions >= config.minimum_effective_decisions
            && actor_overflow_decisions >= config.minimum_overflow_decisions
            && actor_overflow_decisions <= effective_decisions
            && content_disjoint_evaluation;
        let overflow_explains_failure = overflow_error_ratio >= config.minimum_overflow_error_ratio;
        let disposition = if !fixed_slot_failed || !overflow_explains_failure {
            ActorSetReadinessDisposition::RetainFixedSlots
        } else if !corpus_ready {
            ActorSetReadinessDisposition::InsufficientCorpus
        } else {
            ActorSetReadinessDisposition::ReadyForSetEncoderComparison
        };
        let mut report = Self {
            schema: ACTOR_SET_READINESS_SCHEMA_V1,
            dataset_sha256,
            fixed_slot_failure,
            config,
            unique_episodes,
            effective_decisions,
            actor_overflow_decisions,
            content_disjoint_evaluation,
            fixed_slot_failed,
            overflow_error_ratio,
            disposition,
            set_encoders_enabled: disposition
                == ActorSetReadinessDisposition::ReadyForSetEncoderComparison,
            promotion_authority: false,
            readiness_sha256: Digest::ZERO,
        };
        report.readiness_sha256 = report.digest()?;
        Ok(report)
    }

    fn digest(&self) -> Result<Digest, ActorSetRepresentationError> {
        canonical_digest(
            b"dusklight.actor-set-readiness/v1\0",
            &(
                self.schema,
                self.dataset_sha256,
                self.fixed_slot_failure,
                self.config,
                self.unique_episodes,
                self.effective_decisions,
                self.actor_overflow_decisions,
                self.content_disjoint_evaluation,
                self.fixed_slot_failed,
                self.overflow_error_ratio,
                self.disposition,
                self.set_encoders_enabled,
                self.promotion_authority,
            ),
        )
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ActorSetEncoding {
    pub schema: &'static str,
    pub actor_feature_schema_sha256: Digest,
    pub qualification: ActorSetEncoderQualification,
    pub readiness_sha256: Option<Digest>,
    pub actor_source_available: bool,
    pub actor_source_complete: bool,
    pub actor_count: usize,
    pub deepsets: Vec<f32>,
    pub objective_attention: Vec<f32>,
    pub encoding_sha256: Digest,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorSetEncoderQualification {
    /// Usable for training and controlled comparisons, but never promotion
    /// authority by itself.
    Exploratory,
    /// Bound to evidence that the fixed-slot baseline failed on a sufficiently
    /// large, content-disjoint overflow corpus.
    FixedSlotFailureQualified,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ActorSetEncoder {
    schema: &'static str,
    actor_feature_schema_sha256: Digest,
    qualification: ActorSetEncoderQualification,
    readiness_sha256: Option<Digest>,
    actor_width: usize,
    deepsets_width: usize,
    attention_width: usize,
    encoder_sha256: Digest,
}

impl ActorSetEncoder {
    /// Construct a complete-set candidate without pretending that it has won a
    /// promotion comparison. Building the representation must not depend on a
    /// different representation failing first.
    pub fn exploratory(
        actor_feature_schema_sha256: Digest,
        actor_width: usize,
    ) -> Result<Self, ActorSetRepresentationError> {
        Self::build(
            actor_feature_schema_sha256,
            actor_width,
            ActorSetEncoderQualification::Exploratory,
            None,
        )
    }

    pub fn from_readiness(
        readiness: &ActorSetReadinessReport,
        actor_feature_schema_sha256: Digest,
        actor_width: usize,
    ) -> Result<Self, ActorSetRepresentationError> {
        if readiness.schema != ACTOR_SET_READINESS_SCHEMA_V1
            || readiness.readiness_sha256 != readiness.digest()?
            || !readiness.set_encoders_enabled
            || readiness.disposition != ActorSetReadinessDisposition::ReadyForSetEncoderComparison
        {
            return Err(ActorSetRepresentationError::new(
                "actor-set encoder is not readiness-qualified",
            ));
        }
        Self::build(
            actor_feature_schema_sha256,
            actor_width,
            ActorSetEncoderQualification::FixedSlotFailureQualified,
            Some(readiness.readiness_sha256),
        )
    }

    fn build(
        actor_feature_schema_sha256: Digest,
        actor_width: usize,
        qualification: ActorSetEncoderQualification,
        readiness_sha256: Option<Digest>,
    ) -> Result<Self, ActorSetRepresentationError> {
        if actor_feature_schema_sha256 == Digest::ZERO
            || actor_width == 0
            || actor_width > MAX_ACTOR_WIDTH
        {
            return Err(ActorSetRepresentationError::new(
                "actor-set width is invalid",
            ));
        }
        let mut encoder = Self {
            schema: ACTOR_SET_ENCODING_SCHEMA_V2,
            actor_feature_schema_sha256,
            qualification,
            readiness_sha256,
            actor_width,
            // count/presence plus sum, mean, minimum, and maximum.
            deepsets_width: 2 + actor_width * 4,
            // count/presence, attended value, attention concentration.
            attention_width: 3 + actor_width,
            encoder_sha256: Digest::ZERO,
        };
        encoder.encoder_sha256 = canonical_digest(
            b"dusklight.actor-set-encoder/v2\0",
            &(
                encoder.schema,
                encoder.actor_feature_schema_sha256,
                encoder.qualification,
                encoder.readiness_sha256,
                encoder.actor_width,
                encoder.deepsets_width,
                encoder.attention_width,
            ),
        )?;
        Ok(encoder)
    }

    pub fn encode(
        &self,
        actor_feature_schema_sha256: Digest,
        actors: &[Vec<f32>],
        objective_query: &[f32],
        actor_source_available: bool,
        actor_source_truncated: bool,
    ) -> Result<ActorSetEncoding, ActorSetRepresentationError> {
        if actor_feature_schema_sha256 != self.actor_feature_schema_sha256
            || actors.len() > ACTOR_SET_MAXIMUM_ACTORS
            || objective_query.len() != self.actor_width
            || objective_query.iter().any(|value| !value.is_finite())
            || actors.iter().any(|actor| {
                actor.len() != self.actor_width || actor.iter().any(|value| !value.is_finite())
            })
            || actor_source_truncated
            || (!actor_source_available && !actors.is_empty())
        {
            return Err(ActorSetRepresentationError::new(
                "actor-set input is invalid",
            ));
        }
        let ordered_actors = canonical_actor_order(actors);
        let deepsets = deepsets_features(&ordered_actors, self.actor_width);
        let objective_attention =
            attention_features(&ordered_actors, objective_query, self.actor_width);
        debug_assert_eq!(deepsets.len(), self.deepsets_width);
        debug_assert_eq!(objective_attention.len(), self.attention_width);
        if deepsets
            .iter()
            .chain(&objective_attention)
            .any(|value| !value.is_finite())
        {
            return Err(ActorSetRepresentationError::new(
                "actor-set encoding is nonfinite",
            ));
        }
        let mut encoding = ActorSetEncoding {
            schema: ACTOR_SET_ENCODING_SCHEMA_V2,
            actor_feature_schema_sha256: self.actor_feature_schema_sha256,
            qualification: self.qualification,
            readiness_sha256: self.readiness_sha256,
            actor_source_available,
            actor_source_complete: actor_source_available,
            actor_count: actors.len(),
            deepsets,
            objective_attention,
            encoding_sha256: Digest::ZERO,
        };
        encoding.encoding_sha256 = canonical_digest(
            b"dusklight.actor-set-encoding/v2\0",
            &(
                encoding.schema,
                self.encoder_sha256,
                encoding.actor_feature_schema_sha256,
                encoding.qualification,
                encoding.readiness_sha256,
                encoding.actor_source_available,
                encoding.actor_source_complete,
                encoding.actor_count,
                &encoding.deepsets,
                &encoding.objective_attention,
            ),
        )?;
        Ok(encoding)
    }
}

fn canonical_actor_order(actors: &[Vec<f32>]) -> Vec<&[f32]> {
    let mut ordered = actors.iter().map(Vec::as_slice).collect::<Vec<_>>();
    ordered.sort_by(|left, right| {
        left.iter()
            .zip(*right)
            .map(|(left, right)| left.total_cmp(right))
            .find(|ordering| !ordering.is_eq())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    ordered
}

fn deepsets_features(actors: &[&[f32]], width: usize) -> Vec<f32> {
    let mut output = vec![
        f32::from(!actors.is_empty()),
        actors.len() as f32 / ACTOR_SET_MAXIMUM_ACTORS as f32,
    ];
    if actors.is_empty() {
        output.resize(2 + width * 4, 0.0);
        return output;
    }
    let mut sum = vec![0.0; width];
    let mut minimum = vec![f32::INFINITY; width];
    let mut maximum = vec![f32::NEG_INFINITY; width];
    for actor in actors {
        for index in 0..width {
            sum[index] += actor[index];
            minimum[index] = minimum[index].min(actor[index]);
            maximum[index] = maximum[index].max(actor[index]);
        }
    }
    let mean = sum
        .iter()
        .map(|value| *value / actors.len() as f32)
        .collect::<Vec<_>>();
    output.extend(sum);
    output.extend(mean);
    output.extend(minimum);
    output.extend(maximum);
    output
}

fn attention_features(actors: &[&[f32]], query: &[f32], width: usize) -> Vec<f32> {
    let mut output = vec![
        f32::from(!actors.is_empty()),
        actors.len() as f32 / ACTOR_SET_MAXIMUM_ACTORS as f32,
    ];
    if actors.is_empty() {
        output.resize(3 + width, 0.0);
        return output;
    }
    let scale = (width as f32).sqrt().max(1.0);
    let scores = actors
        .iter()
        .map(|actor| {
            actor
                .iter()
                .zip(query)
                .map(|(actor, query)| actor * query)
                .sum::<f32>()
                / scale
        })
        .collect::<Vec<_>>();
    let maximum = scores.iter().copied().max_by(f32::total_cmp).unwrap();
    let exponentials = scores
        .iter()
        .map(|score| (score - maximum).exp())
        .collect::<Vec<_>>();
    let denominator = exponentials.iter().sum::<f32>();
    let weights = exponentials
        .iter()
        .map(|value| *value / denominator)
        .collect::<Vec<_>>();
    let mut attended = vec![0.0; width];
    for (actor, weight) in actors.iter().zip(&weights) {
        for (output, value) in attended.iter_mut().zip(actor.iter()) {
            *output += value * weight;
        }
    }
    output.extend(attended);
    output.push(weights.into_iter().max_by(f32::total_cmp).unwrap());
    output
}

fn validate_readiness_inputs(
    dataset_sha256: Digest,
    evidence: FixedSlotFailureEvidence,
    config: ActorSetReadinessConfig,
) -> Result<(), ActorSetRepresentationError> {
    if dataset_sha256 == Digest::ZERO
        || evidence.report_sha256 == Digest::ZERO
        || evidence.representation_sha256 == Digest::ZERO
        || !evidence.held_out_mse.is_finite()
        || evidence.held_out_mse < 0.0
        || !evidence.overflow_subset_mse.is_finite()
        || evidence.overflow_subset_mse < 0.0
        || config.minimum_unique_episodes == 0
        || config.minimum_effective_decisions == 0
        || config.minimum_overflow_decisions == 0
        || !config.maximum_fixed_slot_held_out_mse.is_finite()
        || config.maximum_fixed_slot_held_out_mse < 0.0
        || !config.minimum_overflow_error_ratio.is_finite()
        || config.minimum_overflow_error_ratio < 1.0
    {
        return Err(ActorSetRepresentationError::new(
            "actor-set readiness input is invalid",
        ));
    }
    Ok(())
}

fn canonical_digest<T: Serialize>(
    domain: &[u8],
    value: &T,
) -> Result<Digest, ActorSetRepresentationError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| ActorSetRepresentationError::new(error.to_string()))?;
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(bytes);
    Ok(Digest(hasher.finalize().into()))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActorSetRepresentationError(String);

impl ActorSetRepresentationError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for ActorSetRepresentationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for ActorSetRepresentationError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn failure() -> FixedSlotFailureEvidence {
        FixedSlotFailureEvidence {
            report_sha256: Digest([1; 32]),
            representation_sha256: Digest([2; 32]),
            held_out_mse: 0.2,
            overflow_subset_mse: 0.4,
        }
    }

    #[test]
    fn set_encoders_require_fixed_slot_failure_and_large_overflow_corpus() {
        let config = ActorSetReadinessConfig::default();
        let small =
            ActorSetReadinessReport::assess(Digest([3; 32]), failure(), 10, 100, 20, true, config)
                .unwrap();
        assert_eq!(
            small.disposition,
            ActorSetReadinessDisposition::InsufficientCorpus
        );
        assert!(ActorSetEncoder::from_readiness(&small, Digest([4; 32]), 2).is_err());

        let mut passing_fixed_slots = failure();
        passing_fixed_slots.held_out_mse = 0.01;
        let retained = ActorSetReadinessReport::assess(
            Digest([3; 32]),
            passing_fixed_slots,
            200,
            5000,
            500,
            true,
            config,
        )
        .unwrap();
        assert_eq!(
            retained.disposition,
            ActorSetReadinessDisposition::RetainFixedSlots
        );
        assert!(ActorSetEncoder::from_readiness(&retained, Digest([4; 32]), 2).is_err());
    }

    #[test]
    fn qualified_deepsets_and_attention_are_permutation_invariant() {
        let readiness = ActorSetReadinessReport::assess(
            Digest([3; 32]),
            failure(),
            200,
            5000,
            500,
            true,
            ActorSetReadinessConfig::default(),
        )
        .unwrap();
        assert!(readiness.set_encoders_enabled);
        let encoder = ActorSetEncoder::from_readiness(&readiness, Digest([4; 32]), 2).unwrap();
        let actors = vec![vec![1.0, 0.0], vec![0.0, 2.0], vec![3.0, 1.0]];
        let reversed = actors.iter().cloned().rev().collect::<Vec<_>>();
        let first = encoder
            .encode(Digest([4; 32]), &actors, &[1.0, 0.0], true, false)
            .unwrap();
        let second = encoder
            .encode(Digest([4; 32]), &reversed, &[1.0, 0.0], true, false)
            .unwrap();
        assert_eq!(first, second);
        assert_eq!(
            first.qualification,
            ActorSetEncoderQualification::FixedSlotFailureQualified
        );
        assert_eq!(first.readiness_sha256, Some(readiness.readiness_sha256));
        assert_eq!(first.actor_feature_schema_sha256, Digest([4; 32]));
        assert!(first.actor_source_complete);
        assert_ne!(first.encoding_sha256, Digest::ZERO);
        assert_eq!(first.actor_count, 3);
        assert!(first.objective_attention[2] > 2.0);
    }

    #[test]
    fn exploratory_encoder_consumes_complete_populations_above_controller_capacity() {
        let encoder = ActorSetEncoder::exploratory(Digest([4; 32]), 3).unwrap();
        // The bounded native tactic/controller view has 256 slots, while the
        // canonical learning observation is explicitly allowed to exceed it.
        let actors = (0..257)
            .map(|index| vec![index as f32 / 257.0, (index % 7) as f32, 1.0])
            .collect::<Vec<_>>();
        let reversed = actors.iter().cloned().rev().collect::<Vec<_>>();
        let first = encoder
            .encode(Digest([4; 32]), &actors, &[0.25, 0.5, 1.0], true, false)
            .unwrap();
        let second = encoder
            .encode(Digest([4; 32]), &reversed, &[0.25, 0.5, 1.0], true, false)
            .unwrap();
        assert_eq!(first, second);
        assert_eq!(first.actor_count, 257);
        assert!(first.actor_source_complete);
        assert_eq!(
            first.qualification,
            ActorSetEncoderQualification::Exploratory
        );
        assert_eq!(first.readiness_sha256, None);
    }

    #[test]
    fn complete_set_encoder_rejects_truncation_and_nonfinite_aggregation() {
        let encoder = ActorSetEncoder::exploratory(Digest([4; 32]), 1).unwrap();
        assert!(
            encoder
                .encode(Digest([4; 32]), &[vec![1.0]], &[1.0], true, true)
                .is_err()
        );
        assert!(
            encoder
                .encode(
                    Digest([4; 32]),
                    &[vec![f32::MAX], vec![f32::MAX]],
                    &[1.0],
                    true,
                    false,
                )
                .is_err()
        );
        assert!(
            encoder
                .encode(Digest([4; 32]), &[vec![1.0]], &[1.0], false, false)
                .is_err()
        );
        assert!(
            encoder
                .encode(Digest([5; 32]), &[vec![1.0]], &[1.0], true, false)
                .is_err()
        );

        let absent = encoder
            .encode(Digest([4; 32]), &[], &[1.0], false, false)
            .unwrap();
        assert!(!absent.actor_source_available);
        assert!(!absent.actor_source_complete);
        assert_eq!(absent.actor_count, 0);
    }
}
