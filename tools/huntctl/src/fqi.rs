//! Deterministic, finite-batch fitted Q iteration.
//!
//! The learner deliberately operates on compact, memory-backed feature vectors
//! and discrete macro actions. It is not tied to a game process or tape format.

use std::cmp::Ordering;
use std::error::Error;
use std::fmt;

pub const MAX_FQI_TRANSITIONS: usize = 250_000;
pub const MAX_FQI_ACTIONS: usize = 128;
pub const MAX_FQI_ITERATIONS: usize = 128;
pub const MAX_FQI_TREES_PER_ACTION: usize = 127;
pub const MAX_FQI_TREE_DEPTH: usize = 24;

/// One observed macro-action transition.
#[derive(Clone, Debug, PartialEq)]
pub struct Transition {
    pub state: Vec<f32>,
    pub action: u32,
    /// Number of simulation ticks consumed by the action.
    pub duration: u32,
    pub reward: f32,
    pub next_state: Vec<f32>,
    pub terminal: bool,
}

/// Controls both Bellman fitting and the small regression forests.
#[derive(Clone, Debug, PartialEq)]
pub struct FqiConfig {
    /// Number of fitted Bellman backups.
    pub iterations: usize,
    /// Trees fitted independently for each action on each backup.
    pub trees_per_action: usize,
    pub max_tree_depth: usize,
    pub min_samples_leaf: usize,
    /// Features considered at each split. Zero selects `sqrt(feature_width)`.
    pub features_per_split: usize,
    /// Maximum candidate thresholds inspected per feature and node.
    pub max_thresholds_per_feature: usize,
    /// Per-tick discount. A transition is discounted by `discount^duration`.
    pub discount: f32,
    /// Reproducible seed used for bootstrap samples and feature selection.
    pub seed: u64,
    /// Bootstrap transition rows for each tree. Disabling this is useful for
    /// exact tiny datasets; feature randomization still differentiates trees.
    pub bootstrap: bool,
    /// Feature indices whose finite f32 values are category identifiers rather
    /// than ordered quantities. Trees split these by equality, never by `<=`.
    /// The feature-schema owner must authenticate and supply this metadata.
    pub categorical_features: Vec<usize>,
}

impl Default for FqiConfig {
    fn default() -> Self {
        Self {
            iterations: 24,
            trees_per_action: 31,
            max_tree_depth: 8,
            min_samples_leaf: 1,
            features_per_split: 0,
            max_thresholds_per_feature: 32,
            discount: 0.995,
            seed: 0xd15c_a11d_5eed_f017,
            bootstrap: true,
            categorical_features: Vec::new(),
        }
    }
}

/// Ensemble estimate used to rank an action.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QEstimate {
    pub action: u32,
    pub mean: f64,
    /// Population variance across trees. It is epistemic disagreement, not a
    /// calibrated probability or confidence interval.
    pub variance: f64,
}

/// A fitted, immutable Q function.
#[derive(Clone, Debug)]
pub struct FittedQ {
    feature_width: usize,
    actions: Vec<u32>,
    forests: Vec<RegressionForest>,
}

