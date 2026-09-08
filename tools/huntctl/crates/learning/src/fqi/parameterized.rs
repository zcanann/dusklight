//! Bellman fitting with actions embedded in regression features.
//! Successor candidates are supplied per state, so an unavailable action is
//! never maximized over merely because it appeared elsewhere in replay.
use super::*;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ParameterizedTraining {
    pub discount: f32,
    pub completed_backups: u64,
    /// Maximum absolute Bellman target minus prior prediction in the last
    /// backup, over this fit's rows. Not a held-out error or convergence proof.
    pub last_max_target_residual: f64,
}

#[derive(Clone, Debug)]
pub struct ParameterizedTransition {
    pub state_action: Vec<f32>,
    pub reward: f32,
    pub duration: u32,
    pub terminal: bool,
    /// An empty set on a nonterminal row is missing support, not a terminal.
    pub successor_actions: Vec<Vec<f32>>,
    /// Shared prefix of each successor query; candidates supply only suffixes.
    pub successor_state: Vec<f32>,
}

impl FittedQ {
    pub fn fit_parameterized(
        feature_width: usize,
        samples: &[ParameterizedTransition],
        config: &FqiConfig,
    ) -> Result<Self, FqiError> {
        Self::fit_parameterized_from_prior(feature_width, samples, config, None)
    }

    pub fn parameterized_training(&self) -> Option<&ParameterizedTraining> {
        self.parameterized_training.as_ref()
    }

