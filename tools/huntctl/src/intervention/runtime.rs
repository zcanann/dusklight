//! Mandatory execution admission and write-audit contract for `DUSKINTR`.

use super::{
    InterventionFlagDomain, InterventionOperation, InterventionPhase, InterventionPrecondition,
    InterventionSelector, InterventionTape, InterventionTimer,
};
use serde::Serialize;
use std::error::Error;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

pub const INTERVENTION_AUDIT_SCHEMA: &str = "dusklight-intervention-write-audit/v1";
pub const EXPERIMENTAL_INTERVENTIONS_COMPILED: bool = true;

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
    FacingYaw(i16),
    CubicCurve([[f32; 3]; 4]),
    TargetPlayer(bool),
    Health(i16),
    Timer {
        timer: InterventionTimer,
        ticks: u16,
    },
    Flag {
        domain: InterventionFlagDomain,
        index: u16,
        value: bool,
    },
    ActorPresence {
        exists: bool,
        position: Option<[f32; 3]>,
    },
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
        if tape
            .interventions
            .iter()
            .any(|intervention| intervention.phase != InterventionPhase::BeforeGameTick)
        {
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

    /// Completes and atomically persists the mandatory audit artifact.
    /// Existing outputs and temporary files are rejected so evidence from a
    /// previous run can never be silently replaced.
    pub fn finish_and_write(self) -> Result<Self, InterventionRuntimeError> {
        let audit = self.finish()?;
        let output = &audit.admission.audit_output;
        if let Some(parent) = output
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(|error| {
                runtime_error(format!(
                    "cannot create intervention audit directory: {error}"
                ))
            })?;
        }
        if output.exists() {
            return Err(runtime_error("intervention audit output already exists"));
        }
        let mut temporary = output.as_os_str().to_owned();
        temporary.push(".tmp");
        let temporary = PathBuf::from(temporary);
        let encoded = serde_json::to_vec_pretty(&audit).map_err(|error| {
            runtime_error(format!("cannot serialize intervention audit: {error}"))
        })?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| {
                runtime_error(format!(
                    "cannot create intervention audit temporary file: {error}"
                ))
            })?;
        let write_result = file
            .write_all(&encoded)
            .and_then(|()| file.write_all(b"\n"))
            .and_then(|()| file.sync_all());
        if let Err(error) = write_result {
            let _ = fs::remove_file(&temporary);
            return Err(runtime_error(format!(
                "cannot persist intervention audit: {error}"
            )));
        }
        drop(file);
        if let Err(error) = fs::rename(&temporary, output) {
            let _ = fs::remove_file(&temporary);
            return Err(runtime_error(format!(
                "cannot install intervention audit: {error}"
            )));
        }
        Ok(audit)
    }
}

pub fn audit_value_for_operation(operation: &InterventionOperation) -> InterventionAuditValue {
    match operation {
        InterventionOperation::SetPosition { value }
        | InterventionOperation::AddPosition { value }
        | InterventionOperation::SetVelocity { value }
        | InterventionOperation::AddVelocity { value } => InterventionAuditValue::Vector3(*value),
        InterventionOperation::SetFacingYaw { value } => InterventionAuditValue::FacingYaw(*value),
        InterventionOperation::MoveAlongCubicCurve { control_points } => {
            InterventionAuditValue::CubicCurve(*control_points)
        }
        InterventionOperation::SetTargetPlayer { enabled } => {
            InterventionAuditValue::TargetPlayer(*enabled)
        }
        InterventionOperation::SetHealth { value } => InterventionAuditValue::Health(*value),
        InterventionOperation::SetTimer { timer, ticks } => InterventionAuditValue::Timer {
            timer: *timer,
            ticks: *ticks,
        },
        InterventionOperation::SetFlag {
            domain,
            index,
            value,
        } => InterventionAuditValue::Flag {
            domain: *domain,
            index: *index,
            value: *value,
        },
        InterventionOperation::SpawnAtPosition { value } => InterventionAuditValue::ActorPresence {
            exists: true,
            position: Some(*value),
        },
        InterventionOperation::Despawn => InterventionAuditValue::ActorPresence {
            exists: false,
            position: None,
        },
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
    fn missing_runtime_opt_in_cannot_admit_writes() {
        let mut disabled = request();
        disabled.allow_gameplay_writes = false;
        assert!(InterventionExecutionAdmission::admit(disabled, &tape()).is_err());
        assert!(InterventionExecutionAdmission::admit(request(), &tape()).is_ok());
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

    #[test]
    fn complete_audit_is_persisted_once_at_the_required_destination() {
        let root = std::env::temp_dir().join(format!(
            "dusklight-intervention-audit-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let output = root.join("audit.json");
        let tape = tape();
        let mut execution_request = request();
        execution_request.audit_output = output.clone();
        let admission = InterventionExecutionAdmission::admit(execution_request, &tape).unwrap();
        let mut audit = InterventionWriteAudit::begin(admission, &tape).unwrap();
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
        let persisted = audit.finish_and_write().unwrap();
        assert!(persisted.complete);
        let decoded: serde_json::Value =
            serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
        assert_eq!(decoded["complete"], true);
        assert_eq!(decoded["entries"][0]["outcome"], "applied");
        assert!(!PathBuf::from(format!("{}.tmp", output.display())).exists());
        assert!(persisted.finish_and_write().is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
