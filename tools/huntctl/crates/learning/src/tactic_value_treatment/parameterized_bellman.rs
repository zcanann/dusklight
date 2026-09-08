use super::hindsight_goals::{self, BellmanGoalKind, BellmanReplayStats};
use super::*;
use crate::fact_registry::FactRegistry;
use crate::fact_snapshot::FRONT_ROLL_DO_STATUS;
use crate::fqi::ParameterizedTransition;
use crate::generalized_tactic_value::prediction::{
    regression_action_features, regression_state_features,
};
use crate::learner_state::LearnerState;
use crate::parameterized_tactic_proposals::{
    MAX_PARAMETERIZED_TACTIC_TICKS, ParameterizedTacticProposalContext,
    propose_parameterized_tactics,
};
use crate::tactic_features::GoalConditionedTacticFeatureEncoder;

impl ContinuousTacticValueModel {
    pub fn fit_bellman(
        rows: &[OptionTransitionSample],
        goal_distance: usize,
        iterations: usize,
        discount: f32,
    ) -> Result<Self, GeneralizedTacticValueError> {
        fit(rows, goal_distance, iterations, discount, false)
    }

    /// One time-cost Bellman learner over native and auxiliary goal tasks.
    /// Raw rewards/facts are not rewritten. Every task pays realized native
    /// ticks; only reaching that task's goal stops its continuation.
    pub fn fit_hindsight_bellman(
        rows: &[OptionTransitionSample],
        goal_distance: usize,
        iterations: usize,
        discount: f32,
    ) -> Result<Self, GeneralizedTacticValueError> {
        fit(rows, goal_distance, iterations, discount, true)
    }
}

fn fit(
    rows: &[OptionTransitionSample],
    goal_distance: usize,
    iterations: usize,
    discount: f32,
    hindsight: bool,
) -> Result<ContinuousTacticValueModel, GeneralizedTacticValueError> {
    let encoder = GoalConditionedTacticFeatureEncoder::new([0.0; 3]).map_err(invalid)?;
    if rows.len() < 2 || rows.len() > crate::fqi::MAX_FQI_TRANSITIONS {
        return Err(GeneralizedTacticValueError::SampleCount);
    }
    if iterations == 0
        || iterations > crate::fqi::MAX_FQI_ITERATIONS
        || !discount.is_finite()
        || !(0.0..=1.0).contains(&discount)
    {
        return Err(GeneralizedTacticValueError::InvalidConfig);
    }
    if goal_distance != encoder.goal_distance_feature()
        || rows.iter().any(|row| {
            row.feature_schema_sha256 != encoder.schema_sha256
                || row.value_sample.state.len() != encoder.feature_width()
        })
    {
        return Err(GeneralizedTacticValueError::FeatureWidth);
    }
    let mut stats = BellmanReplayStats::default();
    let registry = FactRegistry::canonical();
    let mut samples = Vec::new();
    for row in rows {
        row.validate().map_err(invalid)?;
        let state = &row.value_sample.state;
        let next = &row.value_sample.next_state;
        let (state, next, reward) = if hindsight {
            (
                hindsight_goals::query_features(state, BellmanGoalKind::Authored)?,
                hindsight_goals::query_features(next, BellmanGoalKind::Authored)?,
                -(row.value_sample.duration_ticks as f32),
            )
        } else {
            (state.clone(), next.clone(), row.value_sample.reward)
        };
        samples.push(sample(
            row,
            state,
            next,
            reward,
            row.value_sample.terminal,
            goal_distance,
            &registry,
            &mut stats,
        )?);
        stats.native_rows += 1;
        stats.native_terminals += usize::from(row.value_sample.terminal);
    }
    if hindsight {
        for goal in hindsight_goals::sampled_goals(rows)? {
            stats.auxiliary_goals += 1;
            for row in rows {
                if goal.reached(&row.before) {
                    stats.already_satisfied_omissions += 1;
                    continue;
                }
                let terminal = goal.reached(&row.after);
                samples.push(sample(
                    row,
                    goal.features(&row.before, &row.value_sample.state)?,
                    goal.features(&row.after, &row.value_sample.next_state)?,
                    -(row.value_sample.duration_ticks as f32),
                    terminal,
                    goal_distance,
                    &registry,
                    &mut stats,
                )?);
                stats.auxiliary_rows += 1;
                stats.auxiliary_terminals += usize::from(terminal);
            }
        }
    }
    let mut config = FqiConfig {
        iterations,
        discount,
        ..continuous_forest_config()
    };
    if hindsight {
        config
            .categorical_features
            .extend(encoder.feature_width()..encoder.feature_width() + 5);
    }
    let forest = FittedQ::fit_parameterized(samples[0].state_action.len(), &samples, &config)
        .map_err(invalid)?;
    let hindsight_query = hindsight.then_some(
        if stats.native_terminals == 0 && stats.auxiliary_terminals > 0 {
            BellmanGoalKind::Coordinate
        } else {
            BellmanGoalKind::Authored
        },
    );
    Ok(ContinuousTacticValueModel {
        forest,
        hindsight_query,
        bellman_stats: Some(stats),
    })
}

