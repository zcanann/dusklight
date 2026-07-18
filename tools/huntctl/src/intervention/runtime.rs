//! Mandatory execution admission and write-audit contract for `DUSKINTR`.

use super::{
    InterventionOperation, InterventionPhase, InterventionPrecondition, InterventionSelector,
    InterventionTape,
};
use serde::Serialize;
use std::error::Error;
use std::fmt;
use std::path::PathBuf;

pub const INTERVENTION_AUDIT_SCHEMA: &str = "dusklight-intervention-write-audit/v1";
pub const EXPERIMENTAL_INTERVENTIONS_COMPILED: bool = cfg!(feature = "experimental-interventions");

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InterventionFidelity {
    ExperimentalTypedGameplayWrites,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InterventionExecutionRequest {
    pub allow_gameplay_writes: bool,
    pub fidelity: InterventionFidelity,
    pub audit_output: PathBuf,
    pub intervention_artifact_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct InterventionExecutionAdmission {
    pub compile_time_capability: bool,
    pub runtime_write_opt_in: bool,
    pub fidelity: InterventionFidelity,
    pub phase: InterventionPhase,
    pub preconditions_required: bool,
    pub audit_required: bool,
    pub audit_output: PathBuf,
    pub intervention_artifact_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum InterventionAuditValue {
    Vector3([f32; 3]),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InterventionAuditOutcome {
    Applied,
    PreconditionsFailed,
    TargetLost,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct InterventionWriteAuditEntry {
    pub intervention_index: usize,
    pub tick: u32,
    pub phase: InterventionPhase,
    pub selector: InterventionSelector,
    pub resolved_process_id: Option<u32>,
    pub precondition: InterventionPrecondition,
    pub before: Option<InterventionAuditValue>,
    pub written: Option<InterventionAuditValue>,
    pub after: Option<InterventionAuditValue>,
    pub outcome: InterventionAuditOutcome,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct InterventionWriteAudit {
    pub schema: &'static str,
    pub admission: InterventionExecutionAdmission,
    pub expected_applications: usize,
    pub entries: Vec<InterventionWriteAuditEntry>,
    pub complete: bool,
}

#[derive(Debug)]
pub struct InterventionRuntimeError(String);

impl fmt::Display for InterventionRuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for InterventionRuntimeError {}

impl InterventionExecutionAdmission {
    pub fn admit(
        request: InterventionExecutionRequest,
        tape: &InterventionTape,
    ) -> Result<Self, InterventionRuntimeError> {
        tape.validate()
            .map_err(|error| InterventionRuntimeError(error.to_string()))?;
        if !request.allow_gameplay_writes {
            return Err(runtime_error(
                "runtime --allow-gameplay-writes opt-in is required",
            ));
        }
        if request.audit_output.as_os_str().is_empty() {
            return Err(runtime_error(
                "an always-on intervention audit output is required",
            ));
        }
        validate_sha256(&request.intervention_artifact_sha256)?;
        if tape.interventions.iter().any(|intervention| {
            intervention.phase != InterventionPhase::BeforeGameTick
                || intervention.precondition != InterventionPrecondition::ActorExists
        }) {
            return Err(runtime_error(
                "intervention phase or precondition is not executable",
            ));
        }
        if !EXPERIMENTAL_INTERVENTIONS_COMPILED {
            return Err(runtime_error(
                "binary lacks the experimental-interventions compile-time capability",
            ));
        }
        Ok(Self {
            compile_time_capability: true,
            runtime_write_opt_in: true,
            fidelity: request.fidelity,
            phase: InterventionPhase::BeforeGameTick,
            preconditions_required: true,
            audit_required: true,
            audit_output: request.audit_output,
            intervention_artifact_sha256: request.intervention_artifact_sha256,
        })
    }
}

impl InterventionWriteAudit {
    pub fn begin(
        admission: InterventionExecutionAdmission,
        tape: &InterventionTape,
    ) -> Result<Self, InterventionRuntimeError> {
        if !admission.audit_required || admission.audit_output.as_os_str().is_empty() {
            return Err(runtime_error("intervention audit cannot be disabled"));
        }
        let expected_applications = tape
            .interventions
            .iter()
            .try_fold(0_usize, |total, intervention| {
                total.checked_add(intervention.duration_ticks as usize)
            })
            .ok_or_else(|| runtime_error("intervention audit application count overflow"))?;
        Ok(Self {
            schema: INTERVENTION_AUDIT_SCHEMA,
            admission,
            expected_applications,
            entries: Vec::with_capacity(expected_applications),
            complete: false,
        })
    }

    pub fn record(
        &mut self,
        tape: &InterventionTape,
        entry: InterventionWriteAuditEntry,
    ) -> Result<(), InterventionRuntimeError> {
        if self.complete || self.entries.len() >= self.expected_applications {
            return Err(runtime_error(
                "intervention audit is complete or over capacity",
            ));
        }
        let intervention = tape
            .interventions
            .get(entry.intervention_index)
            .ok_or_else(|| {
                runtime_error("intervention audit references an unknown intervention")
            })?;
        if entry.phase != intervention.phase
            || entry.selector != intervention.selector
            || entry.precondition != intervention.precondition
            || entry.tick < intervention.start_tick
            || entry.tick >= intervention.start_tick + intervention.duration_ticks
        {
            return Err(runtime_error(
                "intervention audit entry disagrees with the timeline",
            ));
        }
        match entry.outcome {
            InterventionAuditOutcome::Applied => {
                if entry.resolved_process_id.is_none()
                    || entry.before.is_none()
                    || entry.written.is_none()
                    || entry.after.is_none()
                {
                    return Err(runtime_error(
                        "applied intervention audit requires resolved before/write/after values",
                    ));
                }
            }
            InterventionAuditOutcome::PreconditionsFailed
            | InterventionAuditOutcome::TargetLost => {
                if entry.written.is_some() || entry.after.is_some() {
                    return Err(runtime_error(
                        "failed intervention audit cannot claim a write or after value",
                    ));
                }
            }
        }
        self.entries.push(entry);
        Ok(())
    }

    pub fn finish(mut self) -> Result<Self, InterventionRuntimeError> {
        if self.entries.len() != self.expected_applications {
            return Err(runtime_error(
                "intervention audit is missing scheduled applications",
            ));
        }
        self.complete = true;
        Ok(self)
    }
}

pub fn audit_value_for_operation(operation: &InterventionOperation) -> InterventionAuditValue {
    match operation {
        InterventionOperation::SetPosition { value }
        | InterventionOperation::AddVelocity { value } => InterventionAuditValue::Vector3(*value),
    }
}

fn validate_sha256(digest: &str) -> Result<(), InterventionRuntimeError> {
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(runtime_error(
            "intervention artifact identity is not lowercase SHA-256",
        ));
    }
    Ok(())
}

fn runtime_error(message: impl Into<String>) -> InterventionRuntimeError {
    InterventionRuntimeError(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tape() -> InterventionTape {
        InterventionTape::compile_dsl(
            "timeline 4\nat 1 for 1 before_game_tick process 7 require actor_exists set_position 1 2 3",
        )
        .unwrap()
    }

    fn request() -> InterventionExecutionRequest {
        InterventionExecutionRequest {
            allow_gameplay_writes: true,
            fidelity: InterventionFidelity::ExperimentalTypedGameplayWrites,
            audit_output: "audit/intervention.json".into(),
            intervention_artifact_sha256: "ab".repeat(32),
        }
    }

    #[test]
    fn normal_build_or_missing_runtime_opt_in_cannot_admit_writes() {
        let mut disabled = request();
        disabled.allow_gameplay_writes = false;
        assert!(InterventionExecutionAdmission::admit(disabled, &tape()).is_err());
        let result = InterventionExecutionAdmission::admit(request(), &tape());
        assert_eq!(result.is_ok(), EXPERIMENTAL_INTERVENTIONS_COMPILED);
    }

    #[cfg(feature = "experimental-interventions")]
    #[test]
    fn enabled_build_requires_complete_before_write_after_audit() {
        let tape = tape();
        let admission = InterventionExecutionAdmission::admit(request(), &tape).unwrap();
        let mut audit = InterventionWriteAudit::begin(admission, &tape).unwrap();
        assert!(audit.clone().finish().is_err());
        let intervention = &tape.interventions[0];
        audit
            .record(
                &tape,
                InterventionWriteAuditEntry {
                    intervention_index: 0,
                    tick: 1,
                    phase: intervention.phase,
                    selector: intervention.selector.clone(),
                    resolved_process_id: Some(7),
                    precondition: intervention.precondition,
                    before: Some(InterventionAuditValue::Vector3([0.0; 3])),
                    written: Some(audit_value_for_operation(&intervention.operation)),
                    after: Some(InterventionAuditValue::Vector3([1.0, 2.0, 3.0])),
                    outcome: InterventionAuditOutcome::Applied,
                },
            )
            .unwrap();
        assert!(audit.finish().unwrap().complete);
    }

    #[cfg(feature = "experimental-interventions")]
    #[test]
    fn applied_entry_cannot_omit_any_audit_value() {
        let tape = tape();
        let admission = InterventionExecutionAdmission::admit(request(), &tape).unwrap();
        let mut audit = InterventionWriteAudit::begin(admission, &tape).unwrap();
        let intervention = &tape.interventions[0];
        assert!(
            audit
                .record(
                    &tape,
                    InterventionWriteAuditEntry {
                        intervention_index: 0,
                        tick: 1,
                        phase: intervention.phase,
                        selector: intervention.selector.clone(),
                        resolved_process_id: Some(7),
                        precondition: intervention.precondition,
                        before: Some(InterventionAuditValue::Vector3([0.0; 3])),
                        written: None,
                        after: None,
                        outcome: InterventionAuditOutcome::Applied,
                    },
                )
                .is_err()
        );
    }
}
