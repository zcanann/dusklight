//! Identical-input control/treatment plans and retained intervention evidence.

use crate::artifact::{BuildIdentity, Digest};
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fmt;

pub const INTERVENTION_EXPERIMENT_PLAN_SCHEMA: &str = "dusklight-intervention-experiment-plan/v1";
pub const INTERVENTION_EXPERIMENT_PAIR_SCHEMA: &str = "dusklight-intervention-experiment-pair/v1";
const MAX_EXPERIMENT_ID_BYTES: usize = 128;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InterventionExperimentInputs {
    pub build: BuildIdentity,
    pub scenario_id: String,
    pub scenario_sha256: Digest,
    pub parent_boundary_sha256: Digest,
    pub absolute_input_tape_sha256: Digest,
    pub observation_schema_sha256: Digest,
    pub oracle_program_sha256: Digest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InterventionExperimentPlan {
    pub schema: String,
    pub experiment_id: String,
    pub inputs: InterventionExperimentInputs,
    pub intervention_sha256: Digest,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InterventionExperimentRole {
    Control,
    Treatment,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InterventionExperimentRunRequest {
    pub role: InterventionExperimentRole,
    pub inputs: InterventionExperimentInputs,
    pub gameplay_writes_enabled: bool,
    pub intervention_sha256: Option<Digest>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InterventionExperimentRunArtifact {
    pub role: InterventionExperimentRole,
    pub inputs: InterventionExperimentInputs,
    pub gameplay_writes_enabled: bool,
    pub intervention_sha256: Option<Digest>,
    pub retained_run_sha256: Digest,
    pub retained_trace_sha256: Digest,
    pub retained_oracle_report_sha256: Digest,
    pub retained_intervention_audit_sha256: Option<Digest>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InterventionExperimentPair {
    pub schema: String,
    pub plan: InterventionExperimentPlan,
    pub control: InterventionExperimentRunArtifact,
    pub treatment: InterventionExperimentRunArtifact,
}

#[derive(Debug)]
pub struct InterventionExperimentError(String);

impl fmt::Display for InterventionExperimentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for InterventionExperimentError {}

impl InterventionExperimentPlan {
    pub fn validate(&self) -> Result<(), InterventionExperimentError> {
        if self.schema != INTERVENTION_EXPERIMENT_PLAN_SCHEMA {
            return Err(experiment_error(
                "unknown intervention experiment plan schema",
            ));
        }
        if self.experiment_id.is_empty()
            || self.experiment_id.len() > MAX_EXPERIMENT_ID_BYTES
            || !self
                .experiment_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err(experiment_error("intervention experiment ID is invalid"));
        }
        self.inputs.validate()?;
        require_digest(self.intervention_sha256, "intervention artifact")
    }
}

impl InterventionExperimentInputs {
    fn validate(&self) -> Result<(), InterventionExperimentError> {
        if self.scenario_id.is_empty() || self.scenario_id.len() > MAX_EXPERIMENT_ID_BYTES {
            return Err(experiment_error("intervention scenario ID is invalid"));
        }
        for (digest, label) in [
            (self.scenario_sha256, "scenario"),
            (self.parent_boundary_sha256, "parent boundary"),
            (self.absolute_input_tape_sha256, "absolute input tape"),
            (self.observation_schema_sha256, "observation schema"),
            (self.oracle_program_sha256, "oracle program"),
            (self.build.feature_digest, "build feature set"),
            (self.build.game_digest, "game image"),
        ] {
            require_digest(digest, label)?;
        }
        if self.build.dusklight_commit.is_empty()
            || self.build.aurora_commit.is_empty()
            || self.build.compiler.is_empty()
            || self.build.target.is_empty()
            || self.build.profile.is_empty()
            || self.build.fidelity_profile.is_empty()
        {
            return Err(experiment_error(
                "intervention build identity is incomplete",
            ));
        }
        Ok(())
    }
}

impl InterventionExperimentPair {
    pub fn validate(&self) -> Result<(), InterventionExperimentError> {
        if self.schema != INTERVENTION_EXPERIMENT_PAIR_SCHEMA {
            return Err(experiment_error(
                "unknown intervention experiment pair schema",
            ));
        }
        self.plan.validate()?;
        self.validate_run(&self.control, InterventionExperimentRole::Control)?;
        self.validate_run(&self.treatment, InterventionExperimentRole::Treatment)?;

        if self.control.gameplay_writes_enabled
            || self.control.intervention_sha256.is_some()
            || self.control.retained_intervention_audit_sha256.is_some()
        {
            return Err(experiment_error(
                "control must run with gameplay writes and interventions disabled",
            ));
        }
        if !self.treatment.gameplay_writes_enabled
            || self.treatment.intervention_sha256 != Some(self.plan.intervention_sha256)
        {
            return Err(experiment_error(
                "treatment must enable exactly the planned intervention",
            ));
        }
        require_digest(
            self.treatment
                .retained_intervention_audit_sha256
                .ok_or_else(|| experiment_error("treatment is missing its retained write audit"))?,
            "treatment intervention audit",
        )?;
        Ok(())
    }

    fn validate_run(
        &self,
        run: &InterventionExperimentRunArtifact,
        expected_role: InterventionExperimentRole,
    ) -> Result<(), InterventionExperimentError> {
        if run.role != expected_role {
            return Err(experiment_error("intervention run role is misplaced"));
        }
        run.inputs.validate()?;
        if run.inputs != self.plan.inputs {
            return Err(experiment_error(
                "control and treatment must use the plan's identical inputs",
            ));
        }
        for (digest, label) in [
            (run.retained_run_sha256, "retained run"),
            (run.retained_trace_sha256, "retained trace"),
            (run.retained_oracle_report_sha256, "retained oracle report"),
        ] {
            require_digest(digest, label)?;
        }
        Ok(())
    }
}

/// Runs the write-disabled control first and the exactly planned treatment
/// second, then validates that both retained artifact sets match the plan.
pub fn execute_intervention_experiment<E>(
    plan: InterventionExperimentPlan,
    mut executor: E,
) -> Result<InterventionExperimentPair, InterventionExperimentError>
where
    E: FnMut(
        &InterventionExperimentRunRequest,
    ) -> Result<InterventionExperimentRunArtifact, InterventionExperimentError>,
{
    plan.validate()?;
    let control_request = InterventionExperimentRunRequest {
        role: InterventionExperimentRole::Control,
        inputs: plan.inputs.clone(),
        gameplay_writes_enabled: false,
        intervention_sha256: None,
    };
    let control = executor(&control_request)?;
    let treatment_request = InterventionExperimentRunRequest {
        role: InterventionExperimentRole::Treatment,
        inputs: plan.inputs.clone(),
        gameplay_writes_enabled: true,
        intervention_sha256: Some(plan.intervention_sha256),
    };
    let treatment = executor(&treatment_request)?;
    let pair = InterventionExperimentPair {
        schema: INTERVENTION_EXPERIMENT_PAIR_SCHEMA.into(),
        plan,
        control,
        treatment,
    };
    pair.validate()?;
    Ok(pair)
}

fn require_digest(digest: Digest, label: &str) -> Result<(), InterventionExperimentError> {
    if digest == Digest::ZERO {
        return Err(experiment_error(format!("{label} digest is unavailable")));
    }
    Ok(())
}

fn experiment_error(message: impl Into<String>) -> InterventionExperimentError {
    InterventionExperimentError(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build() -> BuildIdentity {
        BuildIdentity {
            dusklight_commit: "dusklight-commit".into(),
            aurora_commit: "aurora-commit".into(),
            compiler: "clang".into(),
            target: "aarch64-apple-darwin".into(),
            profile: "debug".into(),
            feature_digest: Digest([1; 32]),
            game_digest: Digest([2; 32]),
            dirty_digest: None,
            fidelity_profile: "experimental_typed_gameplay_writes".into(),
        }
    }

    fn inputs() -> InterventionExperimentInputs {
        InterventionExperimentInputs {
            build: build(),
            scenario_id: "fence-push".into(),
            scenario_sha256: Digest([3; 32]),
            parent_boundary_sha256: Digest([4; 32]),
            absolute_input_tape_sha256: Digest([5; 32]),
            observation_schema_sha256: Digest([6; 32]),
            oracle_program_sha256: Digest([7; 32]),
        }
    }

    fn pair() -> InterventionExperimentPair {
        let inputs = inputs();
        InterventionExperimentPair {
            schema: INTERVENTION_EXPERIMENT_PAIR_SCHEMA.into(),
            plan: InterventionExperimentPlan {
                schema: INTERVENTION_EXPERIMENT_PLAN_SCHEMA.into(),
                experiment_id: "fence-push-001".into(),
                inputs: inputs.clone(),
                intervention_sha256: Digest([8; 32]),
            },
            control: InterventionExperimentRunArtifact {
                role: InterventionExperimentRole::Control,
                inputs: inputs.clone(),
                gameplay_writes_enabled: false,
                intervention_sha256: None,
                retained_run_sha256: Digest([9; 32]),
                retained_trace_sha256: Digest([10; 32]),
                retained_oracle_report_sha256: Digest([11; 32]),
                retained_intervention_audit_sha256: None,
            },
            treatment: InterventionExperimentRunArtifact {
                role: InterventionExperimentRole::Treatment,
                inputs,
                gameplay_writes_enabled: true,
                intervention_sha256: Some(Digest([8; 32])),
                retained_run_sha256: Digest([12; 32]),
                retained_trace_sha256: Digest([13; 32]),
                retained_oracle_report_sha256: Digest([14; 32]),
                retained_intervention_audit_sha256: Some(Digest([15; 32])),
            },
        }
    }

    #[test]
    fn identical_control_and_treatment_retain_both_artifact_sets() {
        let pair = pair();
        pair.validate().unwrap();
        assert_ne!(
            pair.control.retained_run_sha256,
            pair.treatment.retained_run_sha256
        );
    }

    #[test]
    fn rejects_different_inputs_between_roles() {
        let mut pair = pair();
        pair.control.inputs.absolute_input_tape_sha256 = Digest([99; 32]);
        assert!(pair.validate().is_err());
    }

    #[test]
    fn rejects_intervention_or_write_audit_on_control() {
        let mut pair = pair();
        pair.control.gameplay_writes_enabled = true;
        pair.control.intervention_sha256 = Some(pair.plan.intervention_sha256);
        pair.control.retained_intervention_audit_sha256 = Some(Digest([15; 32]));
        assert!(pair.validate().is_err());
    }

    #[test]
    fn treatment_requires_exact_intervention_and_retained_audit() {
        let mut missing_audit = pair();
        missing_audit.treatment.retained_intervention_audit_sha256 = None;
        assert!(missing_audit.validate().is_err());

        let mut wrong_intervention = pair();
        wrong_intervention.treatment.intervention_sha256 = Some(Digest([42; 32]));
        assert!(wrong_intervention.validate().is_err());
    }

    #[test]
    fn runner_executes_disabled_control_before_exact_treatment() {
        let expected = pair();
        let mut requests = Vec::new();
        let completed = execute_intervention_experiment(expected.plan.clone(), |request| {
            requests.push((
                request.role,
                request.gameplay_writes_enabled,
                request.intervention_sha256,
            ));
            Ok(match request.role {
                InterventionExperimentRole::Control => expected.control.clone(),
                InterventionExperimentRole::Treatment => expected.treatment.clone(),
            })
        })
        .unwrap();
        assert_eq!(
            requests,
            [
                (InterventionExperimentRole::Control, false, None),
                (
                    InterventionExperimentRole::Treatment,
                    true,
                    Some(expected.plan.intervention_sha256),
                ),
            ]
        );
        assert_eq!(completed.control, expected.control);
        assert_eq!(completed.treatment, expected.treatment);
    }
}
