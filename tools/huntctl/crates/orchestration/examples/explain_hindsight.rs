//! Read-only, offline inspection of the hindsight critic on recorded experience.
//! Usage: explain_hindsight CHECKPOINT.dtqz GOAL_X GOAL_Y GOAL_Z [ROW_INDEX]
//!        [--bellman | --hindsight-bellman]
//! Fits the complete checkpoint corpus, not the historical online snapshot.
use dusklight_learning::generalized_tactic_value::{
    GeneralizedTacticContext, GeneralizedTacticValueModel,
};
use dusklight_learning::tactic_features::GoalConditionedTacticFeatureEncoder;
use dusklight_learning::tactic_value_treatment::ContinuousTacticValueModel;
use dusklight_orchestration::tactic_q_campaign::TacticQCampaign;
use serde_json::json;
use std::error::Error;
use std::path::Path;

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1).collect::<Vec<_>>();
    let hindsight_bellman = args.last().is_some_and(|arg| arg == "--hindsight-bellman");
    let bellman = hindsight_bellman || args.last().is_some_and(|arg| arg == "--bellman");
    if bellman {
        args.pop();
    }
    if !(4..=5).contains(&args.len()) {
        return Err(
            "usage: explain_hindsight CHECKPOINT GOAL_X GOAL_Y GOAL_Z [ROW_INDEX] [--bellman | --hindsight-bellman]"
                .into(),
        );
    }
    let checkpoint = TacticQCampaign::read_checkpoint_payload(Path::new(&args[0]))?;
    let rows = &checkpoint.training_replay;
    let index = args
        .get(4)
        .map(|s| s.parse::<usize>())
        .transpose()?
        .unwrap_or(0);
    let query = rows
        .get(index)
        .ok_or("query row is absent from training replay")?;
    let encoder = GoalConditionedTacticFeatureEncoder::new([
        args[1].parse()?,
        args[2].parse()?,
        args[3].parse()?,
    ])?;
    let features = encoder.encode(&query.before)?;
    let context = GeneralizedTacticContext::from_facts(&query.before)?;
    // Recorded descriptors are diagnostic candidates, not an assertion that
    // every action is currently applicable. No controller is executed here.
    let mut actions = rows
        .iter()
        .map(|row| row.value_sample.action.clone())
        .collect::<Vec<_>>();
    actions.sort_by(|a, b| a.option_id.cmp(&b.option_id));
    actions.dedup();
    if bellman {
        let started = std::time::Instant::now();
        // A bounded offline diagnostic, not a historical snapshot replay or
        // a campaign configuration. Four actual Bellman updates on all rows.
        let fit = if hindsight_bellman {
            ContinuousTacticValueModel::fit_hindsight_bellman
        } else {
            ContinuousTacticValueModel::fit_bellman
        };
        let model = fit(
            rows,
            encoder.goal_distance_feature(),
            4,
            checkpoint.model_config.fitted_q.discount,
        )?;
        let fit_millis = started.elapsed().as_millis();
        let ranked = model.rank(&features, &context, &actions)?;
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "checkpoint": checkpoint.content_sha256,
                "treatment": if hindsight_bellman { "hindsight_bellman_forest_v3" } else { "continuous_bellman_forest_v2" },
                "goal_query_kind": model.goal_query_kind(),
                "replay_stats": model.bellman_replay_stats(),
                "training_rows": rows.len(),
                "terminal_rows": rows.iter().filter(|row| row.value_sample.terminal).count(),
                "bellman_iterations": 4,
                "fit_millis": fit_millis,
                "query_row": index,
                "recorded_action_candidates": actions.len(),
                "top_predictions": ranked.iter().take(5).map(|estimate| json!({
                    "action": estimate.descriptor.option_id,
                    "mean_q": estimate.mean_q,
                    "variance": estimate.ensemble_variance,
                })).collect::<Vec<_>>(),
            }))?
        );
        return Ok(());
    }
    let model = GeneralizedTacticValueModel::fit_delayed_achieved_goal_returns(
        rows,
        encoder.goal_distance_feature(),
    )?;
    let ranked = model.rank(&features, &context, &actions)?;
    let predictions = ranked
        .iter()
        .take(5)
        .map(|estimate| {
            let neighbors = model.explain_action(&features, &context, &estimate.descriptor)?;
            Ok(json!({
                "action": estimate.descriptor.option_id,
                "predicted_return": estimate.outcome.reward,
                "nearest_distance": estimate.nearest_distance,
                "neighbors": neighbors.iter().map(|neighbor| json!({
                    "sample": neighbor.training_sample_index,
                    "state_distance": neighbor.state_distance,
                    "action_distance": neighbor.action_distance,
                    "weight": neighbor.normalized_weight,
                    "return": neighbor.outcome.reward,
                    "goal_distance": neighbor.state_features[encoder.goal_distance_feature()],
                    "native_goal": neighbor.state_features.last() == Some(&0.0),
                })).collect::<Vec<_>>(),
            }))
        })
        .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "checkpoint": checkpoint.content_sha256,
            "training_rows": rows.len(),
            "query_row": index,
            "query_goal_distance": features[encoder.goal_distance_feature()],
            "recorded_action_candidates": actions.len(),
            "top_predictions": predictions,
        }))?
    );
    Ok(())
}
