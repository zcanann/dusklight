use super::*;
use crate::fact_registry::FactRegistry;
use crate::fact_snapshot::FRONT_ROLL_DO_STATUS;
use crate::fqi::ParameterizedTransition;
use crate::learner_state::LearnerState;
use crate::parameterized_tactic_proposals::{
    MAX_PARAMETERIZED_TACTIC_TICKS, ParameterizedTacticProposalContext,
    propose_parameterized_tactics,
};
use crate::tactic_features::GoalConditionedTacticFeatureEncoder;

impl ContinuousTacticValueModel {
    /// Unlike the conditional-cost control, train every observed transition.
    /// A rollout cutoff remains nonterminal and receives a successor-value
    /// bootstrap. Native terminal evidence alone suppresses that bootstrap.
    pub fn fit_bellman(
        transitions: &[OptionTransitionSample],
        goal_distance_feature: usize,
        iterations: usize,
        discount: f32,
    ) -> Result<Self, GeneralizedTacticValueError> {
        let encoder = GoalConditionedTacticFeatureEncoder::new([0.0; 3]).map_err(invalid)?;
        if transitions.len() < 2 || transitions.len() > crate::fqi::MAX_FQI_TRANSITIONS {
            return Err(GeneralizedTacticValueError::SampleCount);
        }
        if iterations == 0
            || iterations > crate::fqi::MAX_FQI_ITERATIONS
            || !discount.is_finite()
            || !(0.0..=1.0).contains(&discount)
        {
            return Err(GeneralizedTacticValueError::InvalidConfig);
        }
        if goal_distance_feature != encoder.goal_distance_feature()
            || transitions.iter().any(|row| {
                row.feature_schema_sha256 != encoder.schema_sha256
                    || row.value_sample.state.len() != encoder.feature_width()
            })
        {
            return Err(GeneralizedTacticValueError::FeatureWidth);
        }
        let registry = FactRegistry::canonical();
        let mut samples = Vec::with_capacity(transitions.len());
        let mut feature_values = 0usize;
        for row in transitions {
            row.validate().map_err(invalid)?;
            let state_action = regression_features(
                &row.value_sample.state,
                &GeneralizedTacticContext::from_facts(&row.before)?,
                &row.value_sample.action,
            )?;
            let mut successors = Vec::new();
            if !row.value_sample.terminal {
                let facts = &row.after;
                let position = facts.player.position_f32_bits.map(f32::from_bits);
                let goal = std::array::from_fn(|i| {
                    position[i] + row.value_sample.next_state[goal_distance_feature - 3 + i]
                });
                let proposals = propose_parameterized_tactics(ParameterizedTacticProposalContext {
                    seed: CONTINUOUS_FOREST_SEED,
                    decision_index: 0,
                    state_sha256: row.after_state_sha256,
                    player_position: position,
                    camera_yaw_radians: facts
                        .player
                        .camera_yaw_radians_f32_bits
                        .map(f32::from_bits),
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
                let state = LearnerState::build(
                    facts.clone(),
                    &registry,
                    &proposals.catalog,
                    &proposals.blueprints,
                    |_| true,
                )
                .map_err(invalid)?;
                let context = GeneralizedTacticContext::from_facts(facts)?;
                for entry in state.action_mask.iter().filter(|entry| entry.applicable) {
                    let features = regression_features(
                        &row.value_sample.next_state,
                        &context,
                        &entry.descriptor,
                    )?;
                    feature_values = feature_values.saturating_add(features.len());
                    if feature_values > 64_000_000 {
                        return Err(invalid("Bellman successor feature cache exceeds 256 MB"));
                    }
                    successors.push(features);
                }
            }
            samples.push(ParameterizedTransition {
                state_action,
                reward: row.value_sample.reward,
                duration: row.value_sample.duration_ticks,
                terminal: row.value_sample.terminal,
                successor_state_actions: successors,
            });
        }
        let config = FqiConfig {
            iterations,
            discount,
            ..continuous_forest_config()
        };
        let forest = FittedQ::fit_parameterized(samples[0].state_action.len(), &samples, &config)
            .map_err(invalid)?;
        Ok(Self { forest })
    }
}

fn invalid(error: impl std::fmt::Display) -> GeneralizedTacticValueError {
    GeneralizedTacticValueError::InvalidTransition(error.to_string())
}
