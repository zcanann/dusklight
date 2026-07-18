//! Immutable online dataset generations and exact model ancestry.

use crate::artifact::Digest;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

pub const DATASET_GENERATION_SCHEMA_V1: &str = "dusklight-dataset-generation/v1";
pub const MODEL_LINEAGE_SCHEMA_V1: &str = "dusklight-model-lineage/v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DatasetGeneration {
    pub schema: String,
    pub generation: u32,
    pub parent_generation_sha256: Option<Digest>,
    pub corpus_sha256: Vec<Digest>,
    pub generation_sha256: Digest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelLineage {
    pub schema: String,
    pub generation: u32,
    pub dataset_generation_sha256: Digest,
    pub model_artifact_sha256: Digest,
    pub parent_model_artifact_sha256: Option<Digest>,
    pub learner_config_sha256: Digest,
    pub lineage_sha256: Digest,
}

impl DatasetGeneration {
    pub fn initial(corpora: impl IntoIterator<Item = Digest>) -> Result<Self, LineageError> {
        Self::build(0, None, corpora)
    }

    pub fn next(&self, corpora: impl IntoIterator<Item = Digest>) -> Result<Self, LineageError> {
        self.validate()?;
        let generation = self
            .generation
            .checked_add(1)
            .ok_or(LineageError::GenerationOverflow)?;
        let next = Self::build(generation, Some(self.generation_sha256), corpora)?;
        let previous = self.corpus_sha256.iter().copied().collect::<BTreeSet<_>>();
        let current = next.corpus_sha256.iter().copied().collect::<BTreeSet<_>>();
        if !previous.is_subset(&current) {
            return Err(LineageError::DatasetRegression);
        }
        Ok(next)
    }

    pub fn validate(&self) -> Result<(), LineageError> {
        if self.schema != DATASET_GENERATION_SCHEMA_V1
            || self.corpus_sha256.is_empty()
            || self
                .corpus_sha256
                .iter()
                .any(|digest| *digest == Digest::ZERO)
            || self.corpus_sha256.windows(2).any(|pair| pair[0] >= pair[1])
            || (self.generation == 0) != self.parent_generation_sha256.is_none()
            || self.parent_generation_sha256 == Some(Digest::ZERO)
            || self.generation_sha256 != self.identity()
        {
            return Err(LineageError::InvalidDatasetGeneration);
        }
        Ok(())
    }

    fn build(
        generation: u32,
        parent_generation_sha256: Option<Digest>,
        corpora: impl IntoIterator<Item = Digest>,
    ) -> Result<Self, LineageError> {
        let corpus_sha256 = corpora.into_iter().collect::<BTreeSet<_>>();
        if corpus_sha256.is_empty() || corpus_sha256.contains(&Digest::ZERO) {
            return Err(LineageError::InvalidDatasetGeneration);
        }
        let mut value = Self {
            schema: DATASET_GENERATION_SCHEMA_V1.into(),
            generation,
            parent_generation_sha256,
            corpus_sha256: corpus_sha256.into_iter().collect(),
            generation_sha256: Digest::ZERO,
        };
        value.generation_sha256 = value.identity();
        value.validate()?;
        Ok(value)
    }

    fn identity(&self) -> Digest {
        hash_parts(
            b"dusklight.dataset-generation/v1\0",
            std::iter::once(self.generation.to_le_bytes().to_vec())
                .chain(std::iter::once(
                    self.parent_generation_sha256
                        .unwrap_or(Digest::ZERO)
                        .0
                        .to_vec(),
                ))
                .chain(self.corpus_sha256.iter().map(|digest| digest.0.to_vec())),
        )
    }
}

impl ModelLineage {
    pub fn build(
        dataset: &DatasetGeneration,
        model_artifact: &[u8],
        learner_config: &[u8],
        parent: Option<&ModelLineage>,
    ) -> Result<Self, LineageError> {
        dataset.validate()?;
        let parent_model_artifact_sha256 = match (dataset.generation, parent) {
            (0, None) => None,
            (0, Some(_)) | (_, None) => return Err(LineageError::DetachedModelParent),
            (_, Some(parent)) => {
                parent.validate()?;
                if parent.generation.checked_add(1) != Some(dataset.generation)
                    || dataset.parent_generation_sha256 != Some(parent.dataset_generation_sha256)
                {
                    return Err(LineageError::DetachedModelParent);
                }
                Some(parent.model_artifact_sha256)
            }
        };
        if model_artifact.is_empty() || learner_config.is_empty() {
            return Err(LineageError::InvalidModelLineage);
        }
        let mut value = Self {
            schema: MODEL_LINEAGE_SCHEMA_V1.into(),
            generation: dataset.generation,
            dataset_generation_sha256: dataset.generation_sha256,
            model_artifact_sha256: Digest(Sha256::digest(model_artifact).into()),
            parent_model_artifact_sha256,
            learner_config_sha256: Digest(Sha256::digest(learner_config).into()),
            lineage_sha256: Digest::ZERO,
        };
        value.lineage_sha256 = value.identity();
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), LineageError> {
        if self.schema != MODEL_LINEAGE_SCHEMA_V1
            || self.dataset_generation_sha256 == Digest::ZERO
            || self.model_artifact_sha256 == Digest::ZERO
            || self.learner_config_sha256 == Digest::ZERO
            || (self.generation == 0) != self.parent_model_artifact_sha256.is_none()
            || self.parent_model_artifact_sha256 == Some(Digest::ZERO)
            || self.lineage_sha256 != self.identity()
        {
            return Err(LineageError::InvalidModelLineage);
        }
        Ok(())
    }

