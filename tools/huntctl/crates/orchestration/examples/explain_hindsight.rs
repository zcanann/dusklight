//! Read-only, offline inspection of the hindsight critic on recorded experience.
//! Usage: explain_hindsight CHECKPOINT.dtqz GOAL_X GOAL_Y GOAL_Z [ROW_INDEX]
//! Fits the complete checkpoint corpus, not the historical online snapshot.
use dusklight_learning::generalized_tactic_value::{
    GeneralizedTacticContext, GeneralizedTacticValueModel,
};
use dusklight_learning::tactic_features::GoalConditionedTacticFeatureEncoder;
use dusklight_orchestration::tactic_q_campaign::TacticQCampaign;
use serde_json::json;
use std::error::Error;
use std::path::Path;

fn main() -> Result<(), Box<dyn Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if !(4..=5).contains(&args.len()) {
        return Err("usage: explain_hindsight CHECKPOINT GOAL_X GOAL_Y GOAL_Z [ROW_INDEX]".into());
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
    let model = GeneralizedTacticValueModel::fit_delayed_achieved_goal_returns(
        rows,
        encoder.goal_distance_feature(),
    )?;
    // Recorded descriptors are diagnostic candidates, not an assertion that
    // every action is currently applicable. No controller is executed here.
    let mut actions = rows
        .iter()
        .map(|row| row.value_sample.action.clone())
        .collect::<Vec<_>>();
    actions.sort_by(|a, b| a.option_id.cmp(&b.option_id));
    actions.dedup();
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
