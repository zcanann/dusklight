//! Graph-owned work contracts shared by persistent native workers.

use crate::scheduler::ReplayableExpansionQueue;
use crate::state_graph::{
    ActionExpansionStatus, ExactStateId, ExpansionAdmission, ExpansionEvidenceAuthority,
    StateGraph, StateGraphError,
};
use dusklight_automation_contracts::artifact::Digest;
use dusklight_automation_contracts::tape::InputTape;
use dusklight_learning::option_transition::OptionTransitionSample;
use dusklight_learning::option_values::OptionActionDescriptor;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;

pub const GRAPH_EXPANSION_JOB_SCHEMA_V1: &str = "dusklight-graph-expansion-job/v1";

/// Compact worker message. Large routes and typed snapshots remain in the
/// shared graph/content store and are addressed by their exact identities.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GraphExpansionJob {
    pub schema: String,
    pub leased_graph_sha256: Digest,
    pub scheduler_queue_sha256: Digest,
    pub expansion_sha256: Digest,
    pub source: ExactStateId,
    pub action: OptionActionDescriptor,
    pub lease_sha256: Digest,
    pub lease_expires_at_generation: u64,
    pub job_sha256: Digest,
}

impl GraphExpansionJob {
    pub fn from_leased_graph(
        graph: &StateGraph,
        queue: &ReplayableExpansionQueue,
        lease_sha256: Digest,
    ) -> Result<Self, WorkerPoolError> {
        queue.validate().map_err(WorkerPoolError::Scheduler)?;
        let expansion_sha256 = queue
            .selected_expansion_sha256
            .ok_or(WorkerPoolError::Invalid(
                "worker job queue has no selected expansion",
            ))?;
        let expansion = graph
            .expansion(expansion_sha256)
            .ok_or(WorkerPoolError::Invalid("worker job expansion is absent"))?;
        let ActionExpansionStatus::Leased {
            lease_sha256: active_lease,
            expires_at_generation,
        } = &expansion.status
        else {
            return Err(WorkerPoolError::Invalid(
                "worker job expansion is not leased",
            ));
        };
        if *active_lease != lease_sha256 || lease_sha256 == Digest::ZERO {
            return Err(WorkerPoolError::Invalid(
                "worker job lease identity does not match",
            ));
        }
        let leased_graph_sha256 = graph.content_sha256()?;
        let job_sha256 = graph_expansion_job_sha256(
            leased_graph_sha256,
            queue.queue_sha256,
            expansion_sha256,
            expansion.source,
            &expansion.action,
            lease_sha256,
            *expires_at_generation,
        )?;
        Ok(Self {
            schema: GRAPH_EXPANSION_JOB_SCHEMA_V1.into(),
            leased_graph_sha256,
            scheduler_queue_sha256: queue.queue_sha256,
            expansion_sha256,
            source: expansion.source,
            action: expansion.action.clone(),
            lease_sha256,
            lease_expires_at_generation: *expires_at_generation,
            job_sha256,
        })
    }

    pub fn validate_against(&self, graph: &StateGraph) -> Result<(), WorkerPoolError> {
        let expansion = graph
            .expansion(self.expansion_sha256)
            .ok_or(WorkerPoolError::Invalid("worker job expansion is absent"))?;
        if self.schema != GRAPH_EXPANSION_JOB_SCHEMA_V1
            || expansion.source != self.source
            || expansion.action != self.action
            || !matches!(
                &expansion.status,
                ActionExpansionStatus::Leased {
                    lease_sha256,
                    expires_at_generation,
                } if *lease_sha256 == self.lease_sha256
                    && *expires_at_generation == self.lease_expires_at_generation
            )
            || self.job_sha256
                != graph_expansion_job_sha256(
                    self.leased_graph_sha256,
                    self.scheduler_queue_sha256,
                    self.expansion_sha256,
                    self.source,
                    &self.action,
                    self.lease_sha256,
                    self.lease_expires_at_generation,
                )?
        {
            return Err(WorkerPoolError::Invalid(
                "worker job is detached from its graph lease",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct GraphExpansionCompletion {
    pub job_sha256: Digest,
    pub transition: OptionTransitionSample,
    pub route: InputTape,
    pub episode_group: u64,
    pub authority: ExpansionEvidenceAuthority,
}

pub fn admit_graph_expansion_completion(
    graph: &mut StateGraph,
    job: &GraphExpansionJob,
    completion: GraphExpansionCompletion,
) -> Result<ExpansionAdmission, WorkerPoolError> {
    job.validate_against(graph)?;
    if completion.job_sha256 != job.job_sha256
        || completion.transition.before_state_sha256 != job.source.state_sha256
        || completion.transition.source_checkpoint_sha256 != job.source.route_checkpoint_sha256
        || completion.transition.value_sample.action != job.action
    {
        return Err(WorkerPoolError::Invalid(
            "worker completion is detached from its leased job",
        ));
    }
    Ok(graph.admit_leased_completed_expansion(
        completion.transition,
        completion.route,
        completion.episode_group,
        completion.authority,
        job.lease_sha256,
    )?)
}

fn graph_expansion_job_sha256(
    leased_graph_sha256: Digest,
    scheduler_queue_sha256: Digest,
    expansion_sha256: Digest,
    source: ExactStateId,
    action: &OptionActionDescriptor,
    lease_sha256: Digest,
    lease_expires_at_generation: u64,
) -> Result<Digest, WorkerPoolError> {
    let mut hasher = Sha256::new();
    hasher.update(GRAPH_EXPANSION_JOB_SCHEMA_V1.as_bytes());
    hasher.update(leased_graph_sha256.0);
    hasher.update(scheduler_queue_sha256.0);
    hasher.update(expansion_sha256.0);
    hasher.update(source.route_checkpoint_sha256.0);
    hasher.update(source.state_sha256.0);
    hasher.update(action.content_sha256()?.0);
    hasher.update(lease_sha256.0);
    hasher.update(lease_expires_at_generation.to_le_bytes());
    Ok(Digest(hasher.finalize().into()))
}

#[derive(Debug)]
pub enum WorkerPoolError {
    Invalid(&'static str),
    Scheduler(crate::scheduler::SchedulerError),
    Graph(StateGraphError),
    Action(dusklight_learning::option_values::OptionValueError),
}

impl fmt::Display for WorkerPoolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) => write!(formatter, "invalid graph worker job: {message}"),
            Self::Scheduler(error) => write!(formatter, "graph worker schedule failed: {error}"),
            Self::Graph(error) => write!(formatter, "graph worker state failed: {error}"),
            Self::Action(error) => write!(formatter, "graph worker action failed: {error}"),
        }
    }
}

impl Error for WorkerPoolError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Scheduler(error) => Some(error),
            Self::Graph(error) => Some(error),
            Self::Action(error) => Some(error),
            Self::Invalid(_) => None,
        }
    }
}

impl From<StateGraphError> for WorkerPoolError {
    fn from(value: StateGraphError) -> Self {
        Self::Graph(value)
    }
}

impl From<dusklight_learning::option_values::OptionValueError> for WorkerPoolError {
    fn from(value: dusklight_learning::option_values::OptionValueError) -> Self {
        Self::Action(value)
    }
}
