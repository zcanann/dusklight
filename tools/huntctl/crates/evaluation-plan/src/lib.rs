//! Portable, deterministic prelaunch scheduling for native evaluation trials.
//!
//! This crate decides only how already-declared trial identities map to worker
//! lanes. It cannot execute trials, interpret objective truth, rank candidates,
//! or schedule search policy.

use dusklight_automation_contracts::artifact::Digest as ArtifactDigest;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::error::Error;
use std::fmt;

pub const WORKER_SCHEDULE_SCHEMA: &str = "dusklight-evaluation-worker-schedule/v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlannedWorkerAssignment {
    pub trial_index: usize,
    pub candidate_id: String,
    pub attempt: u32,
    pub worker_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluationWorkerSchedule {
    pub schema: String,
    pub worker_lanes: usize,
    pub planned_attempts: usize,
    pub assignments: Vec<PlannedWorkerAssignment>,
}

impl EvaluationWorkerSchedule {
    pub fn build(
        worker_lanes: usize,
        trials: impl IntoIterator<Item = (String, u32)>,
    ) -> Result<Self, WorkerScheduleError> {
        if worker_lanes == 0 {
            return Err(WorkerScheduleError::new(
                "worker schedule must contain at least one lane",
            ));
        }
        let assignments = trials
            .into_iter()
            .enumerate()
            .map(
                |(trial_index, (candidate_id, attempt))| PlannedWorkerAssignment {
                    trial_index,
                    candidate_id,
                    attempt,
                    worker_id: planned_worker_id(trial_index, worker_lanes),
                },
            )
            .collect::<Vec<_>>();
        let schedule = Self {
            schema: WORKER_SCHEDULE_SCHEMA.into(),
            worker_lanes,
            planned_attempts: assignments.len(),
            assignments,
        };
        schedule.validate()?;
        Ok(schedule)
    }

    pub fn validate(&self) -> Result<(), WorkerScheduleError> {
        if self.schema != WORKER_SCHEDULE_SCHEMA {
            return Err(WorkerScheduleError::new(format!(
                "worker schedule schema must be {WORKER_SCHEDULE_SCHEMA}"
            )));
        }
        if self.worker_lanes == 0 {
            return Err(WorkerScheduleError::new(
                "worker schedule must contain at least one lane",
            ));
        }
        if self.planned_attempts != self.assignments.len() {
            return Err(WorkerScheduleError::new(format!(
                "worker schedule declares {} attempts but contains {} assignments",
                self.planned_attempts,
                self.assignments.len()
            )));
        }

        let mut identities = HashSet::with_capacity(self.assignments.len());
        for (trial_index, assignment) in self.assignments.iter().enumerate() {
            if assignment.trial_index != trial_index {
                return Err(WorkerScheduleError::new(format!(
                    "worker schedule assignment {trial_index} claims trial index {}",
                    assignment.trial_index
                )));
            }
            if assignment.candidate_id.is_empty() || assignment.attempt == 0 {
                return Err(WorkerScheduleError::new(format!(
                    "worker schedule assignment {trial_index} has an invalid trial identity"
                )));
            }
            let expected_worker = planned_worker_id(trial_index, self.worker_lanes);
            if assignment.worker_id != expected_worker {
                return Err(WorkerScheduleError::new(format!(
                    "worker schedule assignment {trial_index} names {} instead of {expected_worker}",
                    assignment.worker_id
                )));
            }
            if !identities.insert((&assignment.candidate_id, assignment.attempt)) {
                return Err(WorkerScheduleError::new(format!(
                    "worker schedule repeats candidate {} attempt {}",
                    assignment.candidate_id, assignment.attempt
                )));
            }
        }
        Ok(())
    }

    pub fn sha256(&self) -> Result<ArtifactDigest, WorkerScheduleError> {
        self.validate()?;
        let bytes = serde_json::to_vec_pretty(self).map_err(|error| {
            WorkerScheduleError::new(format!("could not encode worker schedule: {error}"))
        })?;
        Ok(ArtifactDigest(Sha256::digest(bytes).into()))
    }

    pub fn assignments_for_lane(
        &self,
        worker_lane: usize,
    ) -> Result<impl Iterator<Item = &PlannedWorkerAssignment>, WorkerScheduleError> {
        self.validate()?;
        if worker_lane >= self.worker_lanes {
            return Err(WorkerScheduleError::new(format!(
                "worker lane {worker_lane} is outside {} declared lanes",
                self.worker_lanes
            )));
        }
        Ok(self
            .assignments
            .iter()
            .skip(worker_lane)
            .step_by(self.worker_lanes))
    }

    pub fn validate_completed_claims<'a>(
        &self,
        claims: impl IntoIterator<Item = (&'a str, u32, &'a str)>,
    ) -> Result<(), WorkerScheduleError> {
        self.validate()?;
        let planned = self
            .assignments
            .iter()
            .map(|assignment| {
                (
                    (assignment.candidate_id.as_str(), assignment.attempt),
                    assignment.worker_id.as_str(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let mut completed = HashSet::new();
        for (candidate_id, attempt, worker_id) in claims {
            let identity = (candidate_id, attempt);
            if !completed.insert(identity) {
                return Err(WorkerScheduleError::new(format!(
                    "completed evidence repeats candidate {candidate_id} attempt {attempt}"
                )));
            }
            let expected_worker = planned.get(&identity).ok_or_else(|| {
                WorkerScheduleError::new(format!(
                    "completed evidence for candidate {candidate_id} attempt {attempt} is absent from the worker schedule"
                ))
            })?;
            if worker_id != *expected_worker {
                return Err(WorkerScheduleError::new(format!(
                    "completed evidence for candidate {candidate_id} attempt {attempt} names worker {worker_id} instead of {expected_worker}"
                )));
            }
        }
        Ok(())
    }
}

fn planned_worker_id(trial_index: usize, worker_lanes: usize) -> String {
    format!("evaluation/worker-{}", trial_index % worker_lanes)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkerScheduleError {
    message: String,
}

impl WorkerScheduleError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for WorkerScheduleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for WorkerScheduleError {}

#[cfg(test)]
mod tests;
