//! Mandatory no-intervention control pairing for causal intervention runs.

use crate::artifact::Digest;
use crate::episode::{
    EpisodeArtifactIdentity, EpisodeBoundaryIdentity, EpisodeObjectiveIdentity, EpisodeOutcome,
    EpisodeQueryViewIdentity, EpisodeScenarioIdentity, EpisodeSeed, RunBuildIdentity,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;

pub const INTERVENTION_CONTROL_PAIR_SCHEMA: &str = "dusklight-intervention-control-pair/v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PairedRunIdentity {
    pub run_build: RunBuildIdentity,
    pub scenario: EpisodeScenarioIdentity,
    pub parent_boundary: EpisodeBoundaryIdentity,
    pub absolute_tape_sha256: Digest,
    pub query_view: EpisodeQueryViewIdentity,
    pub action_schema_sha256: Digest,
    pub objective: EpisodeObjectiveIdentity,
    pub seed: EpisodeSeed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PairedRunRole {
    Control,
    Treatment,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PairedRunRequest {
    pub role: PairedRunRole,
    pub shared: PairedRunIdentity,
    pub intervention_artifact_sha256: Option<Digest>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PairedRunEvidence {
    pub role: PairedRunRole,
    pub shared: PairedRunIdentity,
    pub intervention_artifact_sha256: Option<Digest>,
    pub intervention_audit_sha256: Option<Digest>,
    pub episode_sha256: Digest,
    pub artifacts: EpisodeArtifactIdentity,
    pub outcome: EpisodeOutcome,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InterventionControlPair {
    pub schema: String,
    pub pair_sha256: Digest,
    pub shared: PairedRunIdentity,
    pub intervention_artifact_sha256: Digest,
    pub control: PairedRunEvidence,
    pub treatment: PairedRunEvidence,
}

#[derive(Debug)]
pub struct InterventionControlPairError(String);

impl fmt::Display for InterventionControlPairError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for InterventionControlPairError {}

/// Executes the intervention-disabled control first and the otherwise
/// identical treatment second. The executor cannot alter shared identity
/// without causing pair construction to fail.
pub fn execute_control_pair<E>(
    shared: PairedRunIdentity,
    intervention_artifact_sha256: Digest,
    mut executor: E,
) -> Result<InterventionControlPair, InterventionControlPairError>
where
    E: FnMut(&PairedRunRequest) -> Result<PairedRunEvidence, InterventionControlPairError>,
{
    validate_shared(&shared)?;
    if intervention_artifact_sha256 == Digest::ZERO {
        return Err(pair_error("intervention artifact identity is missing"));
    }
    let control_request = PairedRunRequest {
        role: PairedRunRole::Control,
        shared: shared.clone(),
        intervention_artifact_sha256: None,
    };
    let control = executor(&control_request)?;
    let treatment_request = PairedRunRequest {
        role: PairedRunRole::Treatment,
        shared: shared.clone(),
        intervention_artifact_sha256: Some(intervention_artifact_sha256),
    };
    let treatment = executor(&treatment_request)?;
    InterventionControlPair::build(shared, intervention_artifact_sha256, control, treatment)
}

impl InterventionControlPair {
    pub fn build(
        shared: PairedRunIdentity,
        intervention_artifact_sha256: Digest,
        control: PairedRunEvidence,
        treatment: PairedRunEvidence,
    ) -> Result<Self, InterventionControlPairError> {
        let mut pair = Self {
            schema: INTERVENTION_CONTROL_PAIR_SCHEMA.into(),
            pair_sha256: Digest::ZERO,
            shared,
            intervention_artifact_sha256,
            control,
            treatment,
        };
        pair.validate_members()?;
        pair.pair_sha256 = pair.compute_identity()?;
        Ok(pair)
    }

    pub fn validate(&self) -> Result<(), InterventionControlPairError> {
        self.validate_members()?;
        if self.pair_sha256 == Digest::ZERO || self.pair_sha256 != self.compute_identity()? {
            return Err(pair_error("intervention control-pair identity mismatch"));
        }
        Ok(())
    }

    fn validate_members(&self) -> Result<(), InterventionControlPairError> {
        if self.schema != INTERVENTION_CONTROL_PAIR_SCHEMA
            || self.intervention_artifact_sha256 == Digest::ZERO
        {
            return Err(pair_error("intervention control-pair header is invalid"));
        }
        validate_shared(&self.shared)?;
        validate_evidence(&self.control, PairedRunRole::Control, &self.shared, None)?;
        validate_evidence(
            &self.treatment,
            PairedRunRole::Treatment,
            &self.shared,
            Some(self.intervention_artifact_sha256),
        )?;
        Ok(())
    }

    fn compute_identity(&self) -> Result<Digest, InterventionControlPairError> {
        let encoded = serde_json::to_vec(&(
            &self.schema,
            &self.shared,
            self.intervention_artifact_sha256,
            &self.control,
            &self.treatment,
        ))
        .map_err(|error| pair_error(error.to_string()))?;
        let mut hasher = Sha256::new();
        hasher.update(b"dusklight.intervention-control-pair/v1\0");
        hasher.update((encoded.len() as u64).to_le_bytes());
        hasher.update(encoded);
        Ok(Digest(hasher.finalize().into()))
    }
}

fn validate_shared(shared: &PairedRunIdentity) -> Result<(), InterventionControlPairError> {
    if shared.run_build.executable_sha256 == Digest::ZERO
        || shared.scenario.digest == Digest::ZERO
        || shared.parent_boundary.digest == Digest::ZERO
        || shared.absolute_tape_sha256 == Digest::ZERO
        || shared.query_view.schema_sha256 == Digest::ZERO
        || shared.action_schema_sha256 == Digest::ZERO
        || shared.objective.digest == Digest::ZERO
        || shared.scenario.id.is_empty()
        || shared.query_view.id.is_empty()
        || shared.objective.id.is_empty()
    {
        return Err(pair_error("paired intervention run identity is incomplete"));
    }
    Ok(())
}

fn validate_evidence(
    evidence: &PairedRunEvidence,
    role: PairedRunRole,
    shared: &PairedRunIdentity,
    intervention: Option<Digest>,
) -> Result<(), InterventionControlPairError> {
    let digests = [
        evidence.episode_sha256,
        evidence.artifacts.absolute_tape_sha256,
        evidence.artifacts.gameplay_trace_sha256,
        evidence.artifacts.transition_corpus_sha256,
        evidence.artifacts.transition_evidence_sha256,
    ];
    if evidence.role != role
        || &evidence.shared != shared
        || evidence.intervention_artifact_sha256 != intervention
        || evidence.artifacts.absolute_tape_sha256 != shared.absolute_tape_sha256
        || digests.contains(&Digest::ZERO)
    {
        return Err(pair_error(
            "paired run evidence changes shared identity or omits retained artifacts",
        ));
    }
    match role {
        PairedRunRole::Control if evidence.intervention_audit_sha256.is_some() => Err(pair_error(
            "no-intervention control cannot carry a mutation audit",
        )),
        PairedRunRole::Treatment
            if evidence
                .intervention_audit_sha256
                .is_none_or(|digest| digest == Digest::ZERO) =>
        {
            Err(pair_error(
                "intervention treatment requires its retained mutation audit",
            ))
        }
        _ => Ok(()),
    }
}

fn pair_error(message: impl Into<String>) -> InterventionControlPairError {
    InterventionControlPairError(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::episode::{EpisodeOutcomeClass, EpisodeReferenceKind};
    use crate::tape::TapeBoot;

    fn shared() -> PairedRunIdentity {
        PairedRunIdentity {
            run_build: RunBuildIdentity {
                executable_sha256: Digest([1; 32]),
                dusklight_commit: Some("dusk".into()),
                aurora_commit: Some("aurora".into()),
                target: Some("arm64-macos".into()),
                profile: Some("debug".into()),
                feature_digest: Some(Digest([2; 32])),
            },
            scenario: EpisodeScenarioIdentity {
                id: "process-boot".into(),
                digest: Digest([3; 32]),
                boot: TapeBoot::Process,
            },
            parent_boundary: EpisodeBoundaryIdentity {
                kind: EpisodeReferenceKind::Boundary,
                digest: Digest([4; 32]),
            },
            absolute_tape_sha256: Digest([5; 32]),
            query_view: EpisodeQueryViewIdentity {
                id: "movement-state/v2".into(),
                schema_sha256: Digest([6; 32]),
            },
            action_schema_sha256: Digest([7; 32]),
            objective: EpisodeObjectiveIdentity {
                id: "fence-crossing".into(),
                digest: Digest([8; 32]),
            },
            seed: EpisodeSeed::Deterministic { value: 9 },
        }
    }

    fn evidence(request: &PairedRunRequest) -> PairedRunEvidence {
        PairedRunEvidence {
            role: request.role,
            shared: request.shared.clone(),
            intervention_artifact_sha256: request.intervention_artifact_sha256,
            intervention_audit_sha256: (request.role == PairedRunRole::Treatment)
                .then_some(Digest([20; 32])),
            episode_sha256: match request.role {
                PairedRunRole::Control => Digest([10; 32]),
                PairedRunRole::Treatment => Digest([11; 32]),
            },
            artifacts: EpisodeArtifactIdentity {
                absolute_tape_sha256: request.shared.absolute_tape_sha256,
                gameplay_trace_sha256: match request.role {
                    PairedRunRole::Control => Digest([12; 32]),
                    PairedRunRole::Treatment => Digest([13; 32]),
                },
                transition_corpus_sha256: match request.role {
                    PairedRunRole::Control => Digest([14; 32]),
                    PairedRunRole::Treatment => Digest([15; 32]),
                },
                transition_evidence_sha256: match request.role {
                    PairedRunRole::Control => Digest([16; 32]),
                    PairedRunRole::Treatment => Digest([17; 32]),
                },
            },
            outcome: EpisodeOutcome {
                class: EpisodeOutcomeClass::Failed,
                reason: "retained causal observation".into(),
            },
        }
    }

    #[test]
    fn control_runs_first_without_intervention_and_both_artifacts_are_retained() {
        let mut roles = Vec::new();
        let pair = execute_control_pair(shared(), Digest([19; 32]), |request| {
            roles.push((request.role, request.intervention_artifact_sha256));
            Ok(evidence(request))
        })
        .unwrap();
        assert_eq!(
            roles,
            [
                (PairedRunRole::Control, None),
                (PairedRunRole::Treatment, Some(Digest([19; 32])))
            ]
        );
        assert_ne!(pair.control.episode_sha256, pair.treatment.episode_sha256);
        assert_eq!(
            pair.control.artifacts.absolute_tape_sha256,
            pair.treatment.artifacts.absolute_tape_sha256
        );
        pair.validate().unwrap();
    }

    #[test]
    fn identity_drift_or_missing_audit_rejects_the_pair() {
        let shared = shared();
        let control_request = PairedRunRequest {
            role: PairedRunRole::Control,
            shared: shared.clone(),
            intervention_artifact_sha256: None,
        };
        let treatment_request = PairedRunRequest {
            role: PairedRunRole::Treatment,
            shared: shared.clone(),
            intervention_artifact_sha256: Some(Digest([19; 32])),
        };
        let control = evidence(&control_request);
        let mut treatment = evidence(&treatment_request);
        treatment.shared.absolute_tape_sha256 = Digest([99; 32]);
        assert!(
            InterventionControlPair::build(
                shared.clone(),
                Digest([19; 32]),
                control.clone(),
                treatment,
            )
            .is_err()
        );
        let mut treatment = evidence(&treatment_request);
        treatment.intervention_audit_sha256 = None;
        assert!(
            InterventionControlPair::build(shared, Digest([19; 32]), control, treatment).is_err()
        );
    }

    #[test]
    fn pair_identity_authenticates_both_retained_results() {
        let mut pair =
            execute_control_pair(shared(), Digest([19; 32]), |request| Ok(evidence(request)))
                .unwrap();
        pair.control.outcome.reason.push_str(" tampered");
        assert!(pair.validate().is_err());
    }
}