impl FittedQ {
    /// Fits an action-specific randomized regression forest with repeated
    /// Bellman targets. `actions` defines the complete action set available at
    /// every non-terminal state and must contain each transition action.
    pub fn fit(
        feature_width: usize,
        actions: &[u32],
        transitions: &[Transition],
        config: &FqiConfig,
    ) -> Result<Self, FqiError> {
        validate_inputs(feature_width, actions, transitions, config)?;

        let mut action_set = actions.to_vec();
        action_set.sort_unstable();

        let mut grouped = vec![Vec::new(); action_set.len()];
        for (row, transition) in transitions.iter().enumerate() {
            let action_index = action_set
                .binary_search(&transition.action)
                .expect("transition actions were validated");
            grouped[action_index].push(row);
        }

        let mut current: Option<Self> = None;
        for iteration in 0..config.iterations {
            let targets: Vec<f64> = transitions
                .iter()
                .enumerate()
                .map(|(transition_index, transition)| {
                    let continuation = match current.as_ref() {
                        Some(model) if !transition.terminal => {
                            model
                                .best_action(&transition.next_state)
                                .expect("validated state and non-empty actions")
                                .mean
                        }
                        _ => 0.0,
                    };
                    let target = f64::from(transition.reward)
                        + discount_for_duration(config.discount, transition.duration)
                            * continuation;
                    if target.is_finite() {
                        Ok(target)
                    } else {
                        Err(FqiError::NonFiniteBellmanTarget {
                            iteration,
                            transition: transition_index,
                        })
                    }
                })
                .collect::<Result<_, _>>()?;

            let forests = grouped
                .iter()
                .enumerate()
                .map(|(action_index, rows)| {
                    RegressionForest::fit(
                        transitions,
                        &targets,
                        rows,
                        feature_width,
                        config,
                        derive_seed(config.seed, iteration, action_index),
                    )
                })
                .collect();

            current = Some(Self {
                feature_width,
                actions: action_set.clone(),
                forests,
            });
        }

        Ok(current.expect("iterations are validated as non-zero"))
    }

    pub fn feature_width(&self) -> usize {
        self.feature_width
    }

    pub fn actions(&self) -> &[u32] {
        &self.actions
    }

    pub fn estimate(&self, state: &[f32], action: u32) -> Result<QEstimate, FqiError> {
        self.validate_state(state)?;
        let index = self
            .actions
            .binary_search(&action)
            .map_err(|_| FqiError::UnknownAction(action))?;
        Ok(self.forests[index].estimate(state, action))
    }

    /// Returns highest mean Q first. Equal means prefer lower ensemble
    /// variance, then the numerically smaller action for deterministic ties.
    pub fn rank_actions(&self, state: &[f32]) -> Result<Vec<QEstimate>, FqiError> {
        self.validate_state(state)?;
        let mut ranked: Vec<QEstimate> = self
            .actions
            .iter()
            .zip(&self.forests)
            .map(|(action, forest)| forest.estimate(state, *action))
            .collect();
        ranked.sort_by(|left, right| {
            right
                .mean
                .total_cmp(&left.mean)
                .then_with(|| left.variance.total_cmp(&right.variance))
                .then_with(|| left.action.cmp(&right.action))
        });
        Ok(ranked)
    }

    pub fn best_action(&self, state: &[f32]) -> Result<QEstimate, FqiError> {
        self.rank_actions(state).map(|ranked| ranked[0])
    }

