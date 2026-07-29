//! Pooled cross-conformal calibration over complete semantic groups.

use super::*;

pub const GENERALIZED_TACTIC_CROSS_CALIBRATION_SCHEMA_V1: &str =
    "dusklight-generalized-tactic-cross-calibration/v1";

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GeneralizedTacticCrossCalibrationFold {
    pub validation_fold: u8,
    pub test_fold: u8,
    pub training_groups: Vec<String>,
    pub validation_groups: Vec<String>,
    pub test_groups: Vec<String>,
    pub group_overlap_count: usize,
    pub training_samples: usize,
    pub validation_samples: usize,
    pub test_samples: usize,
    pub validation_conformal_multiplier: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GeneralizedTacticAxisCrossCalibration {
    pub axis: GeneralizedTacticCalibrationAxis,
    pub folds: Vec<GeneralizedTacticCrossCalibrationFold>,
    pub pooled_validation: GeneralizedTacticCalibrationMetrics,
    pub pooled_test: GeneralizedTacticCalibrationMetrics,
    pub conformal_multiplier: f64,
    pub calibration_rule: String,
    pub nominal_interval_coverage: f64,
    pub test_coverage_at_least_nominal: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GeneralizedTacticCrossCalibrationReport {
    pub schema: String,
    pub source_transition_sha256: Digest,
    pub source_transitions: usize,
    pub goal_distance_feature: usize,
    pub target_kind: String,
    pub config: GeneralizedTacticCalibrationConfig,
    pub fold_schedule: String,
    pub state_region: GeneralizedTacticAxisCrossCalibration,
    pub action_realization: GeneralizedTacticAxisCrossCalibration,
    pub report_sha256: Digest,
}

impl GeneralizedTacticCrossCalibrationReport {
    pub fn validate(&self) -> Result<(), GeneralizedTacticCalibrationError> {
        self.config.validate()?;
        if self.schema != GENERALIZED_TACTIC_CROSS_CALIBRATION_SCHEMA_V1
            || self.source_transition_sha256 == Digest::ZERO
            || self.source_transitions < 5
            || self.target_kind != "authenticated_terminal_conditional_ticks"
            || self.fold_schedule != "test_each_fold_once_validation_preceding_fold_modulo_k"
            || self.state_region.axis != GeneralizedTacticCalibrationAxis::StateRegion
            || self.action_realization.axis != GeneralizedTacticCalibrationAxis::ActionRealization
            || !cross_axis_is_valid(&self.state_region, self.source_transitions, self.config)
            || !cross_axis_is_valid(
                &self.action_realization,
                self.source_transitions,
                self.config,
            )
            || self.report_sha256 == Digest::ZERO
            || self.report_sha256 != self.digest()?
        {
            return Err(GeneralizedTacticCalibrationError::new(
                "generalized tactic cross-calibration report is invalid or detached",
            ));
        }
        Ok(())
    }

    pub(super) fn digest(&self) -> Result<Digest, GeneralizedTacticCalibrationError> {
        canonical_digest(
            b"dusklight.generalized-tactic-cross-calibration/v1\0",
            &(
                &self.schema,
                self.source_transition_sha256,
                self.source_transitions,
                self.goal_distance_feature,
                &self.target_kind,
                self.config,
                &self.fold_schedule,
                &self.state_region,
                &self.action_realization,
            ),
        )
    }
}

pub fn cross_calibrate_generalized_tactic_value(
    transitions: &[OptionTransitionSample],
    goal_distance_feature: usize,
    config: GeneralizedTacticCalibrationConfig,
) -> Result<GeneralizedTacticCrossCalibrationReport, GeneralizedTacticCalibrationError> {
    config.validate()?;
    if transitions.len() < 5 {
        return Err(GeneralizedTacticCalibrationError::new(
            "generalized tactic cross-calibration requires at least five transitions",
        ));
    }
    for transition in transitions {
        transition
            .validate()
            .map_err(|error| GeneralizedTacticCalibrationError::new(error.to_string()))?;
        if goal_distance_feature >= transition.value_sample.state.len() {
            return Err(GeneralizedTacticCalibrationError::new(
                "goal-distance feature is outside the transition state",
            ));
        }
    }
    let fitted = fit_transition_returns(
        transitions,
        usize::from(config.fitted_q_iterations),
        config.per_tick_discount,
    )?;
    let rows = transitions
        .iter()
        .zip(fitted.values)
        .filter_map(|(transition, target)| {
            target.map(|target| {
                Ok(CalibrationRow {
                    transition,
                    target: f64::from(target),
                    state_region: state_region(transition, config.state_region_width)?,
                    action_realization: transition
                        .value_sample
                        .action
                        .content_sha256()
                        .map_err(|error| GeneralizedTacticCalibrationError::new(error.to_string()))?
                        .to_string(),
                })
            })
        })
        .collect::<Result<Vec<_>, GeneralizedTacticCalibrationError>>()?;
    if rows.len() < 5 {
        return Err(GeneralizedTacticCalibrationError::new(
            "generalized tactic cross-calibration requires at least five terminal-connected transitions",
        ));
    }
    let state_region = cross_calibrate_axis(
        &rows,
        goal_distance_feature,
        config,
        GeneralizedTacticCalibrationAxis::StateRegion,
    )?;
    let action_realization = cross_calibrate_axis(
        &rows,
        goal_distance_feature,
        config,
        GeneralizedTacticCalibrationAxis::ActionRealization,
    )?;
    let mut report = GeneralizedTacticCrossCalibrationReport {
        schema: GENERALIZED_TACTIC_CROSS_CALIBRATION_SCHEMA_V1.into(),
        source_transition_sha256: transition_corpus_digest(transitions)?,
        source_transitions: transitions.len(),
        goal_distance_feature,
        target_kind: "authenticated_terminal_conditional_ticks".into(),
        config,
        fold_schedule: "test_each_fold_once_validation_preceding_fold_modulo_k".into(),
        state_region,
        action_realization,
        report_sha256: Digest::ZERO,
    };
    report.report_sha256 = report.digest()?;
    report.validate()?;
    Ok(report)
}

fn cross_calibrate_axis(
    rows: &[CalibrationRow<'_>],
    goal_distance_feature: usize,
    config: GeneralizedTacticCalibrationConfig,
    axis: GeneralizedTacticCalibrationAxis,
) -> Result<GeneralizedTacticAxisCrossCalibration, GeneralizedTacticCalibrationError> {
    let groups = rows
        .iter()
        .map(|row| row_group(row, axis))
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if groups.len() < usize::from(config.group_folds) {
        return Err(GeneralizedTacticCalibrationError::new(format!(
            "{axis:?} cross-calibration has fewer groups than declared folds"
        )));
    }
    let mut pooled_validation = Vec::with_capacity(rows.len());
    let mut pooled_test = Vec::with_capacity(rows.len());
    let mut folds = Vec::with_capacity(usize::from(config.group_folds));
    for test_fold in 0..config.group_folds {
        let validation_fold = (test_fold + config.group_folds - 1) % config.group_folds;
        let mut training_groups = Vec::new();
        let mut validation_groups = Vec::new();
        let mut test_groups = Vec::new();
        for (index, group) in groups.iter().cloned().enumerate() {
            match (index % usize::from(config.group_folds)) as u8 {
                fold if fold == validation_fold => validation_groups.push(group),
                fold if fold == test_fold => test_groups.push(group),
                _ => training_groups.push(group),
            }
        }
        let training_set = training_groups.iter().collect::<BTreeSet<_>>();
        let validation_set = validation_groups.iter().collect::<BTreeSet<_>>();
        let test_set = test_groups.iter().collect::<BTreeSet<_>>();
        let training = rows
            .iter()
            .filter(|row| training_set.contains(row_group(row, axis)))
            .map(|row| row.transition.clone())
            .collect::<Vec<_>>();
        let validation = rows
            .iter()
            .filter(|row| validation_set.contains(row_group(row, axis)))
            .cloned()
            .collect::<Vec<_>>();
        let test = rows
            .iter()
            .filter(|row| test_set.contains(row_group(row, axis)))
            .cloned()
            .collect::<Vec<_>>();
        if training.len() < 2 || validation.is_empty() || test.is_empty() {
            return Err(GeneralizedTacticCalibrationError::new(format!(
                "{axis:?} cross-calibration produced an empty or undersized partition"
            )));
        }
        let model = GeneralizedTacticValueModel::fit_fitted_q_transitions(
            &training,
            goal_distance_feature,
            usize::from(config.fitted_q_iterations),
            config.per_tick_discount,
        )?;
        let training_targets = rows
            .iter()
            .filter(|row| training_set.contains(row_group(row, axis)))
            .map(|row| row.target)
            .collect::<Vec<_>>();
        let return_scale = standard_deviation(&training_targets).max(MINIMUM_UNCERTAINTY_SCALE);
        let validation_predictions = predict_rows(&model, &validation, return_scale)?;
        let nominal = f64::from(config.interval_coverage_millionths) / 1_000_000.0;
        let validation_multiplier = conformal_multiplier(&validation_predictions, nominal);
        pooled_validation.extend(validation_predictions);
        pooled_test.extend(predict_rows(&model, &test, return_scale)?);
        let overlap_count = group_overlap_count(&training_set, &validation_set, &test_set);
        folds.push(GeneralizedTacticCrossCalibrationFold {
            validation_fold,
            test_fold,
            training_groups,
            validation_groups,
            test_groups,
            group_overlap_count: overlap_count,
            training_samples: training.len(),
            validation_samples: validation.len(),
            test_samples: test.len(),
            validation_conformal_multiplier: validation_multiplier,
        });
    }
    let nominal = f64::from(config.interval_coverage_millionths) / 1_000_000.0;
    let multiplier = folds
        .iter()
        .map(|fold| fold.validation_conformal_multiplier)
        .chain(std::iter::once(conformal_multiplier(
            &pooled_validation,
            nominal,
        )))
        .max_by(f64::total_cmp)
        .expect("cross-calibration has at least one fold");
    let validation_metrics = metrics(&pooled_validation, multiplier);
    let test_metrics = metrics(&pooled_test, multiplier);
    Ok(GeneralizedTacticAxisCrossCalibration {
        axis,
        folds,
        pooled_validation: validation_metrics,
        test_coverage_at_least_nominal: test_metrics.interval_coverage >= nominal,
        pooled_test: test_metrics,
        conformal_multiplier: multiplier,
        calibration_rule: "maximum_of_pooled_and_each_whole_fold_conformal_quantile".into(),
        nominal_interval_coverage: nominal,
    })
}

fn cross_axis_is_valid(
    axis: &GeneralizedTacticAxisCrossCalibration,
    source_transitions: usize,
    config: GeneralizedTacticCalibrationConfig,
) -> bool {
    let nominal = f64::from(config.interval_coverage_millionths) / 1_000_000.0;
    let test_groups = axis
        .folds
        .iter()
        .flat_map(|fold| fold.test_groups.iter())
        .collect::<BTreeSet<_>>();
    let validation_groups = axis
        .folds
        .iter()
        .flat_map(|fold| fold.validation_groups.iter())
        .collect::<BTreeSet<_>>();
    let all_groups = axis
        .folds
        .first()
        .map(|fold| {
            fold.training_groups
                .iter()
                .chain(&fold.validation_groups)
                .chain(&fold.test_groups)
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();
    axis.folds.len() == usize::from(config.group_folds)
        && axis
            .folds
            .iter()
            .map(|fold| fold.test_groups.len())
            .sum::<usize>()
            == test_groups.len()
        && axis
            .folds
            .iter()
            .map(|fold| fold.validation_groups.len())
            .sum::<usize>()
            == validation_groups.len()
        && test_groups == all_groups
        && validation_groups == all_groups
        && axis.folds.iter().all(|fold| {
            fold.training_groups
                .iter()
                .chain(&fold.validation_groups)
                .chain(&fold.test_groups)
                .collect::<BTreeSet<_>>()
                == all_groups
        })
        && axis
            .folds
            .iter()
            .enumerate()
            .all(|(test_fold, fold)| cross_fold_is_valid(fold, test_fold as u8, config.group_folds))
        && axis.pooled_validation.samples == source_transitions
        && axis.pooled_test.samples == source_transitions
        && axis.conformal_multiplier.is_finite()
        && axis.conformal_multiplier >= 1.0
        && axis.calibration_rule == "maximum_of_pooled_and_each_whole_fold_conformal_quantile"
        && axis.nominal_interval_coverage == nominal
        && metrics_are_valid(&axis.pooled_validation)
        && metrics_are_valid(&axis.pooled_test)
        && axis.test_coverage_at_least_nominal
            == (axis.pooled_test.interval_coverage >= axis.nominal_interval_coverage)
}

fn cross_fold_is_valid(
    fold: &GeneralizedTacticCrossCalibrationFold,
    expected_test_fold: u8,
    group_folds: u8,
) -> bool {
    let training = fold.training_groups.iter().collect::<BTreeSet<_>>();
    let validation = fold.validation_groups.iter().collect::<BTreeSet<_>>();
    let test = fold.test_groups.iter().collect::<BTreeSet<_>>();
    fold.test_fold == expected_test_fold
        && fold.validation_fold == (expected_test_fold + group_folds - 1) % group_folds
        && fold.training_samples >= 2
        && fold.validation_samples > 0
        && fold.test_samples > 0
        && fold.validation_conformal_multiplier.is_finite()
        && fold.validation_conformal_multiplier >= 1.0
        && training.len() == fold.training_groups.len()
        && validation.len() == fold.validation_groups.len()
        && test.len() == fold.test_groups.len()
        && fold.group_overlap_count == group_overlap_count(&training, &validation, &test)
        && fold.group_overlap_count == 0
}
