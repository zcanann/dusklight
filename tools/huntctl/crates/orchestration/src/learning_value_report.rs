//! Complete-matrix Gate 4 report over independently replayed cell evidence.

use crate::learning_value_comparison::{LearningValueComparisonPlan, LearningValueTreatmentKind};
use crate::learning_value_evidence::{LearningValueCellEvidence, LearningValuePerformanceMetrics};
use dusklight_automation_contracts::artifact::Digest;
use dusklight_harness_contracts::objective_suite::ArtifactReference;
use dusklight_learning::native_goal_reachability::{
    NativeGoalReachabilityNegativeControl, NativeGoalReachabilityNegativeControlReport,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Component, Path};

pub const LEARNING_VALUE_COMPARISON_REPORT_SCHEMA_V1: &str =
    "dusklight-learning-value-comparison-report/v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LearningValueComparisonReport {
    pub schema: String,
    pub content_sha256: Digest,
    pub plan: ArtifactReference,
    pub cells: Vec<ArtifactReference>,
    pub negative_control_report: ArtifactReference,
    pub summaries: Vec<LearningValueTreatmentSummary>,
    pub baseline_comparisons: Vec<LearningValueBaselineComparison>,
    pub complete_matrix: bool,
    pub equal_declared_tick_budgets: bool,
    pub negative_control_loss_verified: bool,
    pub learning_advantage_demonstrated: bool,
    pub promotion_authority: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LearningValueTreatmentSummary {
    pub treatment: LearningValueTreatmentKind,
    pub cells: u64,
    pub charged_simulated_ticks: u64,
    pub evaluated_episodes: u64,
    pub successful_episodes: u64,
    pub successful_episode_rate_millionths: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub best_first_hit_tick: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub median_first_success_charged_simulated_ticks: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LearningValueBaselineComparison {
    pub baseline: LearningValueTreatmentKind,
    pub learned: LearningValueTreatmentKind,
    pub success_rate_gain_millionths: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_hit_improvement_ticks: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample_efficiency_gain_millionths: Option<u32>,
    pub improves_success_rate: bool,
    pub improves_best_first_hit: bool,
    pub improves_sample_efficiency: bool,
    pub learned_wins: bool,
}

impl LearningValueComparisonReport {
    pub fn seal(
        plan: ArtifactReference,
        mut cells: Vec<ArtifactReference>,
        negative_control_report: ArtifactReference,
        repository_root: &Path,
    ) -> Result<Self, LearningValueReportError> {
        cells.sort_by(|left, right| {
            (left.path.as_str(), left.sha256).cmp(&(right.path.as_str(), right.sha256))
        });
        let mut report = build_report(plan, cells, negative_control_report, repository_root)?;
        report.content_sha256 = report.identity()?;
        report.validate_files(repository_root)?;
        Ok(report)
    }

    pub fn validate_files(&self, repository_root: &Path) -> Result<(), LearningValueReportError> {
        if self.schema != LEARNING_VALUE_COMPARISON_REPORT_SCHEMA_V1
            || self.content_sha256 == Digest::ZERO
            || self.promotion_authority
            || self.content_sha256 != self.identity()?
        {
            return Err(report_message(
                "learning-value comparison report or seal is invalid",
            ));
        }
        let mut expected = build_report(
            self.plan.clone(),
            self.cells.clone(),
            self.negative_control_report.clone(),
            repository_root,
        )?;
        expected.content_sha256 = self.content_sha256;
        if &expected != self {
            return Err(report_message(
                "learning-value comparison report differs from its independently replayed matrix",
            ));
        }
        Ok(())
    }

    pub fn to_pretty_json(&self) -> Result<Vec<u8>, LearningValueReportError> {
        let mut bytes = serde_json::to_vec_pretty(self).map_err(report_error)?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    fn identity(&self) -> Result<Digest, LearningValueReportError> {
        let mut canonical = self.clone();
        canonical.content_sha256 = Digest::ZERO;
        canonical_digest(
            b"dusklight.learning-value-comparison-report/v1\0",
            &canonical,
        )
    }
}

fn build_report(
    plan_reference: ArtifactReference,
    cells: Vec<ArtifactReference>,
    negative_control_reference: ArtifactReference,
    root: &Path,
) -> Result<LearningValueComparisonReport, LearningValueReportError> {
    validate_reference_order(&cells)?;
    let plan: LearningValueComparisonPlan = read_json(root, &plan_reference)?;
    plan.validate_files(root).map_err(report_error)?;
    let mut expected_cells = BTreeSet::new();
    for checkpoint in &plan.held_out_checkpoints {
        for seed in &plan.deterministic_seeds {
            for treatment in &plan.treatments {
                expected_cells.insert((checkpoint.id.clone(), *seed, treatment.kind()));
            }
        }
    }
    if cells.len() != expected_cells.len() {
        return Err(report_message(
            "learning-value report does not contain the exact planned cell count",
        ));
    }
    let mut decoded = Vec::with_capacity(cells.len());
    let mut actual_cells = BTreeSet::new();
    for reference in &cells {
        let cell: LearningValueCellEvidence = read_json(root, reference)?;
        cell.validate_files(&plan, root).map_err(report_error)?;
        if !actual_cells.insert((
            cell.checkpoint_id.clone(),
            cell.deterministic_seed,
            cell.treatment,
        )) {
            return Err(report_message(
                "learning-value report contains a duplicate experimental cell",
            ));
        }
        decoded.push(cell);
    }
    if actual_cells != expected_cells {
        return Err(report_message(
            "learning-value report cell matrix differs from the sealed plan cross-product",
        ));
    }
    let summaries = summarize(&decoded)?;
    let negative_control_loss_verified =
        validate_negative_control_loss(root, &negative_control_reference)?;
    if plan.win_rule.require_negative_control_loss && !negative_control_loss_verified {
        return Err(report_message(
            "learning-value report requires negative controls to remove predictive advantage",
        ));
    }
    let baseline_comparisons = compare_baselines(&plan, &summaries)?;
    let learning_advantage_demonstrated = baseline_comparisons
        .iter()
        .all(|comparison| comparison.learned_wins)
        && negative_control_loss_verified;
    Ok(LearningValueComparisonReport {
        schema: LEARNING_VALUE_COMPARISON_REPORT_SCHEMA_V1.into(),
        content_sha256: Digest::ZERO,
        plan: plan_reference,
        cells,
        negative_control_report: negative_control_reference,
        summaries,
        baseline_comparisons,
        complete_matrix: true,
        equal_declared_tick_budgets: true,
        negative_control_loss_verified,
        learning_advantage_demonstrated,
        promotion_authority: false,
    })
}

fn summarize(
    cells: &[LearningValueCellEvidence],
) -> Result<Vec<LearningValueTreatmentSummary>, LearningValueReportError> {
    let mut grouped =
        BTreeMap::<LearningValueTreatmentKind, Vec<&LearningValuePerformanceMetrics>>::new();
    for cell in cells {
        grouped
            .entry(cell.treatment)
            .or_default()
            .push(&cell.metrics);
    }
    grouped
        .into_iter()
        .map(|(treatment, metrics)| summarize_treatment(treatment, &metrics))
        .collect()
}

fn summarize_treatment(
    treatment: LearningValueTreatmentKind,
    metrics: &[&LearningValuePerformanceMetrics],
) -> Result<LearningValueTreatmentSummary, LearningValueReportError> {
    let charged = checked_sum(
        metrics.iter().map(|metric| metric.charged_simulated_ticks),
        "ticks",
    )?;
    let episodes = checked_sum(
        metrics.iter().map(|metric| metric.evaluated_episodes),
        "episodes",
    )?;
    let successes = checked_sum(
        metrics.iter().map(|metric| metric.successful_episodes),
        "successes",
    )?;
    let rate = u32::try_from(successes.saturating_mul(1_000_000) / episodes)
        .map_err(|_| report_message("learning-value summary success rate overflowed"))?;
    let best = metrics
        .iter()
        .filter_map(|metric| metric.best_first_hit_tick)
        .min();
    let mut first_success = metrics
        .iter()
        .filter_map(|metric| metric.first_success_charged_simulated_ticks)
        .collect::<Vec<_>>();
    first_success.sort_unstable();
    let median = (!first_success.is_empty()).then(|| first_success[first_success.len() / 2]);
    Ok(LearningValueTreatmentSummary {
        treatment,
        cells: metrics.len() as u64,
        charged_simulated_ticks: charged,
        evaluated_episodes: episodes,
        successful_episodes: successes,
        successful_episode_rate_millionths: rate,
        best_first_hit_tick: best,
        median_first_success_charged_simulated_ticks: median,
    })
}

fn compare_baselines(
    plan: &LearningValueComparisonPlan,
    summaries: &[LearningValueTreatmentSummary],
) -> Result<Vec<LearningValueBaselineComparison>, LearningValueReportError> {
    let learned_kind = LearningValueTreatmentKind::LearnedThenResidualRefinement;
    let learned = summary(summaries, learned_kind)?;
    [
        LearningValueTreatmentKind::IndependentRandomResidual,
        LearningValueTreatmentKind::CemResidual,
    ]
    .into_iter()
    .map(|baseline_kind| {
        let baseline = summary(summaries, baseline_kind)?;
        let success_gain = i64::from(learned.successful_episode_rate_millionths)
            - i64::from(baseline.successful_episode_rate_millionths);
        let first_hit_improvement =
            lower_improvement(baseline.best_first_hit_tick, learned.best_first_hit_tick);
        let sample_efficiency_gain = relative_lower_improvement_millionths(
            baseline.median_first_success_charged_simulated_ticks,
            learned.median_first_success_charged_simulated_ticks,
        );
        let improves_success_rate =
            success_gain >= i64::from(plan.win_rule.minimum_success_rate_gain_millionths);
        let improves_best_first_hit = first_hit_improvement
            .is_some_and(|gain| gain >= plan.win_rule.minimum_first_hit_improvement_ticks);
        let improves_sample_efficiency = sample_efficiency_gain
            .is_some_and(|gain| gain >= plan.win_rule.minimum_sample_efficiency_gain_millionths);
        Ok(LearningValueBaselineComparison {
            baseline: baseline_kind,
            learned: learned_kind,
            success_rate_gain_millionths: success_gain,
            first_hit_improvement_ticks: first_hit_improvement,
            sample_efficiency_gain_millionths: sample_efficiency_gain,
            improves_success_rate,
            improves_best_first_hit,
            improves_sample_efficiency,
            learned_wins: improves_success_rate
                || improves_best_first_hit
                || improves_sample_efficiency,
        })
    })
    .collect()
}

fn validate_negative_control_loss(
    root: &Path,
    reference: &ArtifactReference,
) -> Result<bool, LearningValueReportError> {
    let report: NativeGoalReachabilityNegativeControlReport = read_json(root, reference)?;
    report.validate().map_err(report_error)?;
    let worsens = |control: NativeGoalReachabilityNegativeControl| {
        report
            .controls
            .iter()
            .find(|row| row.control == Some(control))
            .is_some_and(|row| {
                row.test.reachability_brier > report.baseline.test.reachability_brier
                    && row.test.successful_time_mae_ticks
                        > report.baseline.test.successful_time_mae_ticks
                    && row.test.discounted_return_rmse > report.baseline.test.discounted_return_rmse
            })
    };
    Ok(
        worsens(NativeGoalReachabilityNegativeControl::ShuffledOutcomes)
            && worsens(NativeGoalReachabilityNegativeControl::ActionOnlyInput),
    )
}

fn summary(
    summaries: &[LearningValueTreatmentSummary],
    kind: LearningValueTreatmentKind,
) -> Result<&LearningValueTreatmentSummary, LearningValueReportError> {
    summaries
        .iter()
        .find(|summary| summary.treatment == kind)
        .ok_or_else(|| report_message("learning-value summary omits a planned treatment"))
}

fn lower_improvement(baseline: Option<u64>, learned: Option<u64>) -> Option<u64> {
    match (baseline, learned) {
        (Some(baseline), Some(learned)) => baseline.checked_sub(learned),
        (None, Some(_)) => Some(u64::MAX),
        _ => None,
    }
}

fn relative_lower_improvement_millionths(
    baseline: Option<u64>,
    learned: Option<u64>,
) -> Option<u32> {
    match (baseline, learned) {
        (Some(baseline), Some(learned)) if baseline > 0 && learned < baseline => {
            u32::try_from(baseline.saturating_sub(learned).saturating_mul(1_000_000) / baseline)
                .ok()
        }
        (None, Some(_)) => Some(1_000_000),
        _ => None,
    }
}

fn checked_sum(
    mut values: impl Iterator<Item = u64>,
    label: &str,
) -> Result<u64, LearningValueReportError> {
    values
        .try_fold(0_u64, |total, value| total.checked_add(value))
        .ok_or_else(|| report_message(format!("learning-value summary {label} overflowed")))
}

fn validate_reference_order(
    references: &[ArtifactReference],
) -> Result<(), LearningValueReportError> {
    if references.is_empty()
        || references.windows(2).any(|pair| {
            (pair[0].path.as_str(), pair[0].sha256) >= (pair[1].path.as_str(), pair[1].sha256)
        })
    {
        return Err(report_message(
            "learning-value cell references must be nonempty, unique, and sorted",
        ));
    }
    Ok(())
}

fn read_json<T: for<'de> Deserialize<'de>>(
    root: &Path,
    reference: &ArtifactReference,
) -> Result<T, LearningValueReportError> {
    let bytes = read_reference(root, reference)?;
    serde_json::from_slice(&bytes).map_err(report_error)
}

fn read_reference(
    root: &Path,
    reference: &ArtifactReference,
) -> Result<Vec<u8>, LearningValueReportError> {
    let root = root.canonicalize().map_err(report_error)?;
    let relative = Path::new(&reference.path);
    if reference.sha256 == Digest::ZERO
        || relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(report_message(
            "learning-value report artifact reference is invalid",
        ));
    }
    let path = root.join(relative).canonicalize().map_err(report_error)?;
    if !path.starts_with(&root) || !path.is_file() {
        return Err(report_message(
            "learning-value report artifact must resolve inside the repository",
        ));
    }
    let bytes = fs::read(path).map_err(report_error)?;
    if sha256(&bytes) != reference.sha256 {
        return Err(report_message(
            "learning-value report artifact content differs from its reference",
        ));
    }
    Ok(bytes)
}

fn canonical_digest(
    domain: &[u8],
    value: &impl Serialize,
) -> Result<Digest, LearningValueReportError> {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(serde_json::to_vec(value).map_err(report_error)?);
    Ok(Digest(hasher.finalize().into()))
}

fn sha256(bytes: &[u8]) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Digest(hasher.finalize().into())
}

