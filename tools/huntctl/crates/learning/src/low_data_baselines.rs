//! Deterministic nonparametric baselines for small offline state spaces.

use crate::fqi::Transition;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

pub const MAX_LOCAL_RETURN_SAMPLES: usize = 250_000;
pub const MAX_LOCAL_RETURN_NEIGHBORS: usize = 256;
pub const MAX_TABULAR_AXES: usize = 8;
pub const MAX_TABULAR_CELLS: usize = 100_000;

#[derive(Clone, Debug)]
pub struct ReturnSample {
    pub state: Vec<f32>,
    pub action: u32,
    pub return_to_go: f64,
    pub episode_group: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct LocalFeature {
    pub index: usize,
    pub scale: f64,
    pub categorical: bool,
}

#[derive(Clone, Debug)]
pub struct LocalReturnConfig {
    pub neighbors: usize,
    pub features: Vec<LocalFeature>,
}

#[derive(Clone, Copy, Debug)]
pub struct TabularAxis {
    pub index: usize,
    pub origin: f64,
    pub width: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct BaselineEstimate {
    pub action: u32,
    pub mean_return: f64,
    pub support: usize,
    pub nearest_distance_squared: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct NearestNeighborReturn {
    feature_width: usize,
    config: LocalReturnConfig,
    samples: Vec<ReturnSample>,
    actions: Vec<u32>,
}

#[derive(Clone, Debug)]
pub struct TabularReturn {
    feature_width: usize,
    axes: Vec<TabularAxis>,
    cells: BTreeMap<(Vec<i32>, u32), (f64, usize)>,
    actions: Vec<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BaselineError(String);

impl BaselineError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for BaselineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for BaselineError {}

pub fn empirical_return_samples(
    transitions: &[Transition],
    episode_groups: &[u64],
    per_tick_discount: f32,
) -> Result<Vec<ReturnSample>, BaselineError> {
    if transitions.is_empty()
        || transitions.len() > MAX_LOCAL_RETURN_SAMPLES
        || episode_groups.len() != transitions.len()
        || !per_tick_discount.is_finite()
        || !(0.0..=1.0).contains(&per_tick_discount)
    {
        return Err(BaselineError::new("invalid bounded empirical-return batch"));
    }
    let width = transitions[0].state.len();
    if width == 0 {
        return Err(BaselineError::new("return samples require features"));
    }
    let mut grouped = BTreeMap::<u64, Vec<usize>>::new();
    for (index, (transition, group)) in transitions.iter().zip(episode_groups).enumerate() {
        if transition.state.len() != width
            || transition.next_state.len() != width
            || transition.duration == 0
            || !transition.reward.is_finite()
            || !transition
                .state
                .iter()
                .chain(&transition.next_state)
                .all(|value| value.is_finite())
        {
            return Err(BaselineError::new("invalid empirical-return transition"));
        }
        grouped.entry(*group).or_default().push(index);
    }
    let mut returns = vec![0.0_f64; transitions.len()];
    for indices in grouped.values() {
        let mut continuation = 0.0_f64;
        for index in indices.iter().rev() {
            let transition = &transitions[*index];
            let discounted = f64::from(per_tick_discount).powf(f64::from(transition.duration));
            let value = f64::from(transition.reward)
                + if transition.terminal {
                    0.0
                } else {
                    discounted * continuation
                };
            if !value.is_finite() {
                return Err(BaselineError::new("empirical return is non-finite"));
            }
            returns[*index] = value;
            continuation = value;
        }
    }
    Ok(transitions
        .iter()
        .enumerate()
        .map(|(index, transition)| ReturnSample {
            state: transition.state.clone(),
            action: transition.action,
            return_to_go: returns[index],
            episode_group: episode_groups[index],
        })
        .collect())
}

impl NearestNeighborReturn {
    pub fn fit(
        samples: Vec<ReturnSample>,
        config: LocalReturnConfig,
    ) -> Result<Self, BaselineError> {
        let feature_width = validate_samples(&samples)?;
        if config.neighbors == 0
            || config.neighbors > MAX_LOCAL_RETURN_NEIGHBORS
            || config.features.is_empty()
            || config.features.len() > feature_width
        {
            return Err(BaselineError::new("invalid nearest-neighbor configuration"));
        }
        let mut seen = BTreeSet::new();
        for feature in &config.features {
            if feature.index >= feature_width
                || !feature.scale.is_finite()
                || feature.scale <= 0.0
                || !seen.insert(feature.index)
            {
                return Err(BaselineError::new(
                    "local features must be unique, in range, and positively scaled",
                ));
            }
        }
        let actions = samples
            .iter()
            .map(|sample| sample.action)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        Ok(Self {
            feature_width,
            config,
            samples,
            actions,
        })
    }

    pub fn rank(&self, state: &[f32]) -> Result<Vec<BaselineEstimate>, BaselineError> {
        validate_query(state, self.feature_width)?;
        let mut ranking = Vec::new();
        for action in &self.actions {
            let mut neighbors = self
                .samples
                .iter()
                .enumerate()
                .filter(|(_, sample)| sample.action == *action)
                .map(|(index, sample)| (self.distance(state, &sample.state), index, sample))
                .collect::<Vec<_>>();
            neighbors.sort_by(|left, right| {
                left.0
                    .total_cmp(&right.0)
                    .then_with(|| left.1.cmp(&right.1))
            });
            neighbors.truncate(self.config.neighbors);
            let mean_return = neighbors
                .iter()
                .map(|(_, _, sample)| sample.return_to_go)
                .sum::<f64>()
                / neighbors.len() as f64;
            ranking.push(BaselineEstimate {
                action: *action,
                mean_return,
                support: neighbors.len(),
                nearest_distance_squared: neighbors.first().map(|neighbor| neighbor.0),
            });
        }
        rank_estimates(&mut ranking);
        Ok(ranking)
    }

    fn distance(&self, left: &[f32], right: &[f32]) -> f64 {
        self.config
            .features
            .iter()
            .map(|feature| {
                if feature.categorical {
                    f64::from(left[feature.index] != right[feature.index])
                } else {
                    let delta =
                        f64::from(left[feature.index] - right[feature.index]) / feature.scale;
                    delta * delta
                }
            })
            .sum()
    }
}

impl TabularReturn {
    pub fn fit(samples: &[ReturnSample], axes: Vec<TabularAxis>) -> Result<Self, BaselineError> {
        let feature_width = validate_samples(samples)?;
        if axes.is_empty() || axes.len() > MAX_TABULAR_AXES {
            return Err(BaselineError::new("invalid tabular axis count"));
        }
        let mut seen = BTreeSet::new();
        for axis in &axes {
            if axis.index >= feature_width
                || !axis.origin.is_finite()
                || !axis.width.is_finite()
                || axis.width <= 0.0
                || !seen.insert(axis.index)
            {
                return Err(BaselineError::new("invalid tabular axis"));
            }
        }
        let mut cells = BTreeMap::<(Vec<i32>, u32), (f64, usize)>::new();
        let mut actions = BTreeSet::new();
        for sample in samples {
            let key = tabular_key(&sample.state, &axes)?;
            let entry = cells.entry((key, sample.action)).or_default();
            entry.0 += sample.return_to_go;
            entry.1 += 1;
            actions.insert(sample.action);
            if cells.len() > MAX_TABULAR_CELLS {
                return Err(BaselineError::new("tabular cell bound exceeded"));
            }
        }
        Ok(Self {
            feature_width,
            axes,
            cells,
            actions: actions.into_iter().collect(),
        })
    }

    pub fn rank(&self, state: &[f32]) -> Result<Vec<BaselineEstimate>, BaselineError> {
        validate_query(state, self.feature_width)?;
        let key = tabular_key(state, &self.axes)?;
        let mut ranking = self
            .actions
            .iter()
            .filter_map(|action| {
                self.cells
                    .get(&(key.clone(), *action))
                    .map(|(sum, count)| BaselineEstimate {
                        action: *action,
                        mean_return: *sum / *count as f64,
                        support: *count,
                        nearest_distance_squared: None,
                    })
            })
            .collect::<Vec<_>>();
        rank_estimates(&mut ranking);
        Ok(ranking)
    }
}

fn validate_samples(samples: &[ReturnSample]) -> Result<usize, BaselineError> {
    if samples.is_empty() || samples.len() > MAX_LOCAL_RETURN_SAMPLES {
        return Err(BaselineError::new("invalid return sample count"));
    }
    let width = samples[0].state.len();
    if width == 0
        || samples.iter().any(|sample| {
            sample.state.len() != width
                || sample.state.iter().any(|value| !value.is_finite())
                || !sample.return_to_go.is_finite()
        })
    {
        return Err(BaselineError::new("invalid return samples"));
    }
    Ok(width)
}

fn validate_query(state: &[f32], width: usize) -> Result<(), BaselineError> {
    if state.len() != width || state.iter().any(|value| !value.is_finite()) {
        return Err(BaselineError::new("invalid baseline query state"));
    }
    Ok(())
}

fn tabular_key(state: &[f32], axes: &[TabularAxis]) -> Result<Vec<i32>, BaselineError> {
    axes.iter()
        .map(|axis| {
            let bin = ((f64::from(state[axis.index]) - axis.origin) / axis.width).floor();
            if !bin.is_finite() || bin < f64::from(i32::MIN) || bin > f64::from(i32::MAX) {
                return Err(BaselineError::new("tabular bin is outside i32"));
            }
            Ok(bin as i32)
        })
        .collect()
}

fn rank_estimates(ranking: &mut [BaselineEstimate]) {
    ranking.sort_by(|left, right| {
        right
            .mean_return
            .total_cmp(&left.mean_return)
            .then_with(|| right.support.cmp(&left.support))
            .then_with(|| left.action.cmp(&right.action))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transition(state: f32, action: u32, reward: f32, next: f32, terminal: bool) -> Transition {
        Transition {
            state: vec![state],
            action,
            duration: 1,
            reward,
            next_state: vec![next],
            terminal,
        }
    }

    #[test]
    fn local_and_tabular_returns_rank_only_observed_neighborhoods() {
        let transitions = vec![
            transition(0.0, 1, 0.0, 0.1, false),
            transition(0.1, 1, 5.0, 0.2, true),
            transition(0.0, 2, -1.0, 0.1, true),
            transition(10.0, 2, 20.0, 10.1, true),
        ];
        let samples = empirical_return_samples(&transitions, &[1, 1, 2, 3], 1.0).unwrap();
        assert_eq!(samples[0].return_to_go, 5.0);
        let local = NearestNeighborReturn::fit(
            samples.clone(),
            LocalReturnConfig {
                neighbors: 1,
                features: vec![LocalFeature {
                    index: 0,
                    scale: 1.0,
                    categorical: false,
                }],
            },
        )
        .unwrap();
        assert_eq!(local.rank(&[0.0]).unwrap()[0].action, 1);

        let table = TabularReturn::fit(
            &samples,
            vec![TabularAxis {
                index: 0,
                origin: 0.0,
                width: 1.0,
            }],
        )
        .unwrap();
        assert_eq!(table.rank(&[0.0]).unwrap()[0].action, 1);
        assert!(table.rank(&[100.0]).unwrap().is_empty());
    }

    #[test]
    fn truncated_episode_end_does_not_bootstrap_across_episode_groups() {
        let transitions = vec![
            transition(0.0, 1, 2.0, 1.0, false),
            transition(10.0, 1, 100.0, 11.0, true),
        ];
        let samples = empirical_return_samples(&transitions, &[1, 2], 1.0).unwrap();
        assert_eq!(samples[0].return_to_go, 2.0);
        assert_eq!(samples[1].return_to_go, 100.0);
    }
}
