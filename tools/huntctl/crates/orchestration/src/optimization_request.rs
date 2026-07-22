//! Sealed request boundary for resumable route optimization campaigns.

use dusklight_automation_contracts::artifact::Digest;
use dusklight_automation_contracts::tape::InputTape;
use dusklight_harness_contracts::objective_suite::{
    ArtifactReference, ObjectiveSeed, SchemaIdentity,
};
use dusklight_harness_contracts::run_contract::{HarnessFidelityMode, HarnessRunRequest};
use dusklight_learning::factorized_pad_action::ONLINE_FACTORIZED_PAD_ACTION_SCHEMA_SHA256;
use dusklight_routes::timeline::{ArtifactSource, Timeline};
use dusklight_routes::timeline_materialization::materialize_segment_chain;
use dusklight_search::residual_action::{
    RESIDUAL_PROPOSAL_SCHEMA_ID_V1, residual_proposal_schema_sha256,
};
use dusklight_search::residual_optimizer::ResidualSearchSpace;
use dusklight_search::residual_retention::{FailureRetentionPolicy, ResidualRetentionConfig};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};

pub const OPTIMIZATION_REQUEST_SCHEMA_V1: &str = "dusklight-optimization-request/v1";
const MAX_EXPLORATION_HORIZON_TICKS: u64 = 10_000_000;
const MAX_CANDIDATES: u64 = 10_000_000;
const MAX_SIMULATED_TICKS: u64 = 1_000_000_000_000;
const MAX_WORKERS: u16 = 256;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OptimizationRequest {
    pub schema: String,
    pub content_sha256: Digest,
    pub id: String,
    pub route: RouteOptimizationBinding,
    pub terminal_predicate: TerminalPredicateBinding,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incumbent: Option<OptimizationIncumbent>,
    pub budgets: OptimizationBudgets,
    pub execution: OptimizationExecution,
    pub proposal: OptimizationProposal,
    pub resume: OptimizationResume,
    pub retention: OptimizationRetention,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RouteOptimizationBinding {
    pub timeline: ArtifactReference,
    pub lineage: String,
    pub segment: String,
    pub source_boundary_index: u64,
    pub source_boundary_fingerprint: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalPredicateBinding {
    pub goal: String,
    pub source: ArtifactReference,
    pub program_sha256: Digest,
    pub definition_sha256: Digest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OptimizationIncumbent {
    pub tape: ArtifactReference,
    pub first_hit_tick: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OptimizationBudgets {
    pub exploration_horizon_ticks: u64,
    pub promotion_before_tick: u64,
    pub candidate_budget: u64,
    pub simulated_tick_budget: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OptimizationExecution {
    pub workers: u16,
    pub deterministic_seeds: Vec<u64>,
    pub repetitions: u16,
    pub fidelity: HarnessFidelityMode,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OptimizationProposal {
    pub action_schema: SchemaIdentity,
    pub proposal_schema: SchemaIdentity,
    pub search_space: ResidualSearchSpace,
    pub optimizer: ResidualOptimizerConfig,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResidualOptimizerConfig {
    Random {
        samples: u64,
    },
    Cem {
        population: u32,
        elites: u32,
        generations: u32,
        smoothing_millionths: u32,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OptimizationResume {
    pub state_path: String,
    pub journal_path: String,
    pub checkpoint_every_candidates: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FailedEpisodeRetention {
    None,
    DiversityReservoir,
    All,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OptimizationRetention {
    pub retain_all_successes: bool,
    pub failed_episodes: FailedEpisodeRetention,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed_episode_limit: Option<u64>,
    pub retain_realized_tapes: bool,
    pub retain_native_episode_shards: bool,
    pub retain_gameplay_traces: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OptimizationRequestValidationReport {
    pub schema: &'static str,
    pub request_id: String,
    pub request_sha256: Digest,
    pub timeline_sha256: Digest,
    pub lineage: String,
    pub segment: String,
    pub source_boundary_index: u64,
    pub source_boundary_fingerprint: String,
    pub terminal_goal: String,
    pub incumbent_first_hit_tick: Option<u64>,
    pub exploration_horizon_ticks: u64,
    pub promotion_before_tick: u64,
    pub candidate_budget: u64,
    pub simulated_tick_budget: u64,
    pub search_space_sha256: Digest,
    pub workers: u16,
    pub repetitions: u16,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OptimizationHarnessBindingReport {
    pub schema: &'static str,
    pub optimization_request_sha256: Digest,
    pub harness_template_sha256: Digest,
    pub terminal_goal: String,
    pub incumbent_tape_sha256: Digest,
    pub exploration_horizon_ticks: u64,
    pub fidelity: HarnessFidelityMode,
}

impl OptimizationRequest {
    pub fn validate(&self) -> Result<(), OptimizationRequestError> {
        if self.schema != OPTIMIZATION_REQUEST_SCHEMA_V1 {
            return Err(request_error("unsupported optimization-request schema"));
        }
        validate_id("request id", &self.id)?;
        validate_id("lineage", &self.route.lineage)?;
        validate_id("segment", &self.route.segment)?;
        validate_id("terminal goal", &self.terminal_predicate.goal)?;
        validate_artifact_shape("timeline", &self.route.timeline)?;
        validate_artifact_shape("terminal predicate", &self.terminal_predicate.source)?;
        if let Some(incumbent) = &self.incumbent {
            validate_artifact_shape("incumbent tape", &incumbent.tape)?;
            if incumbent.first_hit_tick == 0 {
                return Err(request_error("incumbent first-hit tick must be positive"));
            }
        }
        if !is_lower_hex(&self.route.source_boundary_fingerprint, 32)
            || self.terminal_predicate.program_sha256 == Digest::ZERO
            || self.terminal_predicate.definition_sha256 == Digest::ZERO
        {
            return Err(request_error(
                "route boundary and terminal predicate identities must be complete",
            ));
        }
        if self.budgets.promotion_before_tick == 0
            || self.budgets.exploration_horizon_ticks <= self.budgets.promotion_before_tick
            || self.budgets.exploration_horizon_ticks > MAX_EXPLORATION_HORIZON_TICKS
            || self.budgets.candidate_budget == 0
            || self.budgets.candidate_budget > MAX_CANDIDATES
            || self.budgets.simulated_tick_budget < self.budgets.exploration_horizon_ticks
            || self.budgets.simulated_tick_budget > MAX_SIMULATED_TICKS
        {
            return Err(request_error(
                "optimization budgets require a positive promotion threshold, a strictly larger exploration horizon, and nonzero candidate/tick budgets",
            ));
        }
        if let Some(incumbent) = &self.incumbent
            && incumbent.first_hit_tick != self.budgets.promotion_before_tick
        {
            return Err(request_error(
                "supplied incumbent first-hit tick must equal the strict promotion threshold",
            ));
        }
        if self.execution.workers == 0
            || self.execution.workers > MAX_WORKERS
            || self.execution.repetitions == 0
            || self.execution.repetitions > 100
            || self.execution.deterministic_seeds.len() != usize::from(self.execution.workers)
            || !strictly_increasing(&self.execution.deterministic_seeds)
        {
            return Err(request_error(
                "execution requires 1..=100 repetitions and one unique sorted deterministic seed per worker",
            ));
        }
        validate_schema("action", &self.proposal.action_schema)?;
        validate_schema("proposal", &self.proposal.proposal_schema)?;
        if self.proposal.action_schema.id != "dusklight-raw-pad-action/v2"
            || self.proposal.action_schema.sha256
                != Digest(ONLINE_FACTORIZED_PAD_ACTION_SCHEMA_SHA256)
            || self.proposal.proposal_schema.id != RESIDUAL_PROPOSAL_SCHEMA_ID_V1
            || self.proposal.proposal_schema.sha256 != residual_proposal_schema_sha256()
        {
            return Err(request_error(
                "optimization action or residual proposal schema is detached from the implemented raw-PAD compiler",
            ));
        }
        self.proposal
            .search_space
            .validate()
            .map_err(|source| request_error(source.to_string()))?;
        match self.proposal.optimizer {
            ResidualOptimizerConfig::Random { samples }
                if samples == 0 || samples > self.budgets.candidate_budget =>
            {
                return Err(request_error(
                    "random optimizer samples must fit the candidate budget",
                ));
            }
            ResidualOptimizerConfig::Cem {
                population,
                elites,
                generations,
                smoothing_millionths,
            } if !(2..=16_384).contains(&population)
                || elites == 0
                || elites >= population
                || generations == 0
                || smoothing_millionths == 0
                || smoothing_millionths > 1_000_000
                || u64::from(population) * u64::from(generations)
                    > self.budgets.candidate_budget =>
            {
                return Err(request_error(
                    "CEM population, elites, generations, and smoothing must fit the implemented optimizer and candidate budget",
                ));
            }
            _ => {}
        }
        validate_build_path("resume state", &self.resume.state_path)?;
        validate_build_path("resume journal", &self.resume.journal_path)?;
        if self.resume.state_path == self.resume.journal_path
            || self.resume.checkpoint_every_candidates == 0
            || self.resume.checkpoint_every_candidates > self.budgets.candidate_budget
        {
            return Err(request_error(
                "resume state and journal must be distinct and checkpoint within the candidate budget",
            ));
        }
        if !self.retention.retain_all_successes
            || !self.retention.retain_realized_tapes
            || !self.retention.retain_native_episode_shards
        {
            return Err(request_error(
                "campaigns must retain all successes, realized tapes, and native episode shards",
            ));
        }
        match (
            self.retention.failed_episodes,
            self.retention.failed_episode_limit,
        ) {
            (FailedEpisodeRetention::None, _)
            | (FailedEpisodeRetention::All, Some(_))
            | (FailedEpisodeRetention::DiversityReservoir, None | Some(0)) => {
                return Err(request_error(
                    "failures must be retained either completely or in a positive diversity reservoir",
                ));
            }
            _ => {}
        }
        if self.content_sha256 == Digest::ZERO
            || self.content_sha256 != self.compute_content_sha256()?
        {
            return Err(request_error(
                "optimization-request content identity is invalid",
            ));
        }
        Ok(())
    }

    pub fn validate_files(
        &self,
        repository_root: &Path,
    ) -> Result<OptimizationRequestValidationReport, OptimizationRequestError> {
        self.validate()?;
        let root = repository_root.canonicalize().map_err(|source| {
            request_error(format!(
                "cannot resolve repository root {}: {source}",
                repository_root.display()
            ))
        })?;
        let timeline_path = validate_artifact_file(&root, "timeline", &self.route.timeline)?;
        let source = fs::read_to_string(&timeline_path)
            .map_err(|source| request_error(format!("cannot read timeline: {source}")))?;
        let timeline = Timeline::parse(&source)
            .map_err(|source| request_error(format!("invalid timeline: {source}")))?;
        timeline
            .validate_artifacts(timeline_path.parent())
            .map_err(|source| request_error(format!("invalid timeline artifacts: {source}")))?;
        let inspection = timeline
            .inspect()
            .map_err(|source| request_error(format!("invalid timeline lineage: {source}")))?;
        let lineage = inspection
            .lineages
            .iter()
            .find(|lineage| lineage.name == self.route.lineage)
            .ok_or_else(|| request_error("optimization lineage is absent from the timeline"))?;
        if !lineage
            .steps
            .iter()
            .any(|step| step.segment == self.route.segment)
        {
            return Err(request_error(
                "optimization segment is not part of the selected lineage",
            ));
        }
        let segment = timeline
            .segments
            .get(&self.route.segment)
            .ok_or_else(|| request_error("optimization segment is absent from the timeline"))?;
        if segment.start_fingerprint != self.route.source_boundary_fingerprint {
            return Err(request_error(
                "optimization source boundary differs from the selected segment",
            ));
        }
        if self.incumbent.is_some() {
            let parent_id = segment.parent.as_deref().ok_or_else(|| {
                request_error("residual incumbent segment has no parent source boundary")
            })?;
            let artifact_root = timeline_path
                .parent()
                .ok_or_else(|| request_error("optimization timeline has no artifact root"))?;
            let parent = materialize_segment_chain(&timeline, artifact_root, parent_id)
                .map_err(|source| request_error(source.to_string()))?;
            if parent.tape.frames.len() as u64 != self.route.source_boundary_index {
                return Err(request_error(
                    "optimization source boundary index differs from the exact materialized parent checkpoint",
                ));
            }
        }

        let goal = timeline
            .goals
            .get(&self.terminal_predicate.goal)
            .ok_or_else(|| request_error("terminal goal is absent from the timeline"))?;
        if goal.segment != self.route.segment {
            return Err(request_error(
                "terminal goal does not belong to the selected optimization segment",
            ));
        }
        let goal_source = goal
            .predicate_source
            .as_ref()
            .ok_or_else(|| request_error("terminal goal does not own a predicate source"))?;
        let expected_goal_source = timeline_path
            .parent()
            .unwrap_or(&root)
            .join(goal_source)
            .canonicalize()
            .map_err(|source| request_error(format!("cannot resolve terminal source: {source}")))?;
        let actual_goal_source =
            validate_artifact_file(&root, "terminal predicate", &self.terminal_predicate.source)?;
        if actual_goal_source != expected_goal_source {
            return Err(request_error(
                "terminal predicate source differs from the timeline-owned goal source",
            ));
        }
        let proof = timeline
            .proofs
            .iter()
            .find(|proof| {
                proof.segment == self.route.segment && proof.goal == self.terminal_predicate.goal
            })
            .ok_or_else(|| request_error("incumbent segment lacks the selected terminal proof"))?;
        if proof.predicate_program_sha256 != self.terminal_predicate.program_sha256.to_string()
            || proof.predicate_definition_sha256
                != self.terminal_predicate.definition_sha256.to_string()
        {
            return Err(request_error(
                "terminal predicate identities differ from the timeline proof",
            ));
        }

        if let Some(incumbent) = &self.incumbent {
            let incumbent_path = validate_artifact_file(&root, "incumbent tape", &incumbent.tape)?;
            let ArtifactSource::Tape(segment_tape) = &segment.artifact else {
                return Err(request_error(
                    "supplied incumbent requires a tape-backed timeline segment",
                ));
            };
            let expected_tape = timeline_path
                .parent()
                .unwrap_or(&root)
                .join(segment_tape)
                .canonicalize()
                .map_err(|source| {
                    request_error(format!("cannot resolve segment tape: {source}"))
                })?;
            if incumbent_path != expected_tape
                || proof.first_hit_tick != Some(incumbent.first_hit_tick)
            {
                return Err(request_error(
                    "incumbent tape or first-hit tick differs from the timeline proof",
                ));
            }
            let incumbent_bytes = fs::read(&incumbent_path)
                .map_err(|source| request_error(format!("cannot read incumbent tape: {source}")))?;
            let incumbent_tape = InputTape::decode(&incumbent_bytes)
                .map_err(|source| request_error(format!("cannot decode incumbent tape: {source}")))?
                .tape;
            self.proposal
                .search_space
                .validate_parent(&incumbent_tape)
                .map_err(|source| request_error(source.to_string()))?;
        }

        Ok(OptimizationRequestValidationReport {
            schema: OPTIMIZATION_REQUEST_SCHEMA_V1,
            request_id: self.id.clone(),
            request_sha256: self.content_sha256,
            timeline_sha256: self.route.timeline.sha256,
            lineage: self.route.lineage.clone(),
            segment: self.route.segment.clone(),
            source_boundary_index: self.route.source_boundary_index,
            source_boundary_fingerprint: self.route.source_boundary_fingerprint.clone(),
            terminal_goal: self.terminal_predicate.goal.clone(),
            incumbent_first_hit_tick: self.incumbent.as_ref().map(|value| value.first_hit_tick),
            exploration_horizon_ticks: self.budgets.exploration_horizon_ticks,
            promotion_before_tick: self.budgets.promotion_before_tick,
            candidate_budget: self.budgets.candidate_budget,
            simulated_tick_budget: self.budgets.simulated_tick_budget,
            search_space_sha256: self
                .proposal
                .search_space
                .sha256()
                .map_err(|source| request_error(source.to_string()))?,
            workers: self.execution.workers,
            repetitions: self.execution.repetitions,
        })
    }

    pub fn refresh_content_sha256(&mut self) -> Result<(), OptimizationRequestError> {
        self.content_sha256 = Digest::ZERO;
        self.content_sha256 = self.compute_content_sha256()?;
        Ok(())
    }

    pub fn residual_retention_config(
        &self,
    ) -> Result<ResidualRetentionConfig, OptimizationRequestError> {
        self.validate()?;
        let incumbent = self
            .incumbent
            .as_ref()
            .ok_or_else(|| request_error("residual retention requires an incumbent parent tape"))?;
        let failures = match (
            self.retention.failed_episodes,
            self.retention.failed_episode_limit,
        ) {
            (FailedEpisodeRetention::DiversityReservoir, Some(capacity)) => {
                FailureRetentionPolicy::DiversityReservoir { capacity }
            }
            (FailedEpisodeRetention::All, None) => FailureRetentionPolicy::All,
            _ => {
                return Err(request_error(
                    "optimization failure retention cannot map to residual retention",
                ));
            }
        };
        let config = ResidualRetentionConfig {
            parent_tape_sha256: incumbent.tape.sha256,
            terminal_program_sha256: self.terminal_predicate.program_sha256,
            terminal_definition_sha256: self.terminal_predicate.definition_sha256,
            exploration_horizon_ticks: self.budgets.exploration_horizon_ticks,
            promotion_before_tick: self.budgets.promotion_before_tick,
            maximum_candidates: self.budgets.candidate_budget,
            failures,
        };
        config
            .validate()
            .map_err(|source| request_error(source.to_string()))?;
        Ok(config)
    }

    /// Verifies that a separately sealed native harness template is an exact
    /// execution authority for this optimization request. Candidate tape,
    /// deterministic seed, and artifact destination are the only fields the
    /// evaluator may subsequently specialize.
    pub fn validate_harness_template(
        &self,
        repository_root: &Path,
        template: &HarnessRunRequest,
    ) -> Result<OptimizationHarnessBindingReport, OptimizationRequestError> {
        self.validate_files(repository_root)?;
        template
            .validate_files(repository_root)
            .map_err(|source| request_error(format!("invalid harness template: {source}")))?;
        let incumbent = self
            .incumbent
            .as_ref()
            .ok_or_else(|| request_error("residual execution requires an incumbent tape"))?;
        let ObjectiveSeed::Tape { artifact } = &template.input else {
            return Err(request_error(
                "residual harness template input must be the incumbent tape",
            ));
        };
        if artifact != &incumbent.tape
            || template.identity.content_digest != incumbent.tape.sha256
            || template.objective.goal != self.terminal_predicate.goal
            || template.objective.source != self.terminal_predicate.source
            || template.objective.program_sha256 != self.terminal_predicate.program_sha256
            || template.action_schema != self.proposal.action_schema
            || template.logical_tick_budget != self.budgets.exploration_horizon_ticks
            || template.fidelity != self.execution.fidelity
        {
            return Err(request_error(
                "harness template differs from the optimization incumbent, terminal predicate, action schema, horizon, or fidelity",
            ));
        }
        Ok(OptimizationHarnessBindingReport {
            schema: "dusklight-optimization-harness-binding/v1",
            optimization_request_sha256: self.content_sha256,
            harness_template_sha256: template.content_sha256,
            terminal_goal: self.terminal_predicate.goal.clone(),
            incumbent_tape_sha256: incumbent.tape.sha256,
            exploration_horizon_ticks: self.budgets.exploration_horizon_ticks,
            fidelity: self.execution.fidelity,
        })
    }

    pub fn to_pretty_json(&self) -> Result<Vec<u8>, OptimizationRequestError> {
        let mut bytes =
            serde_json::to_vec_pretty(self).map_err(|source| request_error(source.to_string()))?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    fn compute_content_sha256(&self) -> Result<Digest, OptimizationRequestError> {
        let mut canonical = self.clone();
        canonical.content_sha256 = Digest::ZERO;
        let bytes =
            serde_json::to_vec(&canonical).map_err(|source| request_error(source.to_string()))?;
        let mut hasher = Sha256::new();
        hasher.update(b"dusklight.optimization-request/v1\0");
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
        Ok(Digest(hasher.finalize().into()))
    }
}

fn validate_artifact_shape(
    label: &str,
    artifact: &ArtifactReference,
) -> Result<(), OptimizationRequestError> {
    validate_relative_path(label, &artifact.path)?;
    if artifact.sha256 == Digest::ZERO {
        return Err(request_error(format!("{label} digest must be nonzero")));
    }
    Ok(())
}

fn validate_artifact_file(
    root: &Path,
    label: &str,
    artifact: &ArtifactReference,
) -> Result<PathBuf, OptimizationRequestError> {
    validate_artifact_shape(label, artifact)?;
    let path = root.join(&artifact.path).canonicalize().map_err(|source| {
        request_error(format!(
            "cannot resolve {label} {}: {source}",
            artifact.path
        ))
    })?;
    if !path.starts_with(root) || !path.is_file() {
        return Err(request_error(format!(
            "{label} must resolve to a file within the repository"
        )));
    }
    let actual = sha256(
        &fs::read(&path)
            .map_err(|source| request_error(format!("cannot read {label}: {source}")))?,
    );
    if actual != artifact.sha256 {
        return Err(request_error(format!("{label} content digest differs")));
    }
    Ok(path)
}

fn validate_schema(label: &str, schema: &SchemaIdentity) -> Result<(), OptimizationRequestError> {
    validate_id(&format!("{label} schema"), &schema.id)?;
    if schema.sha256 == Digest::ZERO {
        return Err(request_error(format!(
            "{label} schema digest must be nonzero"
        )));
    }
    Ok(())
}

fn validate_id(label: &str, value: &str) -> Result<(), OptimizationRequestError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/'))
    {
        return Err(request_error(format!("{label} is invalid")));
    }
    Ok(())
}

fn validate_relative_path(label: &str, value: &str) -> Result<(), OptimizationRequestError> {
    let path = Path::new(value);
    if value.is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(request_error(format!(
            "{label} must be a canonical repository-relative path"
        )));
    }
    Ok(())
}

fn validate_build_path(label: &str, value: &str) -> Result<(), OptimizationRequestError> {
    validate_relative_path(label, value)?;
    if !Path::new(value).starts_with("build") {
        return Err(request_error(format!("{label} must be beneath build/")));
    }
    Ok(())
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn strictly_increasing(values: &[u64]) -> bool {
    !values.is_empty() && values.windows(2).all(|pair| pair[0] < pair[1])
}

fn sha256(bytes: &[u8]) -> Digest {
    Digest(Sha256::digest(bytes).into())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OptimizationRequestError(String);

fn request_error(message: impl Into<String>) -> OptimizationRequestError {
    OptimizationRequestError(message.into())
}

impl fmt::Display for OptimizationRequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for OptimizationRequestError {}
