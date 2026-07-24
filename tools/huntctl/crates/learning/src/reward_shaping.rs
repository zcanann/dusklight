//! Authenticated potential-based reward shaping for proposal models.
//!
//! Shaping is deliberately downstream of terminal predicate evaluation. It may
//! change a learner's proposal signal, but it cannot make a candidate feasible
//! or alter the deterministic leaderboard objective.

use crate::artifact::Digest;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::HashSet;
use std::error::Error;
use std::fmt;

pub const POTENTIAL_SHAPING_SCHEMA_V1: &str = "dusklight-potential-shaping/v1";
pub const REWARD_REPORT_SCHEMA_V1: &str = "dusklight-reward-components/v1";
pub const TACTIC_REWARD_SPEC_SCHEMA_V1: &str = "dusklight-tactic-reward-spec/v1";
const MAX_TERMS: usize = 64;
const MAX_ORDERED_VALUES: usize = 64;
const MAX_NAME_BYTES: usize = 64;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PotentialShapingSpec {
    pub schema: String,
    /// Exact feature schema whose indices and units give the terms meaning.
    pub feature_schema: Digest,
    pub terms: Vec<PotentialTerm>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PotentialTerm {
    /// Negative absolute distance from one declared goal value.
    Distance {
        name: String,
        feature: usize,
        goal: f32,
        scale: f32,
        weight: f32,
        #[serde(default)]
        unavailable_value: Option<f32>,
    },
    /// Clamped progress from `start` to `end`; decreasing corridors are valid.
    CorridorProgress {
        name: String,
        feature: usize,
        start: f32,
        end: f32,
        weight: f32,
        #[serde(default)]
        unavailable_value: Option<f32>,
    },
    /// Exact ordered phase values. Unlisted values are unavailable, not guessed.
    PhaseProgress {
        name: String,
        feature: usize,
        ordered_values: Vec<f32>,
        weight: f32,
        #[serde(default)]
        unavailable_value: Option<f32>,
    },
    /// Exact ordered event-progress values.
    EventProgress {
        name: String,
        feature: usize,
        ordered_values: Vec<f32>,
        weight: f32,
        #[serde(default)]
        unavailable_value: Option<f32>,
    },
}

impl PotentialTerm {
    fn name(&self) -> &str {
        match self {
            Self::Distance { name, .. }
            | Self::CorridorProgress { name, .. }
            | Self::PhaseProgress { name, .. }
            | Self::EventProgress { name, .. } => name,
        }
    }

    fn feature(&self) -> usize {
        match self {
            Self::Distance { feature, .. }
            | Self::CorridorProgress { feature, .. }
            | Self::PhaseProgress { feature, .. }
            | Self::EventProgress { feature, .. } => *feature,
        }
    }

    fn weight(&self) -> f32 {
        match self {
            Self::Distance { weight, .. }
            | Self::CorridorProgress { weight, .. }
            | Self::PhaseProgress { weight, .. }
            | Self::EventProgress { weight, .. } => *weight,
        }
    }

    fn unavailable_value(&self) -> Option<f32> {
        match self {
            Self::Distance {
                unavailable_value, ..
            }
            | Self::CorridorProgress {
                unavailable_value, ..
            }
            | Self::PhaseProgress {
                unavailable_value, ..
            }
            | Self::EventProgress {
                unavailable_value, ..
            } => *unavailable_value,
        }
    }

    fn kind(&self) -> PotentialKind {
        match self {
            Self::Distance { .. } => PotentialKind::Distance,
            Self::CorridorProgress { .. } => PotentialKind::CorridorProgress,
            Self::PhaseProgress { .. } => PotentialKind::PhaseProgress,
            Self::EventProgress { .. } => PotentialKind::EventProgress,
        }
    }

    fn potential(&self, value: f32) -> Result<f64, ShapingError> {
        if self
            .unavailable_value()
            .is_some_and(|missing| missing.to_bits() == value.to_bits())
        {
            return Err(ShapingError::UnavailableFact(self.name().into()));
        }
        let weight = f64::from(self.weight());
        let potential = match self {
            Self::Distance { goal, scale, .. } => {
                -weight * (f64::from(value) - f64::from(*goal)).abs() / f64::from(*scale)
            }
            Self::CorridorProgress { start, end, .. } => {
                let progress = ((f64::from(value) - f64::from(*start))
                    / (f64::from(*end) - f64::from(*start)))
                .clamp(0.0, 1.0);
                weight * progress
            }
            Self::PhaseProgress { ordered_values, .. }
            | Self::EventProgress { ordered_values, .. } => {
                let index = ordered_values
                    .iter()
                    .position(|candidate| candidate.to_bits() == value.to_bits())
                    .ok_or_else(|| ShapingError::UnrecognizedOrderedFact {
                        term: self.name().into(),
                        value,
                    })?;
                weight * index as f64 / (ordered_values.len() - 1) as f64
            }
        };
        if potential.is_finite() {
            Ok(potential)
        } else {
            Err(ShapingError::NonFiniteResult(self.name().into()))
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PotentialKind {
    Distance,
    CorridorProgress,
    PhaseProgress,
    EventProgress,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RewardComponent {
    pub name: String,
    pub kind: PotentialKind,
    pub feature: usize,
    pub source_fact: f32,
    pub next_fact: f32,
    pub source_potential: f64,
    pub next_potential: f64,
    /// Zero at an episodic terminal boundary; otherwise `next_potential`.
    pub effective_next_potential: f64,
    pub shaping_reward: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RewardBreakdown {
    pub base_reward: f32,
    pub duration_ticks: u32,
    pub per_tick_discount: f32,
    pub transition_discount: f64,
    pub terminal: bool,
    /// This is always true: shaping never supplies or changes a goal verdict.
    pub terminal_objective_unchanged: bool,
    /// Shaping may rank or train proposals, but exact simulation alone may
    /// authorize a winner.
    pub promotion_authority: bool,
    pub source_potential: f64,
    pub next_potential: f64,
    pub effective_next_potential: f64,
    pub shaping_reward: f64,
    pub training_reward: f32,
    pub components: Vec<RewardComponent>,
}

/// Campaign-level reward configuration. Every component is training-only:
/// native terminal evidence remains the sole success and promotion authority.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TacticRewardSpec {
    pub schema: String,
    pub terminal_reward: f32,
    pub tick_cost: f32,
    pub novelty_reward: f32,
    pub per_tick_discount: f32,
    #[serde(default)]
    pub potential: Option<PotentialShapingSpec>,
}

impl Default for TacticRewardSpec {
    fn default() -> Self {
        Self {
            schema: TACTIC_REWARD_SPEC_SCHEMA_V1.into(),
            terminal_reward: 1.0,
            tick_cost: 0.001,
            novelty_reward: 0.01,
            per_tick_discount: 0.995,
            potential: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TacticRewardBreakdown {
    pub terminal_observed: bool,
    pub endpoint_novel: bool,
    pub duration_ticks: u32,
    pub terminal_component: f32,
    pub tick_cost_component: f32,
    pub novelty_component: f32,
    pub base_reward: f32,
    pub potential: Option<RewardBreakdown>,
    pub training_reward: f32,
    pub terminal_objective_unchanged: bool,
    pub promotion_authority: bool,
}

impl TacticRewardSpec {
    pub fn evaluate(
        &self,
        feature_schema: Digest,
        state: &[f32],
        next_state: &[f32],
        duration_ticks: u32,
        terminal_observed: bool,
        endpoint_novel: bool,
    ) -> Result<TacticRewardBreakdown, ShapingError> {
        if self.schema != TACTIC_REWARD_SPEC_SCHEMA_V1 {
            return Err(ShapingError::InvalidSchema(self.schema.clone()));
        }
        validate_finite(self.terminal_reward, "terminal_reward")?;
        validate_finite(self.tick_cost, "tick_cost")?;
        validate_finite(self.novelty_reward, "novelty_reward")?;
        if self.tick_cost < 0.0
            || self.novelty_reward < 0.0
            || !self.per_tick_discount.is_finite()
            || !(0.0..=1.0).contains(&self.per_tick_discount)
            || self.per_tick_discount == 0.0
        {
            return Err(ShapingError::InvalidCampaignReward);
        }
        if duration_ticks == 0 {
            return Err(ShapingError::ZeroDuration);
        }
        if state.is_empty()
            || state.len() != next_state.len()
            || state
                .iter()
                .chain(next_state)
                .any(|value| !value.is_finite())
        {
            return Err(ShapingError::FeatureWidthMismatch {
                expected: state.len(),
                state: state.len(),
                next_state: next_state.len(),
            });
        }
        let terminal_component = if terminal_observed {
            self.terminal_reward
        } else {
            0.0
        };
        let tick_cost_component_f64 = -f64::from(self.tick_cost) * f64::from(duration_ticks);
        let tick_cost_component = tick_cost_component_f64 as f32;
        let novelty_component = if endpoint_novel {
            self.novelty_reward
        } else {
            0.0
        };
        let base_reward_f64 =
            f64::from(terminal_component) + tick_cost_component_f64 + f64::from(novelty_component);
        let base_reward = base_reward_f64 as f32;
        if !base_reward_f64.is_finite()
            || !base_reward.is_finite()
            || !tick_cost_component.is_finite()
        {
            return Err(ShapingError::NonFiniteResult("campaign_base_reward".into()));
        }
        let potential = self
            .potential
            .as_ref()
            .map(|potential| {
                if potential.feature_schema != feature_schema {
                    return Err(ShapingError::FeatureSchemaMismatch {
                        expected: potential.feature_schema,
                        actual: feature_schema,
                    });
                }
                potential.shape_reward(
                    state.len(),
                    state,
                    next_state,
                    base_reward,
                    duration_ticks,
                    terminal_observed,
                    self.per_tick_discount,
                )
            })
            .transpose()?;
        let training_reward = potential
            .as_ref()
            .map_or(base_reward, |breakdown| breakdown.training_reward);
        Ok(TacticRewardBreakdown {
            terminal_observed,
            endpoint_novel,
            duration_ticks,
            terminal_component,
            tick_cost_component,
            novelty_component,
            base_reward,
            potential,
            training_reward,
            terminal_objective_unchanged: true,
            promotion_authority: false,
        })
    }
}

impl PotentialShapingSpec {
    pub fn validate(&self, feature_count: usize) -> Result<(), ShapingError> {
        if self.schema != POTENTIAL_SHAPING_SCHEMA_V1 {
            return Err(ShapingError::InvalidSchema(self.schema.clone()));
        }
        if self.feature_schema == Digest::ZERO {
            return Err(ShapingError::MissingFeatureSchema);
        }
        if self.terms.is_empty() || self.terms.len() > MAX_TERMS {
            return Err(ShapingError::InvalidTermCount(self.terms.len()));
        }
        let mut names = HashSet::new();
        for term in &self.terms {
            let name = term.name();
            if name.is_empty()
                || name.len() > MAX_NAME_BYTES
                || !name.bytes().all(|byte| byte.is_ascii_graphic())
                || !names.insert(name)
            {
                return Err(ShapingError::InvalidTermName(name.into()));
            }
            if term.feature() >= feature_count {
                return Err(ShapingError::FeatureOutOfRange {
                    term: name.into(),
                    feature: term.feature(),
                    feature_count,
                });
            }
            validate_finite(term.weight(), name)?;
            if term.weight() < 0.0 {
                return Err(ShapingError::InvalidWeight(name.into()));
            }
            if let Some(unavailable) = term.unavailable_value() {
                validate_finite(unavailable, name)?;
            }
            match term {
                PotentialTerm::Distance { goal, scale, .. } => {
                    validate_finite(*goal, name)?;
                    validate_finite(*scale, name)?;
                    if *scale <= 0.0 {
                        return Err(ShapingError::InvalidScale(name.into()));
                    }
                }
                PotentialTerm::CorridorProgress { start, end, .. } => {
                    validate_finite(*start, name)?;
                    validate_finite(*end, name)?;
                    if start.to_bits() == end.to_bits() {
                        return Err(ShapingError::InvalidCorridor(name.into()));
                    }
                }
                PotentialTerm::PhaseProgress { ordered_values, .. }
                | PotentialTerm::EventProgress { ordered_values, .. } => {
                    if !(2..=MAX_ORDERED_VALUES).contains(&ordered_values.len()) {
                        return Err(ShapingError::InvalidOrderedValues(name.into()));
                    }
                    let mut values = HashSet::new();
                    for value in ordered_values {
                        validate_finite(*value, name)?;
                        if !values.insert(value.to_bits()) {
                            return Err(ShapingError::InvalidOrderedValues(name.into()));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    pub fn identity(&self, feature_count: usize) -> Result<Digest, ShapingError> {
        self.validate(feature_count)?;
        let bytes = serde_json::to_vec(self).map_err(ShapingError::Json)?;
        Ok(Digest(Sha256::digest(bytes).into()))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn shape_reward(
        &self,
        feature_count: usize,
        state: &[f32],
        next_state: &[f32],
        base_reward: f32,
        duration_ticks: u32,
        terminal: bool,
        per_tick_discount: f32,
    ) -> Result<RewardBreakdown, ShapingError> {
        self.validate(feature_count)?;
        if state.len() != feature_count || next_state.len() != feature_count {
            return Err(ShapingError::FeatureWidthMismatch {
                expected: feature_count,
                state: state.len(),
                next_state: next_state.len(),
            });
        }
        validate_finite(base_reward, "base_reward")?;
        if duration_ticks == 0 {
            return Err(ShapingError::ZeroDuration);
        }
        if !per_tick_discount.is_finite() || !(0.0..=1.0).contains(&per_tick_discount) {
            return Err(ShapingError::InvalidDiscount(per_tick_discount));
        }
        let transition_discount = discount_for_duration(per_tick_discount, duration_ticks);
        let mut components = Vec::with_capacity(self.terms.len());
        let mut source_potential = 0.0;
        let mut next_potential = 0.0;
        let mut shaping_reward = 0.0;
        for term in &self.terms {
            let source_fact = state[term.feature()];
            let next_fact = next_state[term.feature()];
            validate_finite(source_fact, term.name())?;
            validate_finite(next_fact, term.name())?;
            let source = term.potential(source_fact)?;
            let next = term.potential(next_fact)?;
            let effective_next = if terminal { 0.0 } else { next };
            let component_reward = transition_discount * effective_next - source;
            source_potential += source;
            next_potential += next;
            shaping_reward += component_reward;
            components.push(RewardComponent {
                name: term.name().into(),
                kind: term.kind(),
                feature: term.feature(),
                source_fact,
                next_fact,
                source_potential: source,
                next_potential: next,
                effective_next_potential: effective_next,
                shaping_reward: component_reward,
            });
        }
        let effective_next_potential = if terminal { 0.0 } else { next_potential };
        let training_reward_f64 = f64::from(base_reward) + shaping_reward;
        let training_reward = training_reward_f64 as f32;
        if !training_reward_f64.is_finite() || !training_reward.is_finite() {
            return Err(ShapingError::NonFiniteResult("training_reward".into()));
        }
        Ok(RewardBreakdown {
            base_reward,
            duration_ticks,
            per_tick_discount,
            transition_discount,
            terminal,
            terminal_objective_unchanged: true,
            promotion_authority: false,
            source_potential,
            next_potential,
            effective_next_potential,
            shaping_reward,
            training_reward,
            components,
        })
    }
}

fn validate_finite(value: f32, field: &str) -> Result<(), ShapingError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(ShapingError::NonFiniteValue(field.into()))
    }
}

fn discount_for_duration(discount: f32, mut duration: u32) -> f64 {
    let mut factor = f64::from(discount);
    let mut result = 1.0;
    while duration != 0 {
        if duration & 1 != 0 {
            result *= factor;
        }
        factor *= factor;
        duration >>= 1;
    }
    result
}

#[derive(Debug)]
pub enum ShapingError {
    Json(serde_json::Error),
    InvalidSchema(String),
    MissingFeatureSchema,
    InvalidTermCount(usize),
    InvalidTermName(String),
    FeatureOutOfRange {
        term: String,
        feature: usize,
        feature_count: usize,
    },
    InvalidWeight(String),
    InvalidScale(String),
    InvalidCorridor(String),
    InvalidOrderedValues(String),
    NonFiniteValue(String),
    UnavailableFact(String),
    UnrecognizedOrderedFact {
        term: String,
        value: f32,
    },
    FeatureWidthMismatch {
        expected: usize,
        state: usize,
        next_state: usize,
    },
    FeatureSchemaMismatch {
        expected: Digest,
        actual: Digest,
    },
    InvalidCampaignReward,
    ZeroDuration,
    InvalidDiscount(f32),
    NonFiniteResult(String),
}

impl fmt::Display for ShapingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(error) => write!(formatter, "shaping JSON error: {error}"),
            Self::InvalidSchema(schema) => write!(formatter, "unsupported shaping schema {schema}"),
            Self::MissingFeatureSchema => formatter.write_str("shaping feature schema is zero"),
            Self::InvalidTermCount(count) => {
                write!(
                    formatter,
                    "shaping term count {count} is outside 1..={MAX_TERMS}"
                )
            }
            Self::InvalidTermName(name) => {
                write!(formatter, "invalid or duplicate shaping term name {name:?}")
            }
            Self::FeatureOutOfRange {
                term,
                feature,
                feature_count,
            } => write!(
                formatter,
                "shaping term {term:?} feature {feature} is outside width {feature_count}"
            ),
            Self::InvalidWeight(term) => {
                write!(formatter, "shaping term {term:?} has a negative weight")
            }
            Self::InvalidScale(term) => write!(
                formatter,
                "distance term {term:?} requires a positive scale"
            ),
            Self::InvalidCorridor(term) => write!(
                formatter,
                "corridor term {term:?} has identical start and end"
            ),
            Self::InvalidOrderedValues(term) => write!(
                formatter,
                "ordered shaping term {term:?} requires 2..={MAX_ORDERED_VALUES} unique finite values"
            ),
            Self::NonFiniteValue(field) => {
                write!(formatter, "shaping value {field:?} is not finite")
            }
            Self::UnavailableFact(term) => write!(
                formatter,
                "shaping fact for {term:?} is explicitly unavailable"
            ),
            Self::UnrecognizedOrderedFact { term, value } => write!(
                formatter,
                "shaping fact {value} is not a declared value for {term:?}"
            ),
            Self::FeatureWidthMismatch {
                expected,
                state,
                next_state,
            } => write!(
                formatter,
                "shaping feature widths are {state}/{next_state}; expected {expected}"
            ),
            Self::FeatureSchemaMismatch { expected, actual } => write!(
                formatter,
                "shaping feature schema {actual} differs from configured schema {expected}"
            ),
            Self::InvalidCampaignReward => formatter
                .write_str("tactic tick cost and novelty reward must be finite and non-negative"),
            Self::ZeroDuration => {
                formatter.write_str("shaping transition duration must be nonzero")
            }
            Self::InvalidDiscount(discount) => write!(
                formatter,
                "shaping discount {discount} is not finite and within 0..=1"
            ),
            Self::NonFiniteResult(term) => {
                write!(formatter, "shaping result for {term:?} is not finite")
            }
        }
    }
}

impl Error for ShapingError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tactic_reward_composes_cost_novelty_terminal_and_potential_without_authority() {
        let feature_schema = Digest([9; 32]);
        let reward = TacticRewardSpec {
            schema: TACTIC_REWARD_SPEC_SCHEMA_V1.into(),
            terminal_reward: 10.0,
            tick_cost: 0.25,
            novelty_reward: 2.0,
            per_tick_discount: 0.5,
            potential: Some(PotentialShapingSpec {
                schema: POTENTIAL_SHAPING_SCHEMA_V1.into(),
                feature_schema,
                terms: vec![PotentialTerm::CorridorProgress {
                    name: "progress".into(),
                    feature: 0,
                    start: 0.0,
                    end: 4.0,
                    weight: 4.0,
                    unavailable_value: None,
                }],
            }),
        }
        .evaluate(feature_schema, &[0.0], &[4.0], 2, false, true)
        .unwrap();

        assert_eq!(reward.terminal_component, 0.0);
        assert_eq!(reward.tick_cost_component, -0.5);
        assert_eq!(reward.novelty_component, 2.0);
        assert_eq!(reward.base_reward, 1.5);
        assert_eq!(reward.training_reward, 2.5);
        assert!(reward.potential.is_some());
        assert!(reward.terminal_objective_unchanged);
        assert!(!reward.promotion_authority);

        let terminal = TacticRewardSpec::default()
            .evaluate(feature_schema, &[0.0], &[1.0], 1, true, false)
            .unwrap();
        assert_eq!(terminal.terminal_component, 1.0);
        assert!(!terminal.endpoint_novel);
        assert!(terminal.terminal_objective_unchanged);
        assert!(!terminal.promotion_authority);

        assert!(matches!(
            TacticRewardSpec {
                per_tick_discount: 0.0,
                ..TacticRewardSpec::default()
            }
            .evaluate(feature_schema, &[0.0], &[1.0], 1, false, false)
            .unwrap_err(),
            ShapingError::InvalidCampaignReward
        ));
    }

    fn spec() -> PotentialShapingSpec {
        PotentialShapingSpec {
            schema: POTENTIAL_SHAPING_SCHEMA_V1.into(),
            feature_schema: Digest([7; 32]),
            terms: vec![
                PotentialTerm::Distance {
                    name: "goal-distance".into(),
                    feature: 0,
                    goal: 0.0,
                    scale: 10.0,
                    weight: 2.0,
                    unavailable_value: Some(-1.0),
                },
                PotentialTerm::CorridorProgress {
                    name: "hallway".into(),
                    feature: 1,
                    start: 0.0,
                    end: 100.0,
                    weight: 3.0,
                    unavailable_value: None,
                },
                PotentialTerm::PhaseProgress {
                    name: "transition-phase".into(),
                    feature: 2,
                    ordered_values: vec![10.0, 20.0, 30.0],
                    weight: 4.0,
                    unavailable_value: None,
                },
                PotentialTerm::EventProgress {
                    name: "event-step".into(),
                    feature: 3,
                    ordered_values: vec![0.0, 1.0, 2.0],
                    weight: 5.0,
                    unavailable_value: None,
                },
            ],
        }
    }

    #[test]
    fn every_supported_fact_produces_an_inspectable_potential_component() {
        let shaped = spec()
            .shape_reward(
                4,
                &[10.0, 0.0, 10.0, 0.0],
                &[5.0, 50.0, 20.0, 1.0],
                -1.0,
                1,
                false,
                1.0,
            )
            .unwrap();
        assert_eq!(shaped.components.len(), 4);
        assert_eq!(shaped.components[0].kind, PotentialKind::Distance);
        assert_eq!(shaped.components[1].kind, PotentialKind::CorridorProgress);
        assert_eq!(shaped.components[2].kind, PotentialKind::PhaseProgress);
        assert_eq!(shaped.components[3].kind, PotentialKind::EventProgress);
        assert_eq!(shaped.source_potential, -2.0);
        assert_eq!(shaped.next_potential, 5.0);
        assert_eq!(shaped.shaping_reward, 7.0);
        assert_eq!(shaped.training_reward, 6.0);
        assert!(shaped.terminal_objective_unchanged);
        assert!(!shaped.promotion_authority);
        assert_eq!(shaped.components[0].source_fact, 10.0);
        assert_eq!(shaped.components[0].next_fact, 5.0);
    }

    #[test]
    fn episodic_terminal_zeroing_makes_discounted_shaping_telescope() {
        let spec = PotentialShapingSpec {
            terms: vec![PotentialTerm::CorridorProgress {
                name: "route".into(),
                feature: 0,
                start: 0.0,
                end: 10.0,
                weight: 10.0,
                unavailable_value: None,
            }],
            ..spec()
        };
        let first = spec
            .shape_reward(1, &[2.0], &[7.0], 0.0, 1, false, 0.9)
            .unwrap();
        let terminal = spec
            .shape_reward(1, &[7.0], &[10.0], 0.0, 1, true, 0.9)
            .unwrap();
        let discounted_shaping = first.shaping_reward + 0.9 * terminal.shaping_reward;
        assert!((discounted_shaping + first.source_potential).abs() < 1.0e-6);
        assert_eq!(terminal.effective_next_potential, 0.0);
        assert_eq!(terminal.components[0].effective_next_potential, 0.0);
    }

    #[test]
    fn unavailable_and_undeclared_facts_are_rejected_without_guessing() {
        assert!(matches!(
            spec().shape_reward(
                4,
                &[-1.0, 0.0, 10.0, 0.0],
                &[0.0, 0.0, 10.0, 0.0],
                0.0,
                1,
                false,
                0.9,
            ),
            Err(ShapingError::UnavailableFact(_))
        ));
        assert!(matches!(
            spec().shape_reward(
                4,
                &[1.0, 0.0, 11.0, 0.0],
                &[0.0, 0.0, 20.0, 0.0],
                0.0,
                1,
                false,
                0.9,
            ),
            Err(ShapingError::UnrecognizedOrderedFact { .. })
        ));
    }

    #[test]
    fn shaping_identity_covers_feature_schema_and_term_meaning() {
        let original = spec().identity(4).unwrap();
        let mut changed = spec();
        let PotentialTerm::Distance { goal, .. } = &mut changed.terms[0] else {
            unreachable!()
        };
        *goal = 1.0;
        assert_ne!(original, changed.identity(4).unwrap());
        changed = spec();
        changed.feature_schema = Digest([8; 32]);
        assert_ne!(original, changed.identity(4).unwrap());
    }
}