    fn validate_state(&self, state: &[f32]) -> Result<(), FqiError> {
        if state.len() != self.feature_width {
            return Err(FqiError::FeatureWidth {
                expected: self.feature_width,
                actual: state.len(),
            });
        }
        if state.iter().any(|value| !value.is_finite()) {
            return Err(FqiError::NonFiniteFeature);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FqiError {
    EmptyFeatures,
    EmptyActions,
    DuplicateAction(u32),
    MissingActionSamples(u32),
    UnknownAction(u32),
    EmptyTransitions,
    FeatureWidth { expected: usize, actual: usize },
    NonFiniteFeature,
    NonFiniteReward,
    ZeroDuration,
    TooManyTransitions { actual: usize, maximum: usize },
    TooManyActions { actual: usize, maximum: usize },
    CategoricalFeatureOutOfRange { index: usize, feature_width: usize },
    DuplicateCategoricalFeature(usize),
    NonFiniteBellmanTarget { iteration: usize, transition: usize },
    InvalidConfig(&'static str),
}

impl fmt::Display for FqiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyFeatures => write!(formatter, "feature width must be non-zero"),
            Self::EmptyActions => write!(formatter, "action set must be non-empty"),
            Self::DuplicateAction(action) => write!(formatter, "duplicate action {action}"),
            Self::MissingActionSamples(action) => {
                write!(formatter, "action {action} has no transition samples")
            }
            Self::UnknownAction(action) => write!(formatter, "unknown action {action}"),
            Self::EmptyTransitions => write!(formatter, "transition batch must be non-empty"),
            Self::FeatureWidth { expected, actual } => write!(
                formatter,
                "feature width mismatch: expected {expected}, got {actual}"
            ),
            Self::NonFiniteFeature => write!(formatter, "features must all be finite"),
            Self::NonFiniteReward => write!(formatter, "rewards must all be finite"),
            Self::ZeroDuration => write!(formatter, "transition duration must be non-zero"),
            Self::TooManyTransitions { actual, maximum } => write!(
                formatter,
                "transition batch contains {actual} rows; maximum is {maximum}"
            ),
            Self::TooManyActions { actual, maximum } => {
                write!(
                    formatter,
                    "action set contains {actual} actions; maximum is {maximum}"
                )
            }
            Self::CategoricalFeatureOutOfRange {
                index,
                feature_width,
            } => write!(
                formatter,
                "categorical feature index {index} is outside feature width {feature_width}"
            ),
            Self::DuplicateCategoricalFeature(index) => {
                write!(formatter, "duplicate categorical feature index {index}")
            }
            Self::NonFiniteBellmanTarget {
                iteration,
                transition,
            } => write!(
                formatter,
                "Bellman target became non-finite at iteration {iteration}, transition {transition}"
            ),
            Self::InvalidConfig(message) => write!(formatter, "invalid FQI config: {message}"),
        }
    }
}

impl Error for FqiError {}

fn validate_inputs(
    feature_width: usize,
    actions: &[u32],
    transitions: &[Transition],
    config: &FqiConfig,
) -> Result<(), FqiError> {
    if feature_width == 0 {
        return Err(FqiError::EmptyFeatures);
    }
    if actions.is_empty() {
        return Err(FqiError::EmptyActions);
    }
    if transitions.is_empty() {
        return Err(FqiError::EmptyTransitions);
    }
    if transitions.len() > MAX_FQI_TRANSITIONS {
        return Err(FqiError::TooManyTransitions {
            actual: transitions.len(),
            maximum: MAX_FQI_TRANSITIONS,
        });
    }
    if actions.len() > MAX_FQI_ACTIONS {
        return Err(FqiError::TooManyActions {
            actual: actions.len(),
            maximum: MAX_FQI_ACTIONS,
        });
    }
    if config.iterations == 0 {
        return Err(FqiError::InvalidConfig("iterations must be non-zero"));
    }
    if config.iterations > MAX_FQI_ITERATIONS {
        return Err(FqiError::InvalidConfig("iterations must not exceed 128"));
    }
    if config.trees_per_action == 0 {
        return Err(FqiError::InvalidConfig("trees_per_action must be non-zero"));
    }
    if config.trees_per_action > MAX_FQI_TREES_PER_ACTION {
        return Err(FqiError::InvalidConfig(
            "trees_per_action must not exceed 127",
        ));
    }
    if config.max_tree_depth > MAX_FQI_TREE_DEPTH {
        return Err(FqiError::InvalidConfig("max_tree_depth must not exceed 24"));
    }
    if config.min_samples_leaf == 0 {
        return Err(FqiError::InvalidConfig("min_samples_leaf must be non-zero"));
    }
    if config.max_thresholds_per_feature == 0 {
        return Err(FqiError::InvalidConfig(
            "max_thresholds_per_feature must be non-zero",
        ));
    }
    if !config.discount.is_finite() || !(0.0..=1.0).contains(&config.discount) {
        return Err(FqiError::InvalidConfig(
            "discount must be finite and between zero and one",
        ));
    }
    let mut categorical_features = config.categorical_features.clone();
    categorical_features.sort_unstable();
    if let Some(index) = categorical_features
        .windows(2)
        .find(|pair| pair[0] == pair[1])
        .map(|pair| pair[0])
    {
        return Err(FqiError::DuplicateCategoricalFeature(index));
    }
    if let Some(index) = categorical_features
        .into_iter()
        .find(|index| *index >= feature_width)
    {
        return Err(FqiError::CategoricalFeatureOutOfRange {
            index,
            feature_width,
        });
    }

    let mut sorted_actions = actions.to_vec();
    sorted_actions.sort_unstable();
    if let Some(action) = sorted_actions
        .windows(2)
        .find(|pair| pair[0] == pair[1])
        .map(|pair| pair[0])
    {
        return Err(FqiError::DuplicateAction(action));
    }

    let mut action_seen = vec![false; sorted_actions.len()];
    for transition in transitions {
        if transition.state.len() != feature_width {
            return Err(FqiError::FeatureWidth {
                expected: feature_width,
                actual: transition.state.len(),
            });
        }
        if transition.next_state.len() != feature_width {
            return Err(FqiError::FeatureWidth {
                expected: feature_width,
                actual: transition.next_state.len(),
            });
        }
        if transition
            .state
            .iter()
            .chain(&transition.next_state)
            .any(|value| !value.is_finite())
        {
            return Err(FqiError::NonFiniteFeature);
        }
        if !transition.reward.is_finite() {
            return Err(FqiError::NonFiniteReward);
        }
        if transition.duration == 0 {
            return Err(FqiError::ZeroDuration);
        }
        let action_index = sorted_actions
            .binary_search(&transition.action)
            .map_err(|_| FqiError::UnknownAction(transition.action))?;
        action_seen[action_index] = true;
    }
    for (action, seen) in sorted_actions.into_iter().zip(action_seen) {
        if !seen {
            return Err(FqiError::MissingActionSamples(action));
        }
    }
    Ok(())
}

#[derive(Clone, Debug)]
struct RegressionForest {
    trees: Vec<RegressionTree>,
}

impl RegressionForest {
    fn fit(
        transitions: &[Transition],
        targets: &[f64],
        action_rows: &[usize],
        feature_width: usize,
        config: &FqiConfig,
        seed: u64,
    ) -> Self {
        let trees = (0..config.trees_per_action)
            .map(|tree_index| {
                let mut random = SplitMix64::new(mix64(seed ^ tree_index as u64));
                let rows = if config.bootstrap {
                    (0..action_rows.len())
                        .map(|_| action_rows[random.index(action_rows.len())])
                        .collect()
                } else {
                    action_rows.to_vec()
                };
                RegressionTree::fit(
                    transitions,
                    targets,
                    &rows,
                    feature_width,
                    config,
                    &mut random,
                )
            })
            .collect();
        Self { trees }
    }

