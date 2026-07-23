//! Sealed, equal-budget protocol for the Gate 4 learning-value comparison.
//!
//! The plan owns the experimental cells. Treatments cannot quietly choose
//! different seeds, checkpoints, tick caps, terminal predicates, repetitions,
//! or metrics because those dimensions occur exactly once in this contract.

use crate::optimization_request::ResidualOptimizerConfig;
use dusklight_automation_contracts::artifact::Digest;
use dusklight_harness_contracts::objective_suite::ArtifactReference;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Component, Path};

pub const LEARNING_VALUE_COMPARISON_PLAN_SCHEMA_V1: &str =
    "dusklight-learning-value-comparison-plan/v1";

const MIN_SEEDS: usize = 3;
const MIN_HELD_OUT_CHECKPOINTS: usize = 2;
const MAX_SIMULATED_TICKS_PER_CELL: u64 = 1_000_000_000_000;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LearningValueComparisonPlan {
    pub schema: String,
    pub content_sha256: Digest,
    pub id: String,
    pub terminal_program_sha256: Digest,
    pub terminal_definition_sha256: Digest,
    pub simulated_tick_budget_per_cell: u64,
    pub repetitions_per_cell: u16,
    pub deterministic_seeds: Vec<u64>,
    pub held_out_checkpoints: Vec<LearningValueCheckpoint>,
    pub treatments: Vec<LearningValueTreatment>,
    pub win_rule: LearningValueWinRule,
    pub promotion_authority: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LearningValueCheckpoint {
    pub id: String,
    pub source: ArtifactReference,
    pub source_boundary_index: u64,
    pub native_source_boundary_fingerprint: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LearningValuePhaseBudget {
    pub discovery_simulated_ticks: u64,
    pub refinement_simulated_ticks: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LearningValueTreatment {
    IndependentRandomResidual {
        budget: LearningValuePhaseBudget,
        optimizer: ResidualOptimizerConfig,
    },
    CemResidual {
        budget: LearningValuePhaseBudget,
        optimizer: ResidualOptimizerConfig,
    },
    DemonstrationAssistedStateReactive {
        budget: LearningValuePhaseBudget,
        generation_limit: u16,
        rollouts_per_generation: u16,
    },
    FromScratchStateReactive {
        budget: LearningValuePhaseBudget,
        generation_limit: u16,
        rollouts_per_generation: u16,
    },
    LearnedThenResidualRefinement {
        budget: LearningValuePhaseBudget,
        generation_limit: u16,
        rollouts_per_generation: u16,
        refinement_optimizer: ResidualOptimizerConfig,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LearningValueTreatmentKind {
    IndependentRandomResidual,
    CemResidual,
    DemonstrationAssistedStateReactive,
    FromScratchStateReactive,
    LearnedThenResidualRefinement,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LearningValueMetric {
    SuccessfulEpisodeRate,
    BestFirstHitTick,
    FirstSuccessSimulatedTicks,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LearningValueWinRule {
    pub metrics: Vec<LearningValueMetric>,
    pub minimum_success_rate_gain_millionths: u32,
    pub minimum_first_hit_improvement_ticks: u64,
    pub minimum_sample_efficiency_gain_millionths: u32,
    pub require_negative_control_loss: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LearningValueComparisonPlanValidationReport {
    pub schema: &'static str,
    pub plan_sha256: Digest,
    pub seeds: u64,
    pub held_out_checkpoints: u64,
    pub treatments: u64,
    pub experimental_cells: u64,
    pub simulated_tick_budget_per_cell: u64,
    pub total_simulated_tick_budget: u64,
    pub identical_initial_state_distribution: bool,
    pub equal_simulated_tick_budgets: bool,
    pub post_discovery_refinement_matches_cem: bool,
    pub promotion_authority: bool,
}

impl LearningValueComparisonPlan {
    /// Re-seal a human-authored draft after independently checking every
    /// checkpoint artifact. A draft may carry either a zero or stale seal.
    pub fn seal(mut self, repository_root: &Path) -> Result<Self, LearningValueComparisonError> {
        self.content_sha256 = Digest::ZERO;
        self.validate_shape(false)?;
        self.validate_checkpoint_files(repository_root)?;
        self.content_sha256 = self.identity()?;
        self.validate_files(repository_root)?;
        Ok(self)
    }

    pub fn validate_files(
        &self,
        repository_root: &Path,
    ) -> Result<LearningValueComparisonPlanValidationReport, LearningValueComparisonError> {
        self.validate_shape(true)?;
        self.validate_checkpoint_files(repository_root)?;
        let cells = (self.deterministic_seeds.len() as u64)
            .checked_mul(self.held_out_checkpoints.len() as u64)
            .and_then(|value| value.checked_mul(self.treatments.len() as u64))
            .ok_or_else(|| {
                comparison_message("learning-value experimental cell count overflowed")
            })?;
        let total = cells
            .checked_mul(self.simulated_tick_budget_per_cell)
            .ok_or_else(|| comparison_message("learning-value total tick budget overflowed"))?;
        Ok(LearningValueComparisonPlanValidationReport {
            schema: LEARNING_VALUE_COMPARISON_PLAN_SCHEMA_V1,
            plan_sha256: self.content_sha256,
            seeds: self.deterministic_seeds.len() as u64,
            held_out_checkpoints: self.held_out_checkpoints.len() as u64,
            treatments: self.treatments.len() as u64,
            experimental_cells: cells,
            simulated_tick_budget_per_cell: self.simulated_tick_budget_per_cell,
            total_simulated_tick_budget: total,
            identical_initial_state_distribution: true,
            equal_simulated_tick_budgets: true,
            post_discovery_refinement_matches_cem: true,
            promotion_authority: false,
        })
    }

    pub fn to_pretty_json(&self) -> Result<Vec<u8>, LearningValueComparisonError> {
        let mut bytes = serde_json::to_vec_pretty(self).map_err(comparison_error)?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    fn validate_shape(&self, require_seal: bool) -> Result<(), LearningValueComparisonError> {
        if self.schema != LEARNING_VALUE_COMPARISON_PLAN_SCHEMA_V1
            || !valid_id(&self.id)
            || self.terminal_program_sha256 == Digest::ZERO
            || self.terminal_definition_sha256 == Digest::ZERO
            || self.simulated_tick_budget_per_cell == 0
            || self.simulated_tick_budget_per_cell > MAX_SIMULATED_TICKS_PER_CELL
            || self.repetitions_per_cell == 0
            || self.deterministic_seeds.len() < MIN_SEEDS
            || self.held_out_checkpoints.len() < MIN_HELD_OUT_CHECKPOINTS
            || self.promotion_authority
            || (require_seal
                && (self.content_sha256 == Digest::ZERO
                    || self.content_sha256 != self.identity()?))
        {
            return Err(comparison_message(
                "learning-value comparison plan or seal is invalid",
            ));
        }
        if !strictly_sorted_unique(&self.deterministic_seeds) {
            return Err(comparison_message(
                "learning-value deterministic seeds must be strictly sorted and unique",
            ));
        }
        if self
            .held_out_checkpoints
            .windows(2)
            .any(|pair| pair[0].id >= pair[1].id)
        {
            return Err(comparison_message(
                "learning-value held-out checkpoints must be sorted by unique id",
            ));
        }
        for checkpoint in &self.held_out_checkpoints {
            validate_id("held-out checkpoint", &checkpoint.id)?;
            validate_artifact_shape("held-out checkpoint", &checkpoint.source)?;
            if checkpoint.source_boundary_index == 0
                || !lower_hex(&checkpoint.native_source_boundary_fingerprint, 32)
            {
                return Err(comparison_message(
                    "learning-value held-out checkpoint identity is invalid",
                ));
            }
        }
        self.validate_treatments()?;
        self.validate_win_rule()?;
        Ok(())
    }

    fn validate_treatments(&self) -> Result<(), LearningValueComparisonError> {
        let kinds = self
            .treatments
            .iter()
            .map(LearningValueTreatment::kind)
            .collect::<BTreeSet<_>>();
        let expected = [
            LearningValueTreatmentKind::IndependentRandomResidual,
            LearningValueTreatmentKind::CemResidual,
            LearningValueTreatmentKind::DemonstrationAssistedStateReactive,
            LearningValueTreatmentKind::FromScratchStateReactive,
            LearningValueTreatmentKind::LearnedThenResidualRefinement,
        ]
        .into_iter()
        .collect::<BTreeSet<_>>();
        if self.treatments.len() != expected.len() || kinds != expected {
            return Err(comparison_message(
                "learning-value plan must contain each of the five treatments exactly once",
            ));
        }
        let mut cem_optimizer = None;
        let mut learned_refinement = None;
        for treatment in &self.treatments {
            let budget = treatment.budget();
            if budget
                .discovery_simulated_ticks
                .checked_add(budget.refinement_simulated_ticks)
                != Some(self.simulated_tick_budget_per_cell)
            {
                return Err(comparison_message(
                    "every learning-value treatment must consume the shared per-cell tick budget",
                ));
            }
            match treatment {
                LearningValueTreatment::IndependentRandomResidual { optimizer, .. } => {
                    if !matches!(optimizer, ResidualOptimizerConfig::Random { .. })
                        || budget.discovery_simulated_ticks != 0
                        || budget.refinement_simulated_ticks == 0
                    {
                        return Err(comparison_message(
                            "independent random treatment requires a random residual optimizer and residual-only budget",
                        ));
                    }
                }
                LearningValueTreatment::CemResidual { optimizer, .. } => {
                    if !matches!(optimizer, ResidualOptimizerConfig::Cem { .. })
                        || budget.discovery_simulated_ticks != 0
                        || budget.refinement_simulated_ticks == 0
                    {
                        return Err(comparison_message(
                            "CEM treatment requires a CEM residual optimizer and residual-only budget",
                        ));
                    }
                    cem_optimizer = Some(optimizer);
                }
                LearningValueTreatment::DemonstrationAssistedStateReactive {
                    generation_limit,
                    rollouts_per_generation,
                    ..
                }
                | LearningValueTreatment::FromScratchStateReactive {
                    generation_limit,
                    rollouts_per_generation,
                    ..
                } => {
                    if *generation_limit < 3
                        || *rollouts_per_generation == 0
                        || budget.discovery_simulated_ticks == 0
                        || budget.refinement_simulated_ticks != 0
                    {
                        return Err(comparison_message(
                            "state-reactive treatments require at least three generations and a discovery-only budget",
                        ));
                    }
                }
                LearningValueTreatment::LearnedThenResidualRefinement {
                    generation_limit,
                    rollouts_per_generation,
                    refinement_optimizer,
                    ..
                } => {
                    if *generation_limit < 3
                        || *rollouts_per_generation == 0
                        || budget.discovery_simulated_ticks == 0
                        || budget.refinement_simulated_ticks == 0
                        || !matches!(refinement_optimizer, ResidualOptimizerConfig::Cem { .. })
                    {
                        return Err(comparison_message(
                            "learned-plus-refinement treatment requires nonzero learning and CEM refinement phases",
                        ));
                    }
                    learned_refinement = Some(refinement_optimizer);
                }
            }
        }
        if cem_optimizer != learned_refinement {
            return Err(comparison_message(
                "learned post-discovery refinement must use the exact CEM baseline optimizer",
            ));
        }
        Ok(())
    }

    fn validate_win_rule(&self) -> Result<(), LearningValueComparisonError> {
        let metrics = self
            .win_rule
            .metrics
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        let expected = [
            LearningValueMetric::SuccessfulEpisodeRate,
            LearningValueMetric::BestFirstHitTick,
            LearningValueMetric::FirstSuccessSimulatedTicks,
        ]
        .into_iter()
        .collect::<BTreeSet<_>>();
        if self.win_rule.metrics.len() != expected.len()
            || metrics != expected
            || self.win_rule.minimum_success_rate_gain_millionths == 0
            || self.win_rule.minimum_first_hit_improvement_ticks == 0
            || self.win_rule.minimum_sample_efficiency_gain_millionths == 0
            || !self.win_rule.require_negative_control_loss
        {
            return Err(comparison_message(
                "learning-value win rule must predeclare all three nonzero criteria and require negative-control loss",
            ));
        }
        Ok(())
    }

    fn validate_checkpoint_files(
        &self,
        repository_root: &Path,
    ) -> Result<(), LearningValueComparisonError> {
        let root = repository_root.canonicalize().map_err(comparison_error)?;
        for checkpoint in &self.held_out_checkpoints {
            let path = root
                .join(&checkpoint.source.path)
                .canonicalize()
                .map_err(|error| {
                    comparison_message(format!(
                        "cannot resolve held-out checkpoint {}: {error}",
                        checkpoint.source.path
                    ))
                })?;
            if !path.starts_with(&root) || !path.is_file() {
                return Err(comparison_message(
                    "held-out checkpoint must resolve to a file within the repository",
                ));
            }
            if sha256(&fs::read(path).map_err(comparison_error)?) != checkpoint.source.sha256 {
                return Err(comparison_message(
                    "held-out checkpoint content differs from its sealed digest",
                ));
            }
        }
        Ok(())
    }

    fn identity(&self) -> Result<Digest, LearningValueComparisonError> {
        let mut canonical = self.clone();
        canonical.content_sha256 = Digest::ZERO;
        canonical_digest(b"dusklight.learning-value-comparison-plan/v1\0", &canonical)
    }
}

impl LearningValueTreatment {
    pub fn kind(&self) -> LearningValueTreatmentKind {
        match self {
            Self::IndependentRandomResidual { .. } => {
                LearningValueTreatmentKind::IndependentRandomResidual
            }
            Self::CemResidual { .. } => LearningValueTreatmentKind::CemResidual,
            Self::DemonstrationAssistedStateReactive { .. } => {
                LearningValueTreatmentKind::DemonstrationAssistedStateReactive
            }
            Self::FromScratchStateReactive { .. } => {
                LearningValueTreatmentKind::FromScratchStateReactive
            }
            Self::LearnedThenResidualRefinement { .. } => {
                LearningValueTreatmentKind::LearnedThenResidualRefinement
            }
        }
    }

    pub fn budget(&self) -> LearningValuePhaseBudget {
        match self {
            Self::IndependentRandomResidual { budget, .. }
            | Self::CemResidual { budget, .. }
            | Self::DemonstrationAssistedStateReactive { budget, .. }
            | Self::FromScratchStateReactive { budget, .. }
            | Self::LearnedThenResidualRefinement { budget, .. } => *budget,
        }
    }
}

fn strictly_sorted_unique<T: Ord>(values: &[T]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn validate_id(label: &str, value: &str) -> Result<(), LearningValueComparisonError> {
    if valid_id(value) {
        Ok(())
    } else {
        Err(comparison_message(format!("{label} id is invalid")))
    }
}

fn validate_artifact_shape(
    label: &str,
    artifact: &ArtifactReference,
) -> Result<(), LearningValueComparisonError> {
    let path = Path::new(&artifact.path);
    if artifact.sha256 == Digest::ZERO
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(comparison_message(format!(
            "{label} artifact reference is invalid"
        )));
    }
    Ok(())
}

fn lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn canonical_digest(
    domain: &[u8],
    value: &impl Serialize,
) -> Result<Digest, LearningValueComparisonError> {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(serde_json::to_vec(value).map_err(comparison_error)?);
    Ok(Digest(hasher.finalize().into()))
}

fn sha256(bytes: &[u8]) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Digest(hasher.finalize().into())
}

#[derive(Debug)]
pub struct LearningValueComparisonError(String);

impl fmt::Display for LearningValueComparisonError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl Error for LearningValueComparisonError {}

fn comparison_message(message: impl Into<String>) -> LearningValueComparisonError {
    LearningValueComparisonError(message.into())
}

fn comparison_error(error: impl Error) -> LearningValueComparisonError {
    comparison_message(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static FIXTURE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn fixture() -> (std::path::PathBuf, LearningValueComparisonPlan) {
        let root = std::env::temp_dir().join(format!(
            "dusklight-learning-value-plan-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos(),
            FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("checkpoints")).expect("create fixture");
        fs::write(root.join("checkpoints/a.bin"), b"checkpoint-a").expect("write a");
        fs::write(root.join("checkpoints/b.bin"), b"checkpoint-b").expect("write b");
        let cem = ResidualOptimizerConfig::Cem {
            population: 16,
            elites: 4,
            generations: 4,
            smoothing_millionths: 500_000,
        };
        let plan = LearningValueComparisonPlan {
            schema: LEARNING_VALUE_COMPARISON_PLAN_SCHEMA_V1.into(),
            content_sha256: Digest::ZERO,
            id: "gate4-fixture".into(),
            terminal_program_sha256: Digest([7; 32]),
            terminal_definition_sha256: Digest([8; 32]),
            simulated_tick_budget_per_cell: 10_000,
            repetitions_per_cell: 2,
            deterministic_seeds: vec![11, 22, 33],
            held_out_checkpoints: vec![
                LearningValueCheckpoint {
                    id: "a".into(),
                    source: ArtifactReference {
                        path: "checkpoints/a.bin".into(),
                        sha256: sha256(b"checkpoint-a"),
                    },
                    source_boundary_index: 100,
                    native_source_boundary_fingerprint: "1".repeat(32),
                },
                LearningValueCheckpoint {
                    id: "b".into(),
                    source: ArtifactReference {
                        path: "checkpoints/b.bin".into(),
                        sha256: sha256(b"checkpoint-b"),
                    },
                    source_boundary_index: 110,
                    native_source_boundary_fingerprint: "2".repeat(32),
                },
            ],
            treatments: vec![
                LearningValueTreatment::IndependentRandomResidual {
                    budget: LearningValuePhaseBudget {
                        discovery_simulated_ticks: 0,
                        refinement_simulated_ticks: 10_000,
                    },
                    optimizer: ResidualOptimizerConfig::Random { samples: 64 },
                },
                LearningValueTreatment::CemResidual {
                    budget: LearningValuePhaseBudget {
                        discovery_simulated_ticks: 0,
                        refinement_simulated_ticks: 10_000,
                    },
                    optimizer: cem.clone(),
                },
                LearningValueTreatment::DemonstrationAssistedStateReactive {
                    budget: LearningValuePhaseBudget {
                        discovery_simulated_ticks: 10_000,
                        refinement_simulated_ticks: 0,
                    },
                    generation_limit: 3,
                    rollouts_per_generation: 4,
                },
                LearningValueTreatment::FromScratchStateReactive {
                    budget: LearningValuePhaseBudget {
                        discovery_simulated_ticks: 10_000,
                        refinement_simulated_ticks: 0,
                    },
                    generation_limit: 3,
                    rollouts_per_generation: 4,
                },
                LearningValueTreatment::LearnedThenResidualRefinement {
                    budget: LearningValuePhaseBudget {
                        discovery_simulated_ticks: 6_000,
                        refinement_simulated_ticks: 4_000,
                    },
                    generation_limit: 3,
                    rollouts_per_generation: 4,
                    refinement_optimizer: cem,
                },
            ],
            win_rule: LearningValueWinRule {
                metrics: vec![
                    LearningValueMetric::SuccessfulEpisodeRate,
                    LearningValueMetric::BestFirstHitTick,
                    LearningValueMetric::FirstSuccessSimulatedTicks,
                ],
                minimum_success_rate_gain_millionths: 10_000,
                minimum_first_hit_improvement_ticks: 1,
                minimum_sample_efficiency_gain_millionths: 10_000,
                require_negative_control_loss: true,
            },
            promotion_authority: false,
        };
        (root, plan)
    }

    #[test]
    fn seals_equal_budget_five_treatment_matrix() {
        let (root, draft) = fixture();
        let plan = draft.seal(&root).expect("seal plan");
        let report = plan.validate_files(&root).expect("validate plan");
        assert_eq!(report.experimental_cells, 30);
        assert_eq!(report.total_simulated_tick_budget, 300_000);
        assert!(report.identical_initial_state_distribution);
        assert!(report.equal_simulated_tick_budgets);
        assert!(report.post_discovery_refinement_matches_cem);
        fs::remove_dir_all(root).expect("remove fixture");
    }

    #[test]
    fn rejects_unequal_budget_and_refinement_drift() {
        let (root, mut draft) = fixture();
        if let LearningValueTreatment::LearnedThenResidualRefinement {
            budget,
            refinement_optimizer,
            ..
        } = &mut draft.treatments[4]
        {
            budget.refinement_simulated_ticks -= 1;
            *refinement_optimizer = ResidualOptimizerConfig::Cem {
                population: 32,
                elites: 4,
                generations: 4,
                smoothing_millionths: 500_000,
            };
        }
        assert!(draft.seal(&root).is_err());
        fs::remove_dir_all(root).expect("remove fixture");
    }

    #[test]
    fn rejects_missing_treatment_or_checkpoint_mutation() {
        let (root, mut draft) = fixture();
        draft.treatments.pop();
        assert!(draft.seal(&root).is_err());

        let (other_root, draft) = fixture();
        let plan = draft.seal(&other_root).expect("seal plan");
        fs::write(other_root.join("checkpoints/a.bin"), b"mutated").expect("mutate checkpoint");
        let error = plan
            .validate_files(&other_root)
            .expect_err("mutated checkpoint must fail");
        assert!(error.to_string().contains("differs"));
        fs::remove_dir_all(root).expect("remove fixture");
        fs::remove_dir_all(other_root).expect("remove other fixture");
    }
}
