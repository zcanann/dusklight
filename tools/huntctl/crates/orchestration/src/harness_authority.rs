//! Shared validation for sealed execution authority composed with anchored goals.

use dusklight_automation_contracts::candidate_envelope::NamedDigest;
use dusklight_evaluation::{AnchoredObjectiveIdentity, EvaluateError, HarnessEvaluateConfig};
use dusklight_learning::offline_rl::movement_action_schema_digest_v2;

pub(crate) fn validate_anchored_harness_request(
    harness: Option<&HarnessEvaluateConfig>,
    objective: &AnchoredObjectiveIdentity,
    operation: &str,
) -> Result<(), EvaluateError> {
    let Some(harness) = harness else {
        return Ok(());
    };
    let request = &harness.request_template;
    let request_objective = NamedDigest::new(
        request.objective.goal.clone(),
        request.objective.program_sha256,
    );
    let request_action_schema = NamedDigest::new(
        request.action_schema.id.clone(),
        request.action_schema.sha256,
    );
    if !anchored_identity_parts_match(
        &request_objective,
        &request_action_schema,
        &objective.goal_milestone,
        &objective.milestone_program_sha256,
    ) {
        return Err(EvaluateError::InvalidConfig(format!(
            "{operation} run request must bind the exact goal, milestone program, and movement action schema"
        )));
    }
    Ok(())
}

fn anchored_identity_parts_match(
    request_objective: &NamedDigest,
    request_action_schema: &NamedDigest,
    anchored_goal: &str,
    anchored_program_sha256: &str,
) -> bool {
    request_objective.id == anchored_goal
        && request_objective.sha256.to_string() == anchored_program_sha256
        && request_action_schema.id == "movement-action/v2"
        && request_action_schema.sha256 == movement_action_schema_digest_v2()
}

#[cfg(test)]
mod tests {
    use super::*;
    use dusklight_automation_contracts::artifact::Digest;

    #[test]
    fn anchored_authority_rejects_goal_program_and_action_drift_independently() {
        let program = Digest([1; 32]);
        let request_objective = NamedDigest::new("goal", program);
        let action = NamedDigest::new("movement-action/v2", movement_action_schema_digest_v2());
        assert!(anchored_identity_parts_match(
            &request_objective,
            &action,
            "goal",
            &program.to_string(),
        ));
        assert!(!anchored_identity_parts_match(
            &request_objective,
            &action,
            "other-goal",
            &program.to_string(),
        ));
        assert!(!anchored_identity_parts_match(
            &request_objective,
            &action,
            "goal",
            &Digest([2; 32]).to_string(),
        ));
        assert!(!anchored_identity_parts_match(
            &request_objective,
            &NamedDigest::new("movement-action/v2", Digest([3; 32])),
            "goal",
            &program.to_string(),
        ));
    }
}
