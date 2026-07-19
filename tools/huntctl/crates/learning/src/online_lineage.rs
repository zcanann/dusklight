//! Immutable online-dataset generations and exact deterministic-refit lineage.

use super::evaluation_isolation::EvaluationGenerationSeal;
use crate::artifact::Digest;
use crate::transition_corpus::TransitionCorpus;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

pub const ONLINE_DATASET_GENERATION_SCHEMA_V1: &str = "dusklight-online-dataset-generation/v1";
pub const ONLINE_MODEL_LINEAGE_SCHEMA_V2: &str = "dusklight-online-model-lineage/v2";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OnlineDatasetGeneration {
    pub schema: String,
    pub generation: u32,
    pub parent_generation_sha256: Option<Digest>,
    pub source_evaluation_generation: u32,
    pub source_evaluation_seal_sha256: Digest,
    pub source_sealed_corpus_sha256: Vec<Digest>,
    pub added_corpus_sha256: Vec<Digest>,
    pub cumulative_corpus_sha256: Vec<Digest>,
    pub feature_schema_sha256: Digest,
    pub action_schema_sha256: Digest,
    pub generation_sha256: Digest,
}

impl OnlineDatasetGeneration {
    pub fn build(
        previous: Option<&Self>,
        evaluation_seal: &EvaluationGenerationSeal,
        corpora: &[TransitionCorpus],
    ) -> Result<Self, OnlineLineageError> {
        let generation = evaluation_seal.minimum_training_generation;
        let corpus_identities = corpus_identities(corpora)?;
        let cumulative = corpus_identities
            .iter()
            .map(|(digest, _, _)| *digest)
            .collect::<BTreeSet<_>>();
        let sealed = evaluation_seal
            .admitted_corpora
            .iter()
            .map(|corpus| corpus.transition_corpus_sha256)
            .collect::<BTreeSet<_>>();
        evaluation_seal
            .admit_training_generation(generation, &sealed)
            .map_err(|error| OnlineLineageError::new(error.to_string()))?;
        if !sealed.is_subset(&cumulative) {
            return Err(OnlineLineageError::new(
                "sealed evaluation corpus is absent from the cumulative dataset",
            ));
        }

        let (parent_generation_sha256, previous_cumulative) = match previous {
            Some(previous) => {
                previous.validate()?;
                if previous.generation.checked_add(1) != Some(generation) {
                    return Err(OnlineLineageError::new(
                        "online dataset generations are not consecutive",
                    ));
                }
                (
                    Some(previous.generation_sha256),
                    previous
                        .cumulative_corpus_sha256
                        .iter()
                        .copied()
                        .collect::<BTreeSet<_>>(),
                )
            }
            None => (None, BTreeSet::new()),
        };
        if !previous_cumulative.is_subset(&cumulative)
            || cumulative != previous_cumulative.union(&sealed).copied().collect()
        {
            return Err(OnlineLineageError::new(
                "online dataset generation is not the immutable parent plus its sealed delta",
            ));
        }
        let added = cumulative
            .difference(&previous_cumulative)
            .copied()
            .collect::<Vec<_>>();
        let feature_schemas = corpus_identities
            .iter()
            .map(|(_, feature, _)| *feature)
            .collect::<BTreeSet<_>>();
        let action_schemas = corpus_identities
            .iter()
            .map(|(_, _, action)| *action)
            .collect::<BTreeSet<_>>();
        if feature_schemas.len() != 1 || action_schemas.len() != 1 {
            return Err(OnlineLineageError::new(
                "online dataset generation mixes corpus schemas",
            ));
        }
        let mut generation_manifest = Self {
            schema: ONLINE_DATASET_GENERATION_SCHEMA_V1.into(),
            generation,
            parent_generation_sha256,
            source_evaluation_generation: evaluation_seal.evaluation_generation,
            source_evaluation_seal_sha256: evaluation_seal.seal_sha256,
            source_sealed_corpus_sha256: sealed.into_iter().collect(),
            added_corpus_sha256: added,
            cumulative_corpus_sha256: cumulative.into_iter().collect(),
            feature_schema_sha256: *feature_schemas.first().expect("one feature schema"),
            action_schema_sha256: *action_schemas.first().expect("one action schema"),
            generation_sha256: Digest::ZERO,
        };
        generation_manifest.generation_sha256 = generation_manifest.digest()?;
        generation_manifest.validate()?;
        Ok(generation_manifest)
    }