#[derive(Debug)]
pub struct LearningValueReportError(String);

impl fmt::Display for LearningValueReportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl Error for LearningValueReportError {}

fn report_message(message: impl Into<String>) -> LearningValueReportError {
    LearningValueReportError(message.into())
}

fn report_error(error: impl Error) -> LearningValueReportError {
    report_message(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregate_metrics_are_episode_weighted_and_sample_efficiency_is_median() {
        let a = metrics(100, 4, 1, Some(12), Some(90));
        let b = metrics(60, 2, 2, Some(8), Some(20));
        let summary =
            summarize_treatment(LearningValueTreatmentKind::CemResidual, &[&a, &b]).unwrap();
        assert_eq!(summary.successful_episode_rate_millionths, 500_000);
        assert_eq!(summary.best_first_hit_tick, Some(8));
        assert_eq!(
            summary.median_first_success_charged_simulated_ticks,
            Some(90)
        );
    }

    #[test]
    fn lower_is_better_comparisons_handle_a_baseline_without_success() {
        assert_eq!(lower_improvement(None, Some(10)), Some(u64::MAX));
        assert_eq!(
            relative_lower_improvement_millionths(None, Some(10)),
            Some(1_000_000)
        );
        assert_eq!(lower_improvement(Some(8), Some(10)), None);
    }

    fn metrics(
        ticks: u64,
        episodes: u64,
        successes: u64,
        best: Option<u64>,
        first: Option<u64>,
    ) -> LearningValuePerformanceMetrics {
        LearningValuePerformanceMetrics {
            charged_simulated_ticks: ticks,
            evaluated_episodes: episodes,
            successful_episodes: successes,
            successful_episode_rate_millionths: u32::try_from(successes * 1_000_000 / episodes)
                .unwrap(),
            best_first_hit_tick: best,
            first_success_charged_simulated_ticks: first,
        }
    }
}