#[allow(clippy::too_many_arguments)]
fn sample(
    row: &OptionTransitionSample,
    state: Vec<f32>,
    next: Vec<f32>,
    reward: f32,
    terminal: bool,
    goal_distance: usize,
    registry: &FactRegistry,
    stats: &mut BellmanReplayStats,
) -> Result<ParameterizedTransition, GeneralizedTacticValueError> {
    let state_action = regression_features(
        &state,
        &GeneralizedTacticContext::from_facts(&row.before)?,
        &row.value_sample.action,
    )?;
    let mut actions = Vec::new();
    let mut successor_state = Vec::new();
    if !terminal {
        let facts = &row.after;
        let position = facts.player.position_f32_bits.map(f32::from_bits);
        let goal = std::array::from_fn(|i| position[i] + next[goal_distance - 3 + i]);
        let proposals = propose_parameterized_tactics(ParameterizedTacticProposalContext {
            seed: CONTINUOUS_FOREST_SEED,
            decision_index: 0,
            state_sha256: row.after_state_sha256,
            player_position: position,
            camera_yaw_radians: facts.player.camera_yaw_radians_f32_bits.map(f32::from_bits),
            prompted_action_available: facts
                .player
                .action_state
                .is_some_and(|action| action.do_status != 0),
            front_roll_prompt_available: facts
                .player
                .action_state
                .is_some_and(|action| action.do_status == FRONT_ROLL_DO_STATUS),
            goal_coordinate: goal,
            maximum_ticks: MAX_PARAMETERIZED_TACTIC_TICKS,
            feedback: None,
        })
        .map_err(invalid)?;
        let mut availability_facts = facts.clone();
        if row.value_sample.terminal {
            // The source run stopped for its own goal, not this auxiliary
            // task. This virtual mask must not stop bootstrapping prematurely.
            availability_facts.terminal = crate::fact_snapshot::TerminalFactSnapshot {
                reason: crate::fact_snapshot::FactTerminalReason::None,
                configured: Some(true),
                reached: Some(false),
                requested_count: Some(1),
                hit_count: Some(0),
                stable_ticks: Some(1),
                consecutive_ticks: Some(0),
                first_hit_tick: None,
            };
        }
        let available = LearnerState::build(
            availability_facts,
            registry,
            &proposals.catalog,
            &proposals.blueprints,
            |_| true,
        )
        .map_err(invalid)?;
        let context = GeneralizedTacticContext::from_facts(facts)?;
        successor_state = regression_state_features(&next, &context)?;
        stats.successor_feature_values += successor_state.len();
        for entry in available
            .action_mask
            .iter()
            .filter(|entry| entry.applicable)
        {
            let action = regression_action_features(&context, &entry.descriptor)?;
            stats.successor_feature_values += action.len();
            if stats.successor_feature_values > 64_000_000 {
                return Err(invalid("Bellman successor feature cache exceeds 256 MB"));
            }
            actions.push(action);
        }
    }
    Ok(ParameterizedTransition {
        state_action,
        reward,
        duration: row.value_sample.duration_ticks,
        terminal,
        successor_state,
        successor_actions: actions,
    })
}

fn invalid(error: impl std::fmt::Display) -> GeneralizedTacticValueError {
    GeneralizedTacticValueError::InvalidTransition(error.to_string())
}