    pub fn validate_corpora(&self, corpora: &[TransitionCorpus]) -> Result<(), OnlineLineageError> {
        self.validate()?;
        let identities = corpus_identities(corpora)?;
        let digests = identities
            .iter()
            .map(|(digest, _, _)| *digest)
            .collect::<BTreeSet<_>>();
        if digests != self.cumulative_corpus_sha256.iter().copied().collect()
            || identities.iter().any(|(_, feature, action)| {
                *feature != self.feature_schema_sha256 || *action != self.action_schema_sha256
            })
        {
            return Err(OnlineLineageError::new(
                "training corpora do not match the immutable dataset generation",
            ));
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<(), OnlineLineageError> {
        let cumulative = self
            .cumulative_corpus_sha256
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        let added = self
            .added_corpus_sha256
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        let sealed = self
            .source_sealed_corpus_sha256
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        if self.schema != ONLINE_DATASET_GENERATION_SCHEMA_V1
            || self.generation == 0
            || cumulative.is_empty()
            || cumulative.len() != self.cumulative_corpus_sha256.len()
            || added.len() != self.added_corpus_sha256.len()
            || sealed.len() != self.source_sealed_corpus_sha256.len()
            || !added.is_subset(&sealed)
            || !sealed.is_subset(&cumulative)
            || self.feature_schema_sha256 == Digest::ZERO
            || self.action_schema_sha256 == Digest::ZERO
            || self.generation_sha256 != self.digest()?
        {
            return Err(OnlineLineageError::new(
                "online dataset generation identity is invalid",
            ));
        }
        Ok(())
    }

    fn digest(&self) -> Result<Digest, OnlineLineageError> {
        canonical_digest(
            b"dusklight.online-dataset-generation/v1\0",
            &(
                &self.schema,
                self.generation,
                self.parent_generation_sha256,
                self.source_evaluation_generation,
                self.source_evaluation_seal_sha256,
                &self.source_sealed_corpus_sha256,
                &self.added_corpus_sha256,
                &self.cumulative_corpus_sha256,
                self.feature_schema_sha256,
                self.action_schema_sha256,
            ),
        )
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OnlineModelLineage {
    pub schema: String,
    pub generation: u32,
    pub dataset_generation_sha256: Digest,
    pub parent_lineage_sha256: Option<Digest>,
    pub parent_model_sha256: Option<Digest>,
    pub model_schema: String,
    pub training_mode: String,
    pub training_config: serde_json::Value,
    pub training_config_sha256: Digest,
    pub model_sha256: Digest,
    pub lineage_sha256: Digest,
}

impl OnlineModelLineage {
    pub fn build<C: Serialize, M: Serialize>(
        dataset: &OnlineDatasetGeneration,
        previous: Option<&Self>,
        model_schema: impl Into<String>,
        training_config: &C,
        model: &M,
    ) -> Result<Self, OnlineLineageError> {
        dataset.validate()?;
        let (parent_lineage_sha256, parent_model_sha256) = match previous {
            Some(previous) => {
                previous.validate()?;
                if previous.generation >= dataset.generation {
                    return Err(OnlineLineageError::new(
                        "parent model lineage is not older than the dataset generation",
                    ));
                }
                (Some(previous.lineage_sha256), Some(previous.model_sha256))
            }
            None => (None, None),
        };
        let training_config = serde_json::to_value(training_config)
            .map_err(|error| OnlineLineageError::new(error.to_string()))?;
        let mut lineage = Self {
            schema: ONLINE_MODEL_LINEAGE_SCHEMA_V2.into(),
            generation: dataset.generation,
            dataset_generation_sha256: dataset.generation_sha256,
            parent_lineage_sha256,
            parent_model_sha256,
            model_schema: model_schema.into(),
            training_mode: "deterministic_full_refit".into(),
            training_config: training_config.clone(),
            training_config_sha256: canonical_digest(
                b"dusklight.online-training-config/v1\0",
                &training_config,
            )?,
            model_sha256: canonical_digest(b"dusklight.online-model/v1\0", model)?,
            lineage_sha256: Digest::ZERO,
        };
        lineage.lineage_sha256 = lineage.digest()?;
        lineage.validate()?;
        Ok(lineage)
    }

    pub fn validate_resume<C: Serialize, M: Serialize>(
        &self,
        dataset: &OnlineDatasetGeneration,
        training_config: &C,
        model: &M,
    ) -> Result<(), OnlineLineageError> {
        self.validate()?;
        dataset.validate()?;
        let training_config = serde_json::to_value(training_config)
            .map_err(|error| OnlineLineageError::new(error.to_string()))?;
        if self.generation != dataset.generation
            || self.dataset_generation_sha256 != dataset.generation_sha256
            || self.training_config_sha256
                != canonical_digest(b"dusklight.online-training-config/v1\0", &training_config)?
            || self.model_sha256 != canonical_digest(b"dusklight.online-model/v1\0", model)?
        {
            return Err(OnlineLineageError::new(
                "resumed dataset, training configuration, or deterministic model differs from lineage",
            ));
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<(), OnlineLineageError> {
        if self.schema != ONLINE_MODEL_LINEAGE_SCHEMA_V2
            || self.generation == 0
            || self.dataset_generation_sha256 == Digest::ZERO
            || self.model_schema.is_empty()
            || self.training_mode != "deterministic_full_refit"
            || self.training_config.is_null()
            || self.training_config_sha256
                != canonical_digest(
                    b"dusklight.online-training-config/v1\0",
                    &self.training_config,
                )?
            || self.training_config_sha256 == Digest::ZERO
            || self.model_sha256 == Digest::ZERO
            || self.parent_lineage_sha256.is_some() != self.parent_model_sha256.is_some()
            || self.lineage_sha256 != self.digest()?
        {
            return Err(OnlineLineageError::new(
                "online model lineage identity is invalid",
            ));
        }
        Ok(())
    }

    fn digest(&self) -> Result<Digest, OnlineLineageError> {
        canonical_digest(
            b"dusklight.online-model-lineage/v1\0",
            &(
                &self.schema,
                self.generation,
                self.dataset_generation_sha256,
                self.parent_lineage_sha256,
                self.parent_model_sha256,
                &self.model_schema,
                &self.training_mode,
                &self.training_config,
                self.training_config_sha256,
                self.model_sha256,
            ),
        )
    }
}

fn corpus_identities(
    corpora: &[TransitionCorpus],
) -> Result<Vec<(Digest, Digest, Digest)>, OnlineLineageError> {
    if corpora.is_empty() {
        return Err(OnlineLineageError::new(
            "online dataset generation has no corpora",
        ));
    }
    corpora
        .iter()
        .map(|corpus| {
            corpus
                .validate()
                .map_err(|error| OnlineLineageError::new(error.to_string()))?;
            Ok((
                corpus
                    .content_digest()
                    .map_err(|error| OnlineLineageError::new(error.to_string()))?,
                corpus.feature_schema,
                corpus.action_schema,
            ))
        })
        .collect()
}

fn canonical_digest<T: Serialize>(domain: &[u8], value: &T) -> Result<Digest, OnlineLineageError> {
    let bytes =
        serde_json::to_vec(value).map_err(|error| OnlineLineageError::new(error.to_string()))?;
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
    Ok(Digest(hasher.finalize().into()))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OnlineLineageError(String);

impl OnlineLineageError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for OnlineLineageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for OnlineLineageError {}

#[cfg(test)]
mod tests {
    use super::super::evaluation_isolation::{EvaluationAttemptInput, EvaluationGenerationSeal};
    use super::*;
    use crate::transition_corpus::{MacroAction, StateReference, StateReferenceKind, Transition};

    fn corpus(value: u8) -> TransitionCorpus {
        TransitionCorpus::new(
            Digest([7; 32]),
            Digest([8; 32]),
            1,
            vec![Transition {
                source: StateReference {
                    kind: StateReferenceKind::Snapshot,
                    digest: Digest([value; 32]),
                },
                state: vec![f32::from(value)],
                action: MacroAction {
                    action_id: u32::from(value),
                    macro_kind: 1,
                    parameters: Vec::new(),
                },
                duration_ticks: 1,
                reward: 0.0,
                next: StateReference {
                    kind: StateReferenceKind::Snapshot,
                    digest: Digest([value.saturating_add(1); 32]),
                },
                next_state: vec![f32::from(value) + 1.0],
                terminal: true,
            }],
        )
        .unwrap()
    }

    fn seal(evaluation_generation: u32, corpus: &TransitionCorpus) -> EvaluationGenerationSeal {
        let digest = corpus.content_digest().unwrap();
        EvaluationGenerationSeal::build(
            evaluation_generation,
            2,
            2,
            2,
            0,
            &[
                EvaluationAttemptInput {
                    candidate_id: "candidate-a".into(),
                    attempt: 1,
                    worker_id: "evaluation/worker-0".into(),
                    transition_corpus_sha256: Some(digest),
                },
                EvaluationAttemptInput {
                    candidate_id: "candidate-a".into(),
                    attempt: 2,
                    worker_id: "evaluation/worker-1".into(),
                    transition_corpus_sha256: None,
                },
            ],
        )
        .unwrap()
    }

    #[test]
    fn generation_is_exact_parent_plus_sealed_delta() {
        let first_corpus = corpus(1);
        let first =
            OnlineDatasetGeneration::build(None, &seal(0, &first_corpus), &[first_corpus.clone()])
                .unwrap();
        let second_corpus = corpus(2);
        let second = OnlineDatasetGeneration::build(
            Some(&first),
            &seal(1, &second_corpus),
            &[first_corpus.clone(), second_corpus.clone()],
        )
        .unwrap();
        assert_eq!(
            second.parent_generation_sha256,
            Some(first.generation_sha256)
        );
        assert!(
            OnlineDatasetGeneration::build(
                Some(&first),
                &seal(1, &second_corpus),
                &[second_corpus]
            )
            .is_err()
        );
        second.validate_corpora(&[first_corpus, corpus(2)]).unwrap();
    }

    #[test]
    fn deterministic_refit_must_match_exact_resume_lineage() {
        let corpus = corpus(1);
        let dataset = OnlineDatasetGeneration::build(None, &seal(0, &corpus), &[corpus]).unwrap();
        let config = serde_json::json!({"iterations": 4, "seed": 9});
        let model = serde_json::json!({"trees": [1, 2, 3]});
        let lineage = OnlineModelLineage::build(
            &dataset,
            None,
            "dusklight-fitted-q-model/v2",
            &config,
            &model,
        )
        .unwrap();
        lineage.validate_resume(&dataset, &config, &model).unwrap();
        assert!(
            lineage
                .validate_resume(
                    &dataset,
                    &serde_json::json!({"iterations": 5, "seed": 9}),
                    &model,
                )
                .is_err()
        );
        let encoded = serde_json::to_vec(&lineage).unwrap();
        let decoded: OnlineModelLineage = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded, lineage);
        assert_eq!(decoded.training_config, config);
    }
}
