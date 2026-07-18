//! Compatibility-safe multi-task and transfer-learning experiment plans.

use crate::artifact::Digest;
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

pub const TRANSFER_EXPERIMENT_PLAN_SCHEMA_V1: &str = "dusklight-transfer-experiment-plan/v1";
const MAX_DOMAINS: usize = 256;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TransferDomain {
    pub map_id: String,
    pub tactic_family: String,
    pub goal_family: String,
    pub feature_schema_sha256: Digest,
    pub action_schema_sha256: Digest,
    pub fidelity_profile_sha256: Digest,
    pub code_build_sha256: Digest,
    pub data_build_sha256: Digest,
    pub corpus_manifest_sha256: Digest,
    pub available_decisions: usize,
    pub domain_sha256: Digest,
}

impl TransferDomain {
    #[allow(clippy::too_many_arguments)]
    pub fn seal(
        map_id: impl Into<String>,
        tactic_family: impl Into<String>,
        goal_family: impl Into<String>,
        feature_schema_sha256: Digest,
        action_schema_sha256: Digest,
        fidelity_profile_sha256: Digest,
        code_build_sha256: Digest,
        data_build_sha256: Digest,
        corpus_manifest_sha256: Digest,
        available_decisions: usize,
    ) -> Result<Self, TransferLearningError> {
        let mut domain = Self {
            map_id: map_id.into(),
            tactic_family: tactic_family.into(),
            goal_family: goal_family.into(),
            feature_schema_sha256,
            action_schema_sha256,
            fidelity_profile_sha256,
            code_build_sha256,
            data_build_sha256,
            corpus_manifest_sha256,
            available_decisions,
            domain_sha256: Digest::ZERO,
        };
        domain.domain_sha256 = domain.digest()?;
        domain.validate()?;
        Ok(domain)
    }

    fn validate(&self) -> Result<(), TransferLearningError> {
        if !valid_label(&self.map_id)
            || !valid_label(&self.tactic_family)
            || !valid_label(&self.goal_family)
            || [
                self.feature_schema_sha256,
                self.action_schema_sha256,
                self.fidelity_profile_sha256,
                self.code_build_sha256,
                self.data_build_sha256,
                self.corpus_manifest_sha256,
            ]
            .contains(&Digest::ZERO)
            || self.available_decisions == 0
            || self.domain_sha256 != self.digest()?
        {
            return Err(TransferLearningError::new(
                "transfer domain identity is invalid",
            ));
        }
        Ok(())
    }

