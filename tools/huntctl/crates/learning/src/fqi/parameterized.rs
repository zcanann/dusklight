//! Bellman fitting with actions embedded in regression features.
//! Successor candidates are supplied per state, so an unavailable action is
//! never maximized over merely because it appeared elsewhere in replay.
use super::*;

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
        let mut current: Option<Self> = None;
        let regression_config = FqiConfig {
            iterations: 1,
            ..config.clone()
        };
        for iteration in 0..config.iterations {
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
                row.reward = target as f32;
                // The outer loop owns the Bellman backup. This row is a
                // supervised regression target, not a relabeled game terminal.
                row.terminal = true;
            }
            current = Some(Self::fit(feature_width, &[0], &rows, &regression_config)?);
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