    fn estimate(&self, state: &[f32], action: u32) -> QEstimate {
        let count = self.trees.len() as f64;
        let values: Vec<f64> = self.trees.iter().map(|tree| tree.predict(state)).collect();
        let mean = values.iter().sum::<f64>() / count;
        let variance = values
            .iter()
            .map(|value| {
                let delta = value - mean;
                delta * delta
            })
            .sum::<f64>()
            / count;
        QEstimate {
            action,
            mean,
            variance,
        }
    }
}

#[derive(Clone, Debug)]
struct RegressionTree {
    root: TreeNode,
}

impl RegressionTree {
    fn fit(
        transitions: &[Transition],
        targets: &[f64],
        rows: &[usize],
        feature_width: usize,
        config: &FqiConfig,
        random: &mut SplitMix64,
    ) -> Self {
        let root = TreeNode::fit(transitions, targets, rows, feature_width, config, random, 0);
        Self { root }
    }

    fn predict(&self, state: &[f32]) -> f64 {
        self.root.predict(state)
    }
}

#[derive(Clone, Debug)]
enum TreeNode {
    Leaf(f64),
    Split {
        feature: usize,
        rule: SplitRule,
        left: Box<TreeNode>,
        right: Box<TreeNode>,
    },
}

impl TreeNode {
    #[allow(clippy::too_many_arguments)]
    fn fit(
        transitions: &[Transition],
        targets: &[f64],
        rows: &[usize],
        feature_width: usize,
        config: &FqiConfig,
        random: &mut SplitMix64,
        depth: usize,
    ) -> Self {
        let leaf_value = mean_target(targets, rows);
        if depth >= config.max_tree_depth
            || rows.len() < config.min_samples_leaf.saturating_mul(2)
            || target_sse(targets, rows, leaf_value) <= f64::EPSILON
        {
            return Self::Leaf(leaf_value);
        }

        let requested_features = if config.features_per_split == 0 {
            integer_sqrt(feature_width).max(1)
        } else {
            config.features_per_split.min(feature_width)
        };
        let mut features: Vec<usize> = (0..feature_width).collect();
        random.shuffle(&mut features);
        features.truncate(requested_features);

        let mut best: Option<SplitCandidate> = None;
        for feature in features {
            let mut values: Vec<f32> = rows
                .iter()
                .map(|row| transitions[*row].state[feature])
                .collect();
            values.sort_by(f32::total_cmp);
            values.dedup_by(|left, right| left.total_cmp(right) == Ordering::Equal);
            if values.len() < 2 {
                continue;
            }
            let rules: Vec<SplitRule> = if config.categorical_features.contains(&feature) {
                random.shuffle(&mut values);
                values.truncate(config.max_thresholds_per_feature);
                values
                    .into_iter()
                    .map(SplitRule::CategoricalEqual)
                    .collect()
            } else {
                let boundary_count = values.len() - 1;
                let threshold_slots = config.max_thresholds_per_feature.min(boundary_count);
                (0..threshold_slots)
                    .map(|slot| {
                        let boundary = if threshold_slots == boundary_count {
                            slot
                        } else {
                            slot * boundary_count / threshold_slots
                        };
                        let threshold =
                            ((values[boundary] as f64 + values[boundary + 1] as f64) * 0.5) as f32;
                        SplitRule::NumericLessOrEqual(threshold)
                    })
                    .collect()
            };
            for rule in rules {
                let (left, right): (Vec<usize>, Vec<usize>) = rows
                    .iter()
                    .copied()
                    .partition(|row| rule.goes_left(transitions[*row].state[feature]));
                if left.len() < config.min_samples_leaf || right.len() < config.min_samples_leaf {
                    continue;
                }
                let error = target_sse(targets, &left, mean_target(targets, &left))
                    + target_sse(targets, &right, mean_target(targets, &right));
                let candidate = SplitCandidate {
                    feature,
                    rule,
                    error,
                    left,
                    right,
                };
                if best
                    .as_ref()
                    .is_none_or(|existing| candidate.better_than(existing))
                {
                    best = Some(candidate);
                }
            }
        }

        let Some(split) = best else {
            return Self::Leaf(leaf_value);
        };
        Self::Split {
            feature: split.feature,
            rule: split.rule,
            left: Box::new(Self::fit(
                transitions,
                targets,
                &split.left,
                feature_width,
                config,
                random,
                depth + 1,
            )),
            right: Box::new(Self::fit(
                transitions,
                targets,
                &split.right,
                feature_width,
                config,
                random,
                depth + 1,
            )),
        }
    }