    fn digest(&self) -> Result<Digest, TransferLearningError> {
        canonical_digest(
            b"dusklight.transfer-domain/v1\0",
            &(
                &self.map_id,
                &self.tactic_family,
                &self.goal_family,
                self.feature_schema_sha256,
                self.action_schema_sha256,
                self.fidelity_profile_sha256,
                self.code_build_sha256,
                self.data_build_sha256,
                self.corpus_manifest_sha256,
                self.available_decisions,
            ),
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct TransferExperimentConfig {
    pub per_domain_decision_budget: usize,
    pub minimum_source_domains: usize,
}

impl Default for TransferExperimentConfig {
    fn default() -> Self {
        Self {
            per_domain_decision_budget: 10_000,
            minimum_source_domains: 2,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TransferExperimentPlan {
    pub schema: &'static str,
    pub target_domain_sha256: Digest,
    pub source_domain_sha256: Vec<Digest>,
    pub feature_schema_sha256: Digest,
    pub action_schema_sha256: Digest,
    pub fidelity_profile_sha256: Digest,
    pub code_build_sha256: Digest,
    pub data_build_sha256: Digest,
    pub config: TransferExperimentConfig,
    pub domain_count: usize,
    pub map_count: usize,
    pub tactic_family_count: usize,
    pub goal_family_count: usize,
    pub equal_decision_budget_per_domain: bool,
    pub corpora_physically_merged: bool,
    pub shared_encoder: bool,
    pub task_specific_heads: bool,
    pub transfer_is_candidate_only: bool,
    pub promotion_authority: bool,
    pub plan_sha256: Digest,
}

impl TransferExperimentPlan {
    pub fn build(
        target: &TransferDomain,
        sources: &[TransferDomain],
        config: TransferExperimentConfig,
    ) -> Result<Self, TransferLearningError> {
        target.validate()?;
        if sources.len() < config.minimum_source_domains
            || sources.len() >= MAX_DOMAINS
            || config.minimum_source_domains == 0
            || config.per_domain_decision_budget == 0
            || target.available_decisions < config.per_domain_decision_budget
        {
            return Err(TransferLearningError::new(
                "transfer experiment size or sample budget is invalid",
            ));
        }
        let mut domain_ids = BTreeSet::from([target.domain_sha256]);
        for source in sources {
            source.validate()?;
            if source.feature_schema_sha256 != target.feature_schema_sha256
                || source.action_schema_sha256 != target.action_schema_sha256
                || source.fidelity_profile_sha256 != target.fidelity_profile_sha256
                || source.code_build_sha256 != target.code_build_sha256
                || source.data_build_sha256 != target.data_build_sha256
                || source.available_decisions < config.per_domain_decision_budget
                || !domain_ids.insert(source.domain_sha256)
            {
                return Err(TransferLearningError::new(
                    "transfer source is incompatible, under-budget, or duplicated",
                ));
            }
        }
        let domains = std::iter::once(target).chain(sources).collect::<Vec<_>>();
        let map_count = domains
            .iter()
            .map(|domain| domain.map_id.as_str())
            .collect::<BTreeSet<_>>()
            .len();
        let tactic_family_count = domains
            .iter()
            .map(|domain| domain.tactic_family.as_str())
            .collect::<BTreeSet<_>>()
            .len();
        let goal_family_count = domains
            .iter()
            .map(|domain| domain.goal_family.as_str())
            .collect::<BTreeSet<_>>()
            .len();
        let mut source_domain_sha256 = sources
            .iter()
            .map(|source| source.domain_sha256)
            .collect::<Vec<_>>();
        source_domain_sha256.sort();
        let mut plan = Self {
            schema: TRANSFER_EXPERIMENT_PLAN_SCHEMA_V1,
            target_domain_sha256: target.domain_sha256,
            source_domain_sha256,
            feature_schema_sha256: target.feature_schema_sha256,
            action_schema_sha256: target.action_schema_sha256,
            fidelity_profile_sha256: target.fidelity_profile_sha256,
            code_build_sha256: target.code_build_sha256,
            data_build_sha256: target.data_build_sha256,
            config,
            domain_count: domains.len(),
            map_count,
            tactic_family_count,
            goal_family_count,
            equal_decision_budget_per_domain: true,
            corpora_physically_merged: false,
            shared_encoder: true,
            task_specific_heads: true,
            transfer_is_candidate_only: true,
            promotion_authority: false,
            plan_sha256: Digest::ZERO,
        };
        plan.plan_sha256 = plan.digest()?;
        Ok(plan)
    }

    fn digest(&self) -> Result<Digest, TransferLearningError> {
        canonical_digest(
            b"dusklight.transfer-experiment-plan/v1\0",
            &(
                (
                    self.schema,
                    self.target_domain_sha256,
                    &self.source_domain_sha256,
                    self.feature_schema_sha256,
                    self.action_schema_sha256,
                    self.fidelity_profile_sha256,
                    self.code_build_sha256,
                    self.data_build_sha256,
                    self.config,
                    self.domain_count,
                ),
                (
                    self.map_count,
                    self.tactic_family_count,
                    self.goal_family_count,
                    self.equal_decision_budget_per_domain,
                    self.corpora_physically_merged,
                    self.shared_encoder,
                    self.task_specific_heads,
                    self.transfer_is_candidate_only,
                    self.promotion_authority,
                ),
            ),
        )
    }
}

fn valid_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-/".contains(&byte))
}

fn canonical_digest<T: Serialize>(
    domain: &[u8],
    value: &T,
) -> Result<Digest, TransferLearningError> {
    let bytes =
        serde_json::to_vec(value).map_err(|error| TransferLearningError::new(error.to_string()))?;
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(bytes);
    Ok(Digest(hasher.finalize().into()))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransferLearningError(String);

impl TransferLearningError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for TransferLearningError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for TransferLearningError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn domain(map: &str, tactic: &str, goal: &str, action: u8, fidelity: u8) -> TransferDomain {
        TransferDomain::seal(
            map,
            tactic,
            goal,
            Digest([1; 32]),
            Digest([action; 32]),
            Digest([fidelity; 32]),
            Digest([4; 32]),
            Digest([5; 32]),
            Digest([map.as_bytes()[0]; 32]),
            20,
        )
        .unwrap()
    }

    fn config() -> TransferExperimentConfig {
        TransferExperimentConfig {
            per_domain_decision_budget: 10,
            minimum_source_domains: 2,
        }
    }

    #[test]
    fn compatible_maps_tactics_and_goals_share_only_encoder_and_equal_budget() {
        let target = domain("map-a", "movement", "reach-room", 2, 3);
        let sources = vec![
            domain("map-b", "roll", "reach-room", 2, 3),
            domain("map-c", "movement", "collect-item", 2, 3),
        ];
        let plan = TransferExperimentPlan::build(&target, &sources, config()).unwrap();
        assert_eq!(plan.map_count, 3);
        assert_eq!(plan.tactic_family_count, 2);
        assert_eq!(plan.goal_family_count, 2);
        assert!(plan.equal_decision_budget_per_domain);
        assert!(!plan.corpora_physically_merged);
        assert!(plan.shared_encoder);
        assert!(plan.task_specific_heads);
        assert!(!plan.promotion_authority);
        assert_ne!(plan.plan_sha256, Digest::ZERO);
    }

    #[test]
    fn action_or_fidelity_mismatch_is_never_merged() {
        let target = domain("map-a", "movement", "reach-room", 2, 3);
        let action_mismatch = vec![
            domain("map-b", "movement", "reach-room", 9, 3),
            domain("map-c", "movement", "reach-room", 2, 3),
        ];
        assert!(TransferExperimentPlan::build(&target, &action_mismatch, config()).is_err());
        let fidelity_mismatch = vec![
            domain("map-b", "movement", "reach-room", 2, 8),
            domain("map-c", "movement", "reach-room", 2, 3),
        ];
        assert!(TransferExperimentPlan::build(&target, &fidelity_mismatch, config()).is_err());
    }
}
