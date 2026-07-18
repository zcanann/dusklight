//! Authenticated learned-Q child-ordering hints for native beam search.

use crate::artifact::Digest;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

pub const Q_BEAM_PRIOR_SCHEMA_V1: &str = "dusklight-q-beam-priors/v1";
const MAX_PRIOR_ENTRIES: usize = 1_000_000;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct QBeamPrior {
    pub parent_candidate_id: String,
    pub option_index: usize,
    pub q_value: f64,
    pub ensemble_standard_deviation: f64,
    pub observed_support: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct QBeamPriorTable {
    pub schema: String,
    pub model_sha256: Digest,
    pub feature_schema_sha256: Digest,
    pub action_schema_sha256: Digest,
    pub objective_sha256: Digest,
    pub option_catalog_sha256: Digest,
    pub uncertainty_penalty: f64,
    pub entries: Vec<QBeamPrior>,
    pub route_authority: bool,
    pub promotion_authority: bool,
    pub table_sha256: Digest,
}

impl QBeamPriorTable {
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        model_sha256: Digest,
        feature_schema_sha256: Digest,
        action_schema_sha256: Digest,
        objective_sha256: Digest,
        option_catalog_sha256: Digest,
        uncertainty_penalty: f64,
        entries: Vec<QBeamPrior>,
    ) -> Result<Self, PlanningPriorError> {
        let mut table = Self {
            schema: Q_BEAM_PRIOR_SCHEMA_V1.into(),
            model_sha256,
            feature_schema_sha256,
            action_schema_sha256,
            objective_sha256,
            option_catalog_sha256,
            uncertainty_penalty,
            entries,
            route_authority: false,
            promotion_authority: false,
            table_sha256: Digest::ZERO,
        };
        table.canonicalize();
        table.table_sha256 = table.digest()?;
        table.validate_for_catalog(option_catalog_sha256, usize::MAX)?;
        Ok(table)
    }

    pub fn validate_for_catalog(
        &self,
        option_catalog_sha256: Digest,
        option_count: usize,
    ) -> Result<(), PlanningPriorError> {
        if self.schema != Q_BEAM_PRIOR_SCHEMA_V1
            || [
                self.model_sha256,
                self.feature_schema_sha256,
                self.action_schema_sha256,
                self.objective_sha256,
                self.option_catalog_sha256,
            ]
            .contains(&Digest::ZERO)
            || self.option_catalog_sha256 != option_catalog_sha256
            || !self.uncertainty_penalty.is_finite()
            || self.uncertainty_penalty < 0.0
            || self.entries.is_empty()
            || self.entries.len() > MAX_PRIOR_ENTRIES
            || self.route_authority
            || self.promotion_authority
            || self.table_sha256 != self.digest()?
        {
            return Err(PlanningPriorError::new(
                "Q beam prior table identity or authority is invalid",
            ));
        }
        let mut keys = BTreeSet::new();
        for entry in &self.entries {
            if entry.parent_candidate_id.len() != 64
                || !entry
                    .parent_candidate_id
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
                || entry.option_index >= option_count
                || !entry.q_value.is_finite()
                || !entry.ensemble_standard_deviation.is_finite()
                || entry.ensemble_standard_deviation < 0.0
                || entry.observed_support == 0
                || !keys.insert((entry.parent_candidate_id.as_str(), entry.option_index))
            {
                return Err(PlanningPriorError::new(
                    "Q beam prior entry is invalid, unsupported, or duplicated",
                ));
            }
        }
        Ok(())
    }

    pub fn ranked_option_indices(
        &self,
        parent_candidate_id: &str,
        option_count: usize,
    ) -> Vec<usize> {
        let mut indices = (0..option_count).collect::<Vec<_>>();
        indices.sort_by(|left_index, right_index| {
            let left = self.score(parent_candidate_id, *left_index);
            let right = self.score(parent_candidate_id, *right_index);
            match (left, right) {
                (Some(left), Some(right)) => right
                    .total_cmp(&left)
                    .then_with(|| left_index.cmp(right_index)),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => left_index.cmp(right_index),
            }
        });
        indices
    }

    pub fn has_prior(&self, parent_candidate_id: &str, option_index: usize) -> bool {
        self.entry(parent_candidate_id, option_index).is_some()
    }

    fn score(&self, parent_candidate_id: &str, option_index: usize) -> Option<f64> {
        self.entry(parent_candidate_id, option_index).map(|entry| {
            entry.q_value - self.uncertainty_penalty * entry.ensemble_standard_deviation
        })
    }

    fn entry(&self, parent_candidate_id: &str, option_index: usize) -> Option<&QBeamPrior> {
        self.entries
            .binary_search_by(|entry| {
                entry
                    .parent_candidate_id
                    .as_str()
                    .cmp(parent_candidate_id)
                    .then_with(|| entry.option_index.cmp(&option_index))
            })
            .ok()
            .map(|index| &self.entries[index])
    }

    fn canonicalize(&mut self) {
        self.entries.sort_by(|left, right| {
            left.parent_candidate_id
                .cmp(&right.parent_candidate_id)
                .then_with(|| left.option_index.cmp(&right.option_index))
        });
    }

    fn digest(&self) -> Result<Digest, PlanningPriorError> {
        let bytes = serde_json::to_vec(&(
            &self.schema,
            self.model_sha256,
            self.feature_schema_sha256,
            self.action_schema_sha256,
            self.objective_sha256,
            self.option_catalog_sha256,
            self.uncertainty_penalty,
            &self.entries,
            self.route_authority,
            self.promotion_authority,
        ))
        .map_err(|error| PlanningPriorError::new(error.to_string()))?;
        let mut hasher = Sha256::new();
        hasher.update(b"dusklight.q-beam-priors/v1\0");
        hasher.update(bytes);
        Ok(Digest(hasher.finalize().into()))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlanningPriorError(String);

impl PlanningPriorError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for PlanningPriorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for PlanningPriorError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(byte: char) -> String {
        std::iter::repeat_n(byte, 64).collect()
    }

    #[test]
    fn supported_q_values_only_order_children_and_penalize_uncertainty() {
        let parent = id('a');
        let table = QBeamPriorTable::build(
            Digest([1; 32]),
            Digest([2; 32]),
            Digest([3; 32]),
            Digest([4; 32]),
            Digest([5; 32]),
            2.0,
            vec![
                QBeamPrior {
                    parent_candidate_id: parent.clone(),
                    option_index: 0,
                    q_value: 10.0,
                    ensemble_standard_deviation: 5.0,
                    observed_support: 4,
                },
                QBeamPrior {
                    parent_candidate_id: parent.clone(),
                    option_index: 2,
                    q_value: 4.0,
                    ensemble_standard_deviation: 0.5,
                    observed_support: 4,
                },
            ],
        )
        .unwrap();
        table.validate_for_catalog(Digest([5; 32]), 3).unwrap();
        assert_eq!(table.ranked_option_indices(&parent, 3), vec![2, 0, 1]);
        assert!(!table.route_authority);
        assert!(!table.promotion_authority);
    }

    #[test]
    fn detached_catalog_and_unsupported_or_duplicate_entries_fail_closed() {
        let parent = id('b');
        let mut table = QBeamPriorTable::build(
            Digest([1; 32]),
            Digest([2; 32]),
            Digest([3; 32]),
            Digest([4; 32]),
            Digest([5; 32]),
            1.0,
            vec![QBeamPrior {
                parent_candidate_id: parent,
                option_index: 1,
                q_value: 2.0,
                ensemble_standard_deviation: 0.0,
                observed_support: 1,
            }],
        )
        .unwrap();
        assert!(table.validate_for_catalog(Digest([9; 32]), 2).is_err());
        table.entries[0].observed_support = 0;
        assert!(table.validate_for_catalog(Digest([5; 32]), 2).is_err());
    }
}