    fn predict(&self, state: &[f32]) -> f64 {
        match self {
            Self::Leaf(value) => *value,
            Self::Split {
                feature,
                rule,
                left,
                right,
            } => {
                if rule.goes_left(state[*feature]) {
                    left.predict(state)
                } else {
                    right.predict(state)
                }
            }
        }
    }
}

struct SplitCandidate {
    feature: usize,
    rule: SplitRule,
    error: f64,
    left: Vec<usize>,
    right: Vec<usize>,
}

#[derive(Clone, Copy, Debug)]
enum SplitRule {
    NumericLessOrEqual(f32),
    CategoricalEqual(f32),
}

impl SplitRule {
    fn goes_left(self, value: f32) -> bool {
        match self {
            Self::NumericLessOrEqual(threshold) => value <= threshold,
            Self::CategoricalEqual(category) => value == category,
        }
    }

    fn tie_cmp(self, other: Self) -> Ordering {
        match (self, other) {
            (Self::NumericLessOrEqual(left), Self::NumericLessOrEqual(right))
            | (Self::CategoricalEqual(left), Self::CategoricalEqual(right)) => {
                left.total_cmp(&right)
            }
            (Self::NumericLessOrEqual(_), Self::CategoricalEqual(_)) => Ordering::Less,
            (Self::CategoricalEqual(_), Self::NumericLessOrEqual(_)) => Ordering::Greater,
        }
    }
}

impl SplitCandidate {
    fn better_than(&self, other: &Self) -> bool {
        self.error
            .total_cmp(&other.error)
            .then_with(|| self.feature.cmp(&other.feature))
            .then_with(|| self.rule.tie_cmp(other.rule))
            == Ordering::Less
    }
}

fn mean_target(targets: &[f64], rows: &[usize]) -> f64 {
    rows.iter().map(|row| targets[*row]).sum::<f64>() / rows.len() as f64
}

fn target_sse(targets: &[f64], rows: &[usize], mean: f64) -> f64 {
    rows.iter()
        .map(|row| {
            let delta = targets[*row] - mean;
            delta * delta
        })
        .sum()
}

fn integer_sqrt(value: usize) -> usize {
    (value as f64).sqrt().floor() as usize
}

fn discount_for_duration(discount: f32, mut duration: u32) -> f64 {
    let mut discount = f64::from(discount);
    let mut result = 1.0;
    while duration != 0 {
        if duration & 1 != 0 {
            result *= discount;
        }
        discount *= discount;
        duration >>= 1;
    }
    result
}

fn derive_seed(seed: u64, iteration: usize, action: usize) -> u64 {
    mix64(seed ^ (iteration as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15))
        ^ mix64((action as u64).wrapping_mul(0xd1b5_4a32_d192_ed03))
}

fn mix64(mut value: u64) -> u64 {
    value ^= value >> 30;
    value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        mix64(self.state)
    }