    pub fn validate_resume(
        &self,
        dataset: &DatasetGeneration,
        model_artifact: &[u8],
        learner_config: &[u8],
    ) -> Result<(), LineageError> {
        self.validate()?;
        dataset.validate()?;
        if self.generation != dataset.generation
            || self.dataset_generation_sha256 != dataset.generation_sha256
            || self.model_artifact_sha256 != Digest(Sha256::digest(model_artifact).into())
            || self.learner_config_sha256 != Digest(Sha256::digest(learner_config).into())
        {
            return Err(LineageError::ResumeIdentityMismatch);
        }
        Ok(())
    }

    fn identity(&self) -> Digest {
        hash_parts(
            b"dusklight.model-lineage/v1\0",
            [
                self.generation.to_le_bytes().to_vec(),
                self.dataset_generation_sha256.0.to_vec(),
                self.model_artifact_sha256.0.to_vec(),
                self.parent_model_artifact_sha256
                    .unwrap_or(Digest::ZERO)
                    .0
                    .to_vec(),
                self.learner_config_sha256.0.to_vec(),
            ],
        )
    }
}

fn hash_parts(domain: &[u8], parts: impl IntoIterator<Item = Vec<u8>>) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    for part in parts {
        hasher.update((part.len() as u64).to_le_bytes());
        hasher.update(part);
    }
    Digest(hasher.finalize().into())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LineageError {
    GenerationOverflow,
    InvalidDatasetGeneration,
    DatasetRegression,
    InvalidModelLineage,
    DetachedModelParent,
    ResumeIdentityMismatch,
}

impl fmt::Display for LineageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "training lineage rejected: {self:?}")
    }
}

impl Error for LineageError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(value: u8) -> Digest {
        Digest([value; 32])
    }

    #[test]
    fn dataset_generations_are_immutable_monotonic_snapshots() {
        let first = DatasetGeneration::initial([digest(1)]).unwrap();
        let second = first.next([digest(2), digest(1)]).unwrap();
        assert_eq!(
            second.parent_generation_sha256,
            Some(first.generation_sha256)
        );
        assert_eq!(second.corpus_sha256, vec![digest(1), digest(2)]);
        assert_eq!(
            first.next([digest(2)]),
            Err(LineageError::DatasetRegression)
        );
    }

    #[test]
    fn resume_requires_exact_dataset_model_and_config() {
        let first_dataset = DatasetGeneration::initial([digest(1)]).unwrap();
        let first = ModelLineage::build(&first_dataset, b"model-0", b"config", None).unwrap();
        first
            .validate_resume(&first_dataset, b"model-0", b"config")
            .unwrap();
        assert_eq!(
            first.validate_resume(&first_dataset, b"changed", b"config"),
            Err(LineageError::ResumeIdentityMismatch)
        );

        let second_dataset = first_dataset.next([digest(1), digest(2)]).unwrap();
        let second =
            ModelLineage::build(&second_dataset, b"model-1", b"config", Some(&first)).unwrap();
        assert_eq!(
            second.parent_model_artifact_sha256,
            Some(first.model_artifact_sha256)
        );
    }

    #[test]
    fn detached_parent_is_rejected() {
        let first_dataset = DatasetGeneration::initial([digest(1)]).unwrap();
        let first = ModelLineage::build(&first_dataset, b"model-0", b"config", None).unwrap();
        let second_dataset = first_dataset.next([digest(1), digest(2)]).unwrap();
        let unrelated_dataset = DatasetGeneration::initial([digest(3)]).unwrap();
        let unrelated = ModelLineage::build(&unrelated_dataset, b"other", b"config", None).unwrap();
        assert_eq!(
            ModelLineage::build(&second_dataset, b"model-1", b"config", Some(&unrelated)),
            Err(LineageError::DetachedModelParent)
        );
        assert!(ModelLineage::build(&second_dataset, b"model-1", b"config", Some(&first)).is_ok());
    }
}
