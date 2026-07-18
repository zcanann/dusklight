//! Evidence-strength labels for intervention findings and natural reproduction.

use super::experiment::{
    InterventionExperimentInputs, InterventionExperimentPair, InterventionExperimentPlan,
};
use crate::artifact::Digest;
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fmt;

pub const INTERVENTION_EVIDENCE_SCHEMA: &str = "dusklight-intervention-evidence/v1";
const MAX_MECHANISM_SUMMARY_BYTES: usize = 2_048;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InterventionEvidenceLevel {
    Existence,
    Mechanism,
    NormalInputReproduced,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NormalInputReproductionEvidence {
    pub inputs: InterventionExperimentInputs,
    pub gameplay_writes_enabled: bool,
    pub intervention_sha256: Option<Digest>,
    pub intervention_audit_sha256: Option<Digest>,
    pub cold_replay_count: u16,
    pub setup_reproduced_each_replay: bool,
    pub retained_replay_bundle_sha256: Digest,
    pub retained_trace_sha256: Digest,
    pub retained_oracle_report_sha256: Digest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InterventionEvidenceClaim {
    pub schema: String,
    pub source_experiment_sha256: Digest,
    pub source_plan: InterventionExperimentPlan,
    pub level: InterventionEvidenceLevel,
    pub mechanism_summary: Option<String>,
    pub normal_input_reproduction: Option<NormalInputReproductionEvidence>,
}

#[derive(Debug)]
pub struct InterventionEvidenceError(String);

impl fmt::Display for InterventionEvidenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for InterventionEvidenceError {}

impl InterventionEvidenceClaim {
    pub fn from_intervention_experiment(
        pair: &InterventionExperimentPair,
        source_experiment_sha256: Digest,
        level: InterventionEvidenceLevel,
        mechanism_summary: Option<String>,
    ) -> Result<Self, InterventionEvidenceError> {
        pair.validate()
            .map_err(|error| evidence_error(error.to_string()))?;
        if level == InterventionEvidenceLevel::NormalInputReproduced {
            return Err(evidence_error(
                "an intervention experiment cannot directly claim normal-input reproduction",
            ));
        }
        let claim = Self {
            schema: INTERVENTION_EVIDENCE_SCHEMA.into(),
            source_experiment_sha256,
            source_plan: pair.plan.clone(),
            level,
            mechanism_summary,
            normal_input_reproduction: None,
        };
        claim.validate()?;
        Ok(claim)
    }

    pub fn promote_with_normal_input(
        mut self,
        reproduction: NormalInputReproductionEvidence,
    ) -> Result<Self, InterventionEvidenceError> {
        validate_normal_reproduction(&self.source_plan.inputs, &reproduction)?;
        self.level = InterventionEvidenceLevel::NormalInputReproduced;
        self.normal_input_reproduction = Some(reproduction);
        self.validate()?;
        Ok(self)
    }

    pub fn validate(&self) -> Result<(), InterventionEvidenceError> {
        if self.schema != INTERVENTION_EVIDENCE_SCHEMA
            || self.source_experiment_sha256 == Digest::ZERO
        {
            return Err(evidence_error("intervention evidence identity is invalid"));
        }
        self.source_plan
            .validate()
            .map_err(|error| evidence_error(error.to_string()))?;
        if self.mechanism_summary.as_ref().is_some_and(|summary| {
            summary.is_empty() || summary.len() > MAX_MECHANISM_SUMMARY_BYTES
        }) {
            return Err(evidence_error("intervention mechanism summary is invalid"));
        }
        match self.level {
            InterventionEvidenceLevel::Existence => {
                if self.mechanism_summary.is_some() || self.normal_input_reproduction.is_some() {
                    return Err(evidence_error(
                        "existence evidence cannot claim a mechanism or natural reproduction",
                    ));
                }
            }
            InterventionEvidenceLevel::Mechanism => {
                if self.mechanism_summary.is_none() || self.normal_input_reproduction.is_some() {
                    return Err(evidence_error(
                        "mechanism evidence requires a summary and cannot claim natural reproduction",
                    ));
                }
            }
            InterventionEvidenceLevel::NormalInputReproduced => {
                let reproduction = self.normal_input_reproduction.as_ref().ok_or_else(|| {
                    evidence_error("normal-input evidence is missing its reproduction proof")
                })?;
                validate_normal_reproduction(&self.source_plan.inputs, reproduction)?;
            }
        }
        Ok(())
    }
}

fn validate_normal_reproduction(
    source: &InterventionExperimentInputs,
    reproduction: &NormalInputReproductionEvidence,
) -> Result<(), InterventionEvidenceError> {
    let inputs = &reproduction.inputs;
    if inputs.build.dusklight_commit != source.build.dusklight_commit
        || inputs.build.aurora_commit != source.build.aurora_commit
        || inputs.build.compiler != source.build.compiler
        || inputs.build.target != source.build.target
        || inputs.build.profile != source.build.profile
        || inputs.build.game_digest != source.build.game_digest
        || inputs.build.dirty_digest != source.build.dirty_digest
        || inputs.scenario_id != source.scenario_id
        || inputs.scenario_sha256 != source.scenario_sha256
        || inputs.parent_boundary_sha256 != source.parent_boundary_sha256
        || inputs.observation_schema_sha256 != source.observation_schema_sha256
        || inputs.oracle_program_sha256 != source.oracle_program_sha256
    {
        return Err(evidence_error(
            "normal-input reproduction changed the source/game, setup, observation, or oracle identity",
        ));
    }
    if inputs.build.feature_digest == source.build.feature_digest
        || inputs.build.fidelity_profile == "experimental_typed_gameplay_writes"
    {
        return Err(evidence_error(
            "normal-input reproduction must use a distinct non-intervention build capability",
        ));
    }
    if inputs.absolute_input_tape_sha256 == Digest::ZERO
        || reproduction.gameplay_writes_enabled
        || reproduction.intervention_sha256.is_some()
        || reproduction.intervention_audit_sha256.is_some()
    {
        return Err(evidence_error(
            "normal-input reproduction must use an absolute tape with all gameplay writes disabled",
        ));
    }
    if reproduction.cold_replay_count < 2 || !reproduction.setup_reproduced_each_replay {
        return Err(evidence_error(
            "normal-input reproduction requires repeated successful cold replay",
        ));
    }
    if [
        reproduction.retained_replay_bundle_sha256,
        reproduction.retained_trace_sha256,
        reproduction.retained_oracle_report_sha256,
    ]
    .contains(&Digest::ZERO)
    {
        return Err(evidence_error(
            "normal-input reproduction is missing retained proof artifacts",
        ));
    }
    Ok(())
}

fn evidence_error(message: impl Into<String>) -> InterventionEvidenceError {
    InterventionEvidenceError(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact::BuildIdentity;
    use crate::intervention::experiment::{
        INTERVENTION_EXPERIMENT_PAIR_SCHEMA, INTERVENTION_EXPERIMENT_PLAN_SCHEMA,
        InterventionExperimentRole, InterventionExperimentRunArtifact,
    };

    fn inputs(tape: u8) -> InterventionExperimentInputs {
        InterventionExperimentInputs {
            build: BuildIdentity {
                dusklight_commit: "dusk".into(),
                aurora_commit: "aurora".into(),
                compiler: "clang".into(),
                target: "aarch64-apple-darwin".into(),
                profile: "debug".into(),
                feature_digest: Digest([1; 32]),
                game_digest: Digest([2; 32]),
                dirty_digest: None,
                fidelity_profile: "experimental_typed_gameplay_writes".into(),
            },
            scenario_id: "fence-push".into(),
            scenario_sha256: Digest([3; 32]),
            parent_boundary_sha256: Digest([4; 32]),
            absolute_input_tape_sha256: Digest([tape; 32]),
            observation_schema_sha256: Digest([6; 32]),
            oracle_program_sha256: Digest([7; 32]),
        }
    }

    fn pair() -> InterventionExperimentPair {
        let inputs = inputs(5);
        InterventionExperimentPair {
            schema: INTERVENTION_EXPERIMENT_PAIR_SCHEMA.into(),
            plan: InterventionExperimentPlan {
                schema: INTERVENTION_EXPERIMENT_PLAN_SCHEMA.into(),
                experiment_id: "fence-push-claim".into(),
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

    fn normal_reproduction() -> NormalInputReproductionEvidence {
        let mut reproduction_inputs = inputs(20);
        reproduction_inputs.build.feature_digest = Digest([19; 32]);
        reproduction_inputs.build.fidelity_profile = "retail_compatible_observers".into();
        NormalInputReproductionEvidence {
            inputs: reproduction_inputs,
            gameplay_writes_enabled: false,
            intervention_sha256: None,
            intervention_audit_sha256: None,
            cold_replay_count: 3,
            setup_reproduced_each_replay: true,
            retained_replay_bundle_sha256: Digest([21; 32]),
            retained_trace_sha256: Digest([22; 32]),
            retained_oracle_report_sha256: Digest([23; 32]),
        }
    }

    #[test]
    fn intervention_results_are_limited_to_existence_or_mechanism() {
        assert!(
            InterventionEvidenceClaim::from_intervention_experiment(
                &pair(),
                Digest([30; 32]),
                InterventionEvidenceLevel::Existence,
                None,
            )
            .is_ok()
        );
        assert!(
            InterventionEvidenceClaim::from_intervention_experiment(
                &pair(),
                Digest([30; 32]),
                InterventionEvidenceLevel::NormalInputReproduced,
                None,
            )
            .is_err()
        );
    }

    #[test]
    fn mechanism_requires_an_explicit_bounded_summary() {
        assert!(
            InterventionEvidenceClaim::from_intervention_experiment(
                &pair(),
                Digest([30; 32]),
                InterventionEvidenceLevel::Mechanism,
                None,
            )
            .is_err()
        );
        assert!(
            InterventionEvidenceClaim::from_intervention_experiment(
                &pair(),
                Digest([30; 32]),
                InterventionEvidenceLevel::Mechanism,
                Some("enemy velocity creates the fence-crossing contact".into()),
            )
            .is_ok()
        );
    }

    #[test]
    fn only_repeated_write_free_normal_input_can_promote_the_claim() {
        let claim = InterventionEvidenceClaim::from_intervention_experiment(
            &pair(),
            Digest([30; 32]),
            InterventionEvidenceLevel::Existence,
            None,
        )
        .unwrap();
        let promoted = claim
            .clone()
            .promote_with_normal_input(normal_reproduction())
            .unwrap();
        assert_eq!(
            promoted.level,
            InterventionEvidenceLevel::NormalInputReproduced
        );

        let mut writes_enabled = normal_reproduction();
        writes_enabled.gameplay_writes_enabled = true;
        assert!(
            claim
                .clone()
                .promote_with_normal_input(writes_enabled)
                .is_err()
        );
        let mut one_replay = normal_reproduction();
        one_replay.cold_replay_count = 1;
        assert!(claim.promote_with_normal_input(one_replay).is_err());

        let mut intervention_build = normal_reproduction();
        intervention_build.inputs.build.feature_digest = Digest([1; 32]);
        intervention_build.inputs.build.fidelity_profile =
            "experimental_typed_gameplay_writes".into();
        assert!(
            InterventionEvidenceClaim::from_intervention_experiment(
                &pair(),
                Digest([30; 32]),
                InterventionEvidenceLevel::Existence,
                None,
            )
            .unwrap()
            .promote_with_normal_input(intervention_build)
            .is_err()
        );
    }

    #[test]
    fn reproduction_cannot_change_the_source_setup_identity() {
        let claim = InterventionEvidenceClaim::from_intervention_experiment(
            &pair(),
            Digest([30; 32]),
            InterventionEvidenceLevel::Existence,
            None,
        )
        .unwrap();
        let mut drifted = normal_reproduction();
        drifted.inputs.scenario_sha256 = Digest([99; 32]);
        assert!(claim.promote_with_normal_input(drifted).is_err());
    }
}