    fn index(&mut self, exclusive_end: usize) -> usize {
        (self.next() % exclusive_end as u64) as usize
    }

    fn shuffle<T>(&mut self, values: &mut [T]) {
        for upper in (1..values.len()).rev() {
            let selected = self.index(upper + 1);
            values.swap(upper, selected);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ADVANCE: u32 = 3;
    const WAIT: u32 = 9;

    fn transition(from: f32, action: u32, reward: f32, to: f32, terminal: bool) -> Transition {
        Transition {
            state: vec![from],
            action,
            duration: 1,
            reward,
            next_state: vec![to],
            terminal,
        }
    }

    fn path_transition(
        from: f32,
        nuisance: f32,
        action: u32,
        reward: f32,
        to: f32,
        terminal: bool,
    ) -> Transition {
        Transition {
            state: vec![from, nuisance],
            action,
            duration: 1,
            reward,
            next_state: vec![to, nuisance],
            terminal,
        }
    }

    #[test]
    fn learns_a_finite_batch_shortest_path() {
        // Complete deterministic observations for a two-edge path. Waiting is
        // legal but costs a tick. The second feature is irrelevant sensor noise;
        // zero is deliberately absent so policy checks are out-of-sample.
        let mut batch = Vec::new();
        for nuisance in [-1.0, 1.0] {
            batch.extend([
                path_transition(0.0, nuisance, ADVANCE, 0.0, 1.0, false),
                path_transition(0.0, nuisance, WAIT, -1.0, 0.0, false),
                path_transition(1.0, nuisance, ADVANCE, 10.0, 2.0, true),
                path_transition(1.0, nuisance, WAIT, -1.0, 1.0, false),
            ]);
        }
        let config = FqiConfig {
            iterations: 16,
            trees_per_action: 7,
            max_tree_depth: 3,
            features_per_split: 2,
            discount: 0.9,
            bootstrap: false,
            ..FqiConfig::default()
        };

        let model = FittedQ::fit(2, &[WAIT, ADVANCE], &batch, &config).unwrap();
        assert_eq!(model.best_action(&[0.0, 0.0]).unwrap().action, ADVANCE);
        assert_eq!(model.best_action(&[1.0, 0.0]).unwrap().action, ADVANCE);

        let q0 = model.estimate(&[0.0, 0.0], ADVANCE).unwrap().mean;
        let q1 = model.estimate(&[1.0, 0.0], ADVANCE).unwrap().mean;
        assert!((q0 - 9.0).abs() < 0.001, "Q(start, advance)={q0}");
        assert!((q1 - 10.0).abs() < 0.001, "Q(next, advance)={q1}");

        // Execute the learned greedy policy against the tiny benchmark.
        let mut position = 0;
        let mut steps = 0;
        while position < 2 && steps < 4 {
            let action = model.best_action(&[position as f32, 0.0]).unwrap().action;
            if action == ADVANCE {
                position += 1;
            }
            steps += 1;
        }
        assert_eq!((position, steps), (2, 2));
    }

    #[test]
    fn seeded_bootstrap_is_reproducible_and_reports_disagreement() {
        let batch = vec![
            transition(0.0, ADVANCE, 0.0, 1.0, false),
            transition(1.0, ADVANCE, 4.0, 2.0, true),
            transition(0.0, WAIT, -1.0, 0.0, false),
            transition(1.0, WAIT, -1.0, 1.0, false),
        ];
        let config = FqiConfig {
            iterations: 5,
            trees_per_action: 23,
            seed: 42,
            ..FqiConfig::default()
        };
        let first = FittedQ::fit(1, &[ADVANCE, WAIT], &batch, &config).unwrap();
        let second = FittedQ::fit(1, &[ADVANCE, WAIT], &batch, &config).unwrap();

        let first_rank = first.rank_actions(&[0.0]).unwrap();
        let second_rank = second.rank_actions(&[0.0]).unwrap();
        assert_eq!(first_rank, second_rank);
        assert!(first_rank.iter().any(|estimate| estimate.variance > 0.0));
    }

    #[test]
    fn elapsed_ticks_are_applied_to_discount() {
        let batch = vec![
            Transition {
                state: vec![0.0],
                action: ADVANCE,
                duration: 3,
                reward: 1.0,
                next_state: vec![1.0],
                terminal: false,
            },
            transition(1.0, ADVANCE, 8.0, 2.0, true),
        ];
        let config = FqiConfig {
            iterations: 3,
            trees_per_action: 1,
            max_tree_depth: 2,
            discount: 0.5,
            bootstrap: false,
            ..FqiConfig::default()
        };
        let model = FittedQ::fit(1, &[ADVANCE], &batch, &config).unwrap();
        assert!((model.estimate(&[0.0], ADVANCE).unwrap().mean - 2.0).abs() < 0.001);
    }

    #[test]
    fn categorical_feature_uses_equality_not_numeric_order() {
        let batch = vec![
            transition(1.0, ADVANCE, 0.0, 1.0, true),
            transition(2.0, ADVANCE, 10.0, 2.0, true),
            transition(3.0, ADVANCE, 0.0, 3.0, true),
        ];
        let config = FqiConfig {
            iterations: 1,
            trees_per_action: 1,
            max_tree_depth: 1,
            features_per_split: 1,
            bootstrap: false,
            categorical_features: vec![0],
            ..FqiConfig::default()
        };
        let model = FittedQ::fit(1, &[ADVANCE], &batch, &config).unwrap();

        assert_eq!(model.estimate(&[2.0], ADVANCE).unwrap().mean, 10.0);
        assert_eq!(model.estimate(&[1.0], ADVANCE).unwrap().mean, 0.0);
        assert_eq!(model.estimate(&[3.0], ADVANCE).unwrap().mean, 0.0);
        // An unseen numeric value is merely "not category 2". Its proximity
        // to 2 cannot pull it into the special category's leaf.
        assert_eq!(model.estimate(&[2.5], ADVANCE).unwrap().mean, 0.0);
    }

    #[test]
    fn bellman_values_do_not_overflow_at_f32_range() {
        let batch = vec![Transition {
            state: vec![0.0],
            action: ADVANCE,
            duration: 1,
            reward: f32::MAX,
            next_state: vec![0.0],
            terminal: false,
        }];
        let config = FqiConfig {
            iterations: MAX_FQI_ITERATIONS,
            trees_per_action: 1,
            max_tree_depth: 0,
            discount: 1.0,
            bootstrap: false,
            ..FqiConfig::default()
        };
        let estimate = FittedQ::fit(1, &[ADVANCE], &batch, &config)
            .unwrap()
            .estimate(&[0.0], ADVANCE)
            .unwrap();
        assert!(estimate.mean.is_finite());
        assert!(estimate.variance.is_finite());
        assert!(estimate.mean > f64::from(f32::MAX));
    }

    #[test]
    fn validates_batch_and_query_boundaries() {
        let valid = vec![transition(0.0, ADVANCE, 0.0, 1.0, true)];
        let config = FqiConfig::default();
        assert_eq!(
            FittedQ::fit(2, &[ADVANCE], &valid, &config).unwrap_err(),
            FqiError::FeatureWidth {
                expected: 2,
                actual: 1
            }
        );
        assert_eq!(
            FittedQ::fit(1, &[ADVANCE, ADVANCE], &valid, &config).unwrap_err(),
            FqiError::DuplicateAction(ADVANCE)
        );
        assert_eq!(
            FittedQ::fit(1, &[ADVANCE, WAIT], &valid, &config).unwrap_err(),
            FqiError::MissingActionSamples(WAIT)
        );

        let model = FittedQ::fit(1, &[ADVANCE], &valid, &config).unwrap();
        assert_eq!(
            model.estimate(&[], ADVANCE).unwrap_err(),
            FqiError::FeatureWidth {
                expected: 1,
                actual: 0
            }
        );
        assert_eq!(
            model.estimate(&[0.0], WAIT).unwrap_err(),
            FqiError::UnknownAction(WAIT)
        );

        let invalid_config = FqiConfig {
            categorical_features: vec![1],
            ..config.clone()
        };
        assert_eq!(
            FittedQ::fit(1, &[ADVANCE], &valid, &invalid_config).unwrap_err(),
            FqiError::CategoricalFeatureOutOfRange {
                index: 1,
                feature_width: 1
            }
        );
        let invalid_config = FqiConfig {
            categorical_features: vec![0, 0],
            ..config.clone()
        };
        assert_eq!(
            FittedQ::fit(1, &[ADVANCE], &valid, &invalid_config).unwrap_err(),
            FqiError::DuplicateCategoricalFeature(0)
        );
        let invalid_config = FqiConfig {
            iterations: MAX_FQI_ITERATIONS + 1,
            ..config.clone()
        };
        assert_eq!(
            FittedQ::fit(1, &[ADVANCE], &valid, &invalid_config).unwrap_err(),
            FqiError::InvalidConfig("iterations must not exceed 128")
        );
        let too_many_actions: Vec<u32> = (0..=(MAX_FQI_ACTIONS as u32)).collect();
        assert_eq!(
            FittedQ::fit(1, &too_many_actions, &valid, &config).unwrap_err(),
            FqiError::TooManyActions {
                actual: MAX_FQI_ACTIONS + 1,
                maximum: MAX_FQI_ACTIONS
            }
        );

        let mut invalid = valid[0].clone();
        invalid.reward = f32::NAN;
        assert_eq!(
            FittedQ::fit(1, &[ADVANCE], &[invalid], &config).unwrap_err(),
            FqiError::NonFiniteReward
        );
        let mut invalid = valid[0].clone();
        invalid.next_state[0] = f32::INFINITY;
        assert_eq!(
            FittedQ::fit(1, &[ADVANCE], &[invalid], &config).unwrap_err(),
            FqiError::NonFiniteFeature
        );
        let mut invalid = valid[0].clone();
        invalid.duration = 0;
        assert_eq!(
            FittedQ::fit(1, &[ADVANCE], &[invalid], &config).unwrap_err(),
            FqiError::ZeroDuration
        );
    }
}