    /// Continue Bellman updates from the previous fitted values. The caller
    /// must preserve feature/task semantics when experience changes.
    pub fn fit_parameterized_from_prior(
        feature_width: usize,
        samples: &[ParameterizedTransition],
        config: &FqiConfig,
        prior: Option<&Self>,
    ) -> Result<Self, FqiError> {
        let previous_backups = if let Some(prior) = prior {
            let training = prior
                .parameterized_training
                .as_ref()
                .ok_or(FqiError::InvalidConfig(
                    "prior is not a parameterized Bellman model",
                ))?;
            if prior.feature_width != feature_width
                || prior.actions != [0]
                || training.discount != config.discount
            {
                return Err(FqiError::InvalidConfig("incompatible Bellman prior"));
            }
            training.completed_backups
        } else {
            0
        };
        if config.backup_steps != 1 {
            return Err(FqiError::InvalidConfig(
                "parameterized fitting uses one-step backups",
            ));
        }
        let mut rows = samples
            .iter()
            .map(|sample| Transition {
                state: sample.state_action.clone(),
                action: 0,
                duration: sample.duration,
                reward: sample.reward,
                next_state: sample.state_action.clone(),
                terminal: sample.terminal,
            })
            .collect::<Vec<_>>();
        validate_inputs(feature_width, &[0], &rows, config)?;
        for sample in samples {
            if !sample.terminal && sample.successor_actions.is_empty() {
                return Err(FqiError::InvalidConfig(
                    "nonterminal parameterized row lacks successor actions",
                ));
            }
            if sample
                .successor_state
                .iter()
                .any(|value| !value.is_finite())
            {
                return Err(FqiError::NonFiniteFeature);
            }
            for successor in &sample.successor_actions {
                let width = sample.successor_state.len() + successor.len();
                if width != feature_width {
                    return Err(FqiError::FeatureWidth {
                        expected: feature_width,
                        actual: width,
                    });
                }
                if successor.iter().any(|value| !value.is_finite()) {
                    return Err(FqiError::NonFiniteFeature);
                }
            }
        }
        let mut current = prior.cloned();
        let regression_config = FqiConfig {
            iterations: 1,
            ..config.clone()
        };
        for iteration in 0..config.iterations {
            let mut max_target_residual = 0.0_f64;
            for (index, (sample, row)) in samples.iter().zip(&mut rows).enumerate() {
                let continuation = if sample.terminal || current.is_none() {
                    0.0
                } else {
                    let model = current.as_ref().unwrap();
                    let mut features = sample.successor_state.clone();
                    let prefix = features.len();
                    features.resize(feature_width, 0.0);
                    let mut best = f64::NEG_INFINITY;
                    for action in &sample.successor_actions {
                        features[prefix..].copy_from_slice(action);
                        best = best.max(model.estimate(&features, 0)?.mean);
                    }
                    best
                };
                let target = f64::from(sample.reward)
                    + f64::from(config.discount).powf(f64::from(sample.duration)) * continuation;
                if !target.is_finite() || !(target as f32).is_finite() {
                    return Err(FqiError::NonFiniteBellmanTarget {
                        iteration,
                        transition: index,
                    });
                }
                let previous = current
                    .as_ref()
                    .map(|model| {
                        model
                            .estimate(&sample.state_action, 0)
                            .map(|estimate| estimate.mean)
                    })
                    .transpose()?
                    .unwrap_or(0.0);
                max_target_residual = max_target_residual.max((target - previous).abs());
                row.reward = target as f32;
                // The outer loop owns the Bellman backup. This row is a
                // supervised regression target, not a relabeled game terminal.
                row.terminal = true;
            }
            let mut fitted = Self::fit(feature_width, &[0], &rows, &regression_config)?;
            fitted.parameterized_training = Some(ParameterizedTraining {
                discount: config.discount,
                completed_backups: previous_backups
                    .checked_add(iteration as u64 + 1)
                    .ok_or(FqiError::InvalidConfig("Bellman backup count overflow"))?,
                last_max_target_residual: max_target_residual,
            });
            current = Some(fitted);
        }
        Ok(current.expect("iterations validated"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(
        x: f32,
        reward: f32,
        duration: u32,
        terminal: bool,
        next: &[f32],
    ) -> ParameterizedTransition {
        ParameterizedTransition {
            state_action: vec![x],
            reward,
            duration,
            terminal,
            successor_actions: next.iter().map(|x| vec![*x]).collect(),
            successor_state: Vec::new(),
        }
    }

    fn config() -> FqiConfig {
        FqiConfig {
            iterations: 6,
            trees_per_action: 1,
            bootstrap: false,
            discount: 0.9,
            ..FqiConfig::default()
        }
    }

    #[test]
    fn resumed_backups_match_uninterrupted_learning_and_escape_short_horizon() {
        let samples = [
            row(0.0, -1.0, 1, false, &[0.0, 1.0]),
            row(1.0, -20.0, 20, true, &[]),
        ];
        let config = FqiConfig {
            iterations: 12,
            discount: 0.999,
            ..config()
        };
        let first = FittedQ::fit_parameterized(1, &samples, &config).unwrap();
        let bytes = serde_cbor::to_vec(&first).unwrap();
        let restored: FittedQ = serde_cbor::from_slice(&bytes).unwrap();
        let resumed =
            FittedQ::fit_parameterized_from_prior(1, &samples, &config, Some(&restored)).unwrap();
        let continuous = FittedQ::fit_parameterized(
            1,
            &samples,
            &FqiConfig {
                iterations: 24,
                ..config.clone()
            },
        )
        .unwrap();
        assert_eq!(
            serde_cbor::to_vec(&resumed).unwrap(),
            serde_cbor::to_vec(&continuous).unwrap()
        );
        assert_eq!(
            resumed.parameterized_training().unwrap().completed_backups,
            24
        );
        assert!(
            resumed.estimate(&[0.0], 0).unwrap().mean < resumed.estimate(&[1.0], 0).unwrap().mean
        );
        assert!(
            FittedQ::fit_parameterized_from_prior(
                1,
                &samples,
                &FqiConfig {
                    discount: 0.9,
                    ..config
                },
                Some(&restored)
            )
            .is_err()
        );
    }

    #[test]
    fn bounded_backups_do_not_establish_cost_to_goal_convergence() {
        // A fully observed self-loop costs one tick; a known terminal action
        // costs twenty. A cold, short fit still optimistically prefers waiting.
        // This is a planning-depth limitation, not missing terminal evidence
        // or an unavailable successor contaminating the target.
        let samples = [
            row(0.0, -1.0, 1, false, &[0.0, 1.0]),
            row(1.0, -20.0, 20, true, &[]),
        ];
        let short = FittedQ::fit_parameterized(
            1,
            &samples,
            &FqiConfig {
                iterations: 12,
                discount: 0.999,
                ..config()
            },
        )
        .unwrap();
        let deeper = FittedQ::fit_parameterized(
            1,
            &samples,
            &FqiConfig {
                iterations: 24,
                discount: 0.999,
                ..config()
            },
        )
        .unwrap();
        let waiting = short.estimate(&[0.0], 0).unwrap().mean;
        let finishing = short.estimate(&[1.0], 0).unwrap().mean;
        assert!(waiting > finishing, "{waiting} vs {finishing}");
        assert!(
            deeper.estimate(&[0.0], 0).unwrap().mean < deeper.estimate(&[1.0], 0).unwrap().mean
        );
    }

    #[test]
    fn factored_successor_queries_match_full_vectors() {
        let rows = vec![
            ParameterizedTransition {
                state_action: vec![0.0, 0.0],
                reward: -1.0,
                duration: 1,
                terminal: false,
                successor_state: vec![1.0],
                successor_actions: vec![vec![0.0], vec![1.0]],
            },
            ParameterizedTransition {
                state_action: vec![1.0, 0.0],
                reward: 2.0,
                duration: 1,
                terminal: true,
                successor_state: vec![],
                successor_actions: vec![],
            },
            ParameterizedTransition {
                state_action: vec![1.0, 1.0],
                reward: 5.0,
                duration: 1,
                terminal: true,
                successor_state: vec![],
                successor_actions: vec![],
            },
        ];
        let mut expanded = rows.clone();
        expanded[0].successor_state.clear();
        expanded[0].successor_actions = vec![vec![1.0, 0.0], vec![1.0, 1.0]];
        let compact = FittedQ::fit_parameterized(2, &rows, &config()).unwrap();
        let full = FittedQ::fit_parameterized(2, &expanded, &config()).unwrap();
        assert_eq!(
            compact.estimate(&[0.0, 0.0], 0).unwrap(),
            full.estimate(&[0.0, 0.0], 0).unwrap()
        );
    }

    #[test]
    fn delayed_rewards_use_only_available_successors_and_real_duration() {
        let samples = vec![
            row(0.0, -1.0, 2, false, &[1.0]),
            row(1.0, 10.0, 1, true, &[]),
            row(2.0, 1000.0, 1, true, &[]),
        ];
        let model = FittedQ::fit_parameterized(1, &samples, &config()).unwrap();
        let q = model.estimate(&[0.0], 0).unwrap().mean;
        assert!(
            (q - 7.1).abs() < 1e-5,
            "unavailable high-value action leaked: {q}"
        );
        assert_eq!(model.estimate(&[1.0], 0).unwrap().mean, 10.0);
    }

    #[test]
    fn nonterminal_cutoffs_bootstrap_instead_of_becoming_failures() {
        let mut settings = config();
        settings.iterations = 3;
        let samples = vec![row(0.0, -1.0, 1, false, &[0.0])];
        let model = FittedQ::fit_parameterized(1, &samples, &settings).unwrap();
        assert!((model.estimate(&[0.0], 0).unwrap().mean + 2.71).abs() < 1e-5);
        let missing = vec![row(0.0, -1.0, 1, false, &[])];
        assert!(FittedQ::fit_parameterized(1, &missing, &settings).is_err());
    }
}
