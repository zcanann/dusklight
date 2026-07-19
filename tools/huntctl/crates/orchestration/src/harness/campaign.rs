//! Planning and execution for the top-level objective campaign command.

use super::execution::execute_request;
use super::objective_suite::{
    ExpectedTerminalClass, ObjectiveBoot, ObjectiveCaseRole, ObjectiveSeed, ObjectiveSuite,
};
use super::run_contract::{
    HarnessBoundaryFingerprint, HarnessRunRequest, HarnessRunResult, HarnessTerminalReason,
};
use crate::artifact::Digest;
use crate::compatibility::{CompatibilityMode, ensure_compatible};
use crate::search::LexicographicScore;
use crate::search_evaluator::{
    HarnessEvaluateConfig, ProposerReplayVerdict, ProposerTournamentConfig, TournamentDefinition,
    TournamentProposerKind, derive_candidate_request,
};
use crate::run_proposer_tournament;
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

pub const CAMPAIGN_PLAN_SCHEMA_V1: &str = "dusklight-campaign-plan/v1";

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CampaignProposer {
    Scripted,
    Random,
    Structured,
    Learned,
}

impl FromStr for CampaignProposer {
    type Err = CampaignPlanError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "scripted" => Ok(Self::Scripted),
            "random" => Ok(Self::Random),
            "structured" => Ok(Self::Structured),
            "learned" => Ok(Self::Learned),
            _ => Err(plan_error(format!(
                "unknown proposer {value:?}; expected scripted, random, structured, or learned"
            ))),
        }
    }
}

pub struct CampaignPlanConfig<'a> {
    pub repository_root: &'a Path,
    pub suite_path: &'a Path,
    pub case_id: &'a str,
    pub output_root: &'a Path,
    pub proposers: &'a [CampaignProposer],
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CampaignPlan {
    pub schema: &'static str,
    pub dry_run: bool,
    pub suite_id: String,
    pub suite_sha256: Digest,
    pub case_id: String,
    pub case_role: ObjectiveCaseRole,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub control_for: Option<String>,
    pub expected_terminal: ExpectedTerminalClass,
    pub proposers: Vec<CampaignProposer>,
    pub resolved_paths: CampaignResolvedPaths,
    pub identities: CampaignIdentities,
    pub required_facts: Vec<String>,
    pub required_capabilities: Vec<String>,
    pub budgets: CampaignBudgets,
    pub outputs: CampaignOutputs,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CampaignResolvedPaths {
    pub repository_root: PathBuf,
    pub suite: PathBuf,
    pub scenario: PathBuf,
    pub objective: PathBuf,
    pub observation_view: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed_input: Option<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CampaignIdentities {
    pub scenario_sha256: Digest,
    pub objective_source_sha256: Digest,
    pub objective_program_sha256: Digest,
    pub observation_source_sha256: Digest,
    pub observation_schema_sha256: Digest,
    pub action_schema_id: String,
    pub action_schema_sha256: Digest,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed_input_sha256: Option<Digest>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CampaignBudgets {
    pub logical_ticks_per_episode: u64,
    pub host_timeout_seconds: u32,
    pub repetitions: u16,
    pub selected_proposers: u64,
    pub planned_episodes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CampaignOutputs {
    pub root: PathBuf,
    pub available: bool,
    pub requests: PathBuf,
    pub episodes: PathBuf,
    pub finalists: PathBuf,
    pub replays: PathBuf,
    pub report: PathBuf,
}

pub struct CampaignRunConfig<'a> {
    pub plan: CampaignPlanConfig<'a>,
    pub request_template_path: &'a Path,
    pub tournament_definition_path: &'a Path,
    pub workers: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CampaignReplayVerdict {
    Proved,
    ObjectiveMiss,
    Failed,
    Nondeterministic,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CampaignReportRow {
    pub proposer: String,
    pub proposer_kind: CampaignProposer,
    pub selected_candidates: usize,
    pub charged_episodes: u64,
    pub charged_candidate_ticks: u64,
    pub objective_hits: usize,
    pub useful_boundary_states: usize,
    pub tournament_replay_verdict: ProposerReplayVerdict,
    pub cold_replay_verdict: CampaignReplayVerdict,
    pub cold_replay_attempts: u16,
    pub best_candidate_id: String,
    pub best_score: LexicographicScore,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub best_proved_tape: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub best_proved_tape_sha256: Option<Digest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_hit_tick: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub boundary_fingerprint: Option<HarnessBoundaryFingerprint>,
    pub replay_results: Vec<PathBuf>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CampaignReport {
    pub schema: &'static str,
    pub passed: bool,
    pub plan: CampaignPlan,
    pub request_template: PathBuf,
    pub request_template_sha256: Digest,
    pub materialized_request: PathBuf,
    pub materialized_request_sha256: Digest,
    pub tournament_definition: PathBuf,
    pub tournament_definition_sha256: Digest,
    pub tournament_summary: PathBuf,
    pub tournament_summary_sha256: Digest,
    pub observed_terminal_classes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_blocker: Option<CampaignBlocker>,
    pub rows: Vec<CampaignReportRow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub winner_proposer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub winner_tape: Option<PathBuf>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CampaignBlocker {
    pub terminal: HarnessTerminalReason,
    pub kind: String,
    pub value: String,
    pub message: String,
    pub artifact: PathBuf,
}

pub fn campaign_proposers_from_definition(
    definition: &TournamentDefinition,
) -> Result<Vec<CampaignProposer>, CampaignPlanError> {
    let proposers = definition
        .proposers
        .iter()
        .map(|proposer| proposer_kind(proposer.kind))
        .collect::<BTreeSet<_>>();
    if proposers.len() != definition.proposers.len() {
        return Err(plan_error(
            "campaign tournament must contain at most one lane for each proposer kind",
        ));
    }
    Ok(proposers.into_iter().collect())
}

pub fn resolve_campaign_plan(
    config: &CampaignPlanConfig<'_>,
) -> Result<CampaignPlan, CampaignPlanError> {
    let repository_root = config.repository_root.canonicalize().map_err(|error| {
        plan_error(format!(
            "cannot resolve repository root {}: {error}",
            config.repository_root.display()
        ))
    })?;
    let suite_path = resolve_existing_file(&repository_root, config.suite_path, "suite")?;
    let suite: ObjectiveSuite = serde_json::from_slice(
        &fs::read(&suite_path)
            .map_err(|error| plan_error(format!("cannot read objective suite: {error}")))?,
    )
    .map_err(|error| plan_error(format!("cannot decode objective suite: {error}")))?;
    suite
        .validate_files(&repository_root)
        .map_err(|error| plan_error(error.to_string()))?;
    let case = suite
        .cases
        .iter()
        .find(|case| case.id == config.case_id)
        .ok_or_else(|| plan_error(format!("suite has no case {:?}", config.case_id)))?;

    if config.proposers.is_empty() {
        return Err(plan_error("campaign requires at least one proposer"));
    }
    let proposers = config.proposers.iter().copied().collect::<BTreeSet<_>>();
    if proposers.len() != config.proposers.len() {
        return Err(plan_error("campaign proposers must be unique"));
    }
    let proposers = proposers.into_iter().collect::<Vec<_>>();

    let relative_output = canonical_relative_output(config.output_root)?;
    let output_root = repository_root.join(relative_output);
    let output_available = !output_root.exists();
    let seed = seed_artifact(&case.seed);
    let seed_input = seed
        .map(|artifact| resolve_existing_file(&repository_root, Path::new(&artifact.path), "seed"))
        .transpose()?;
    let required_capabilities = required_capabilities(case);
    let selected_proposers = u64::try_from(proposers.len()).unwrap_or(u64::MAX);
    let planned_episodes = selected_proposers.saturating_mul(u64::from(case.repetitions));

    Ok(CampaignPlan {
        schema: CAMPAIGN_PLAN_SCHEMA_V1,
        dry_run: true,
        suite_id: suite.id,
        suite_sha256: suite.content_sha256,
        case_id: case.id.clone(),
        case_role: case.role,
        control_for: case.control_for.clone(),
        expected_terminal: case.expected_terminal,
        proposers,
        resolved_paths: CampaignResolvedPaths {
            repository_root: repository_root.clone(),
            suite: suite_path,
            scenario: resolve_existing_file(
                &repository_root,
                Path::new(&case.scenario.path),
                "scenario",
            )?,
            objective: resolve_existing_file(
                &repository_root,
                Path::new(&case.objective.source.path),
                "objective",
            )?,
            observation_view: resolve_existing_file(
                &repository_root,
                Path::new(&case.observation_view.source.path),
                "observation view",
            )?,
            seed_input,
        },
        identities: CampaignIdentities {
            scenario_sha256: case.scenario.sha256,
            objective_source_sha256: case.objective.source.sha256,
            objective_program_sha256: case.objective.program_sha256,
            observation_source_sha256: case.observation_view.source.sha256,
            observation_schema_sha256: case.observation_view.schema_sha256,
            action_schema_id: case.action_schema.id.clone(),
            action_schema_sha256: case.action_schema.sha256,
            seed_input_sha256: seed.map(|artifact| artifact.sha256),
        },
        required_facts: case.observation_requirements.facts.clone(),
        required_capabilities,
        budgets: CampaignBudgets {
            logical_ticks_per_episode: case.logical_tick_budget,
            host_timeout_seconds: case.host_timeout_seconds,
            repetitions: case.repetitions,
            selected_proposers,
            planned_episodes,
        },
        outputs: CampaignOutputs {
            root: output_root.clone(),
            available: output_available,
            requests: output_root.join("requests"),
            episodes: output_root.join("evaluations"),
            finalists: output_root.join("finalists"),
            replays: output_root.join("replays"),
            report: output_root.join("report.json"),
        },
    })
}

pub fn run_campaign(config: &CampaignRunConfig<'_>) -> Result<CampaignReport, CampaignPlanError> {
    if config.workers == 0 {
        return Err(plan_error("campaign workers must be greater than zero"));
    }
    let mut plan = resolve_campaign_plan(&config.plan)?;
    if !plan.outputs.available {
        return Err(plan_error(format!(
            "campaign output already exists: {}",
            plan.outputs.root.display()
        )));
    }
    let repository_root = plan.resolved_paths.repository_root.clone();
    let suite: ObjectiveSuite = read_json(&plan.resolved_paths.suite, "objective suite")?;
    let case = suite
        .cases
        .iter()
        .find(|case| case.id == plan.case_id)
        .ok_or_else(|| plan_error("resolved campaign case disappeared from its suite"))?;
    if !matches!(
        case.expected_terminal,
        ExpectedTerminalClass::Reached | ExpectedTerminalClass::ObjectiveMiss
    ) {
        return Err(plan_error(
            "executing campaigns currently require reached or objective_miss terminal classes",
        ));
    }

    let request_template_path = resolve_existing_file(
        &repository_root,
        config.request_template_path,
        "run request template",
    )?;
    let mut request: HarnessRunRequest = read_json(&request_template_path, "run request template")?;
    request
        .validate_files(&repository_root)
        .map_err(|error| plan_error(format!("invalid run request template: {error}")))?;
    let source_request_sha256 = request.content_sha256;
    request.id = format!("campaign-{}", case.id);
    request.boot = case.boot.clone();
    request.scenario = case.scenario.clone();
    request.objective = case.objective.clone();
    request.observation_view = case.observation_view.clone();
    request.action_schema = case.action_schema.clone();
    request.observation_requirements = case.observation_requirements.clone();
    request.input = case.seed.clone();
    request.logical_tick_budget = case.logical_tick_budget;
    request.host_timeout_seconds = case.host_timeout_seconds;
    request.identity.scenario_id = case.id.clone();
    request.identity.scenario_digest = case.scenario.sha256;
    request.identity.predicate_program_digest = case.objective.program_sha256;
    request.identity.observation_schema_digest = case.observation_view.schema_sha256;
    request.identity.action_schema_digest = case.action_schema.sha256;
    request.artifact_destination = repository_relative_string(
        &repository_root,
        &plan.outputs.requests.join("template"),
        "campaign request destination",
    )?;
    request.content_sha256 = Digest::ZERO;
    request
        .refresh_content_sha256()
        .map_err(|error| plan_error(format!("cannot seal campaign request: {error}")))?;
    request.validate_files(&repository_root).map_err(|error| {
        plan_error(format!("materialized campaign request is invalid: {error}"))
    })?;

    let definition_path = resolve_existing_file(
        &repository_root,
        config.tournament_definition_path,
        "tournament definition",
    )?;
    let definition_bytes = fs::read(&definition_path)
        .map_err(|error| plan_error(format!("cannot read tournament definition: {error}")))?;
    let definition: TournamentDefinition = serde_json::from_slice(&definition_bytes)
        .map_err(|error| plan_error(format!("cannot decode tournament definition: {error}")))?;
    let definition_proposers = campaign_proposers_from_definition(&definition)?;
    if definition_proposers != plan.proposers {
        return Err(plan_error(
            "campaign plan proposers do not match the tournament definition",
        ));
    }
    let definition_directory = definition_path
        .parent()
        .ok_or_else(|| plan_error("tournament definition has no parent directory"))?
        .to_path_buf();
    let game = repository_root.join(&request.executable.path);
    let dvd = repository_root.join(&request.game_data.path);
    let tournament = run_proposer_tournament(&ProposerTournamentConfig {
        definition,
        definition_directory,
        game,
        dvd,
        output_root: plan.outputs.root.clone(),
        working_directory: repository_root.clone(),
        game_args_prefix: Vec::new(),
        workers: config.workers,
        repetitions: u32::from(case.repetitions),
        timeout: Duration::from_secs(u64::from(case.host_timeout_seconds)),
        harness: Some(HarnessEvaluateConfig {
            repository_root: repository_root.clone(),
            request_template: request.clone(),
        }),
        anchored: None,
    })
    .map_err(|error| plan_error(format!("campaign tournament failed: {error}")))?;

    fs::create_dir_all(&plan.outputs.requests).map_err(|error| {
        plan_error(format!("cannot create campaign request directory: {error}"))
    })?;
    let materialized_request_path = plan.outputs.requests.join("template.json");
    fs::write(
        &materialized_request_path,
        request
            .to_pretty_json()
            .map_err(|error| plan_error(error.to_string()))?,
    )
    .map_err(|error| plan_error(format!("cannot write campaign request template: {error}")))?;

    let mut rows = Vec::with_capacity(tournament.rows.len());
    for tournament_row in &tournament.rows {
        let proposer_kind = proposer_kind(tournament_row.kind);
        let mut replay_results = Vec::new();
        let mut proofs = Vec::new();
        if let Some(tape_path) = tournament_row.best_proved_tape.as_ref() {
            for repetition in 1..=case.repetitions {
                let artifact_root = plan
                    .outputs
                    .replays
                    .join(&tournament_row.name)
                    .join(format!("attempt-{repetition:03}"));
                let artifact_destination = repository_relative_string(
                    &repository_root,
                    &artifact_root,
                    "campaign replay destination",
                )?;
                let replay_request = derive_candidate_request(
                    &request,
                    &repository_root,
                    tape_path,
                    &artifact_destination,
                    request.rng_seed,
                )
                .map_err(|error| plan_error(format!("cannot derive replay request: {error}")))?;
                let request_path = plan
                    .outputs
                    .requests
                    .join(&tournament_row.name)
                    .join(format!("replay-{repetition:03}.json"));
                if let Some(parent) = request_path.parent() {
                    fs::create_dir_all(parent).map_err(|error| {
                        plan_error(format!("cannot create replay request directory: {error}"))
                    })?;
                }
                fs::write(
                    &request_path,
                    replay_request
                        .to_pretty_json()
                        .map_err(|error| plan_error(error.to_string()))?,
                )
                .map_err(|error| plan_error(format!("cannot write replay request: {error}")))?;
                let result =
                    execute_request(&replay_request, &repository_root, u32::from(repetition))
                        .map_err(|error| {
                            plan_error(format!("campaign cold replay failed: {error}"))
                        })?;
                ensure_compatible(
                    CompatibilityMode::Replay,
                    &request.identity,
                    &result.identity,
                )
                .map_err(|error| {
                    plan_error(format!("campaign cold replay is incompatible: {error}"))
                })?;
                replay_results.push(artifact_root.join("result.json"));
                proofs.push(ReplayProof::from_result(&result));
            }
        }
        let (cold_replay_verdict, first_hit_tick, boundary_fingerprint) =
            assess_replay_proofs(&proofs, tournament_row.replay_verdict);
        rows.push(CampaignReportRow {
            proposer: tournament_row.name.clone(),
            proposer_kind,
            selected_candidates: tournament_row.selected_candidates,
            charged_episodes: tournament_row.charged_episodes,
            charged_candidate_ticks: tournament_row.charged_candidate_ticks,
            objective_hits: tournament_row.predicate_hits,
            useful_boundary_states: tournament_row.boundary_diversity,
            tournament_replay_verdict: tournament_row.replay_verdict,
            cold_replay_verdict,
            cold_replay_attempts: u16::try_from(proofs.len()).unwrap_or(u16::MAX),
            best_candidate_id: tournament_row.best_candidate_id.clone(),
            best_score: tournament_row.best_score,
            best_proved_tape: tournament_row.best_proved_tape.clone(),
            best_proved_tape_sha256: tournament_row.best_proved_tape_sha256,
            first_hit_tick,
            boundary_fingerprint,
            replay_results,
        });
    }
    let evaluation_path = plan.outputs.root.join("evaluations/evaluation.json");
    let evaluation: serde_json::Value = read_json(&evaluation_path, "campaign evaluation")?;
    let observed_terminal_classes = evaluation["attempts"]
        .as_array()
        .ok_or_else(|| plan_error("campaign evaluation omitted attempts"))?
        .iter()
        .map(|attempt| {
            attempt["harness_terminal"]
                .as_str()
                .ok_or_else(|| plan_error("campaign attempt omitted harness terminal"))
                .map(str::to_owned)
        })
        .collect::<Result<BTreeSet<_>, _>>()?
        .into_iter()
        .collect::<Vec<_>>();
    let first_blocker = first_campaign_blocker(&evaluation, &plan.outputs.root)?;
    let passed = match case.expected_terminal {
        ExpectedTerminalClass::Reached => rows
            .iter()
            .any(|row| row.cold_replay_verdict == CampaignReplayVerdict::Proved),
        ExpectedTerminalClass::ObjectiveMiss => {
            rows.iter()
                .all(|row| row.cold_replay_verdict == CampaignReplayVerdict::ObjectiveMiss)
                && observed_terminal_classes.as_slice() == ["exhausted"]
        }
        ExpectedTerminalClass::Unsupported | ExpectedTerminalClass::Impossible => false,
    };
    let winner = rows
        .iter()
        .filter(|row| row.cold_replay_verdict == CampaignReplayVerdict::Proved)
        .max_by_key(|row| row.best_score)
        .map(|row| (row.proposer.clone(), row.best_proved_tape.clone()));
    let tournament_summary = plan.outputs.root.join("tournament.summary.json");
    let tournament_summary_bytes = fs::read(&tournament_summary)
        .map_err(|error| plan_error(format!("cannot read tournament summary: {error}")))?;
    plan.dry_run = false;
    let report = CampaignReport {
        schema: "dusklight-campaign-report/v1",
        passed,
        plan,
        request_template: request_template_path,
        request_template_sha256: source_request_sha256,
        materialized_request: materialized_request_path,
        materialized_request_sha256: request.content_sha256,
        tournament_definition: definition_path,
        tournament_definition_sha256: Digest(Sha256::digest(&definition_bytes).into()),
        tournament_summary,
        tournament_summary_sha256: Digest(Sha256::digest(&tournament_summary_bytes).into()),
        observed_terminal_classes,
        first_blocker,
        rows,
        winner_proposer: winner.as_ref().map(|(proposer, _)| proposer.clone()),
        winner_tape: winner.and_then(|(_, tape)| tape),
    };
    fs::write(
        &report.plan.outputs.report,
        serde_json::to_vec_pretty(&report)
            .map_err(|error| plan_error(format!("cannot encode campaign report: {error}")))?,
    )
    .map_err(|error| plan_error(format!("cannot write campaign report: {error}")))?;
    Ok(report)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ReplayProof {
    terminal: HarnessTerminalReason,
    reached: bool,
    first_hit_tick: Option<u64>,
    boundary_fingerprint: Option<HarnessBoundaryFingerprint>,
    realized_input_sha256: Option<Digest>,
}

impl ReplayProof {
    fn from_result(result: &super::run_contract::HarnessRunResult) -> Self {
        Self {
            terminal: result.terminal,
            reached: result.objective.reached,
            first_hit_tick: result.objective.first_hit_tick,
            boundary_fingerprint: result.objective.boundary_fingerprint.clone(),
            realized_input_sha256: result
                .artifacts
                .realized_input
                .as_ref()
                .map(|artifact| artifact.sha256),
        }
    }
}

fn assess_replay_proofs(
    proofs: &[ReplayProof],
    tournament_verdict: ProposerReplayVerdict,
) -> (
    CampaignReplayVerdict,
    Option<u64>,
    Option<HarnessBoundaryFingerprint>,
) {
    if proofs.is_empty() {
        return (
            if tournament_verdict == ProposerReplayVerdict::ObjectiveMiss {
                CampaignReplayVerdict::ObjectiveMiss
            } else {
                CampaignReplayVerdict::Failed
            },
            None,
            None,
        );
    }
    let reference = &proofs[0];
    if proofs.iter().skip(1).any(|proof| proof != reference) {
        return (CampaignReplayVerdict::Nondeterministic, None, None);
    }
    if reference.terminal == HarnessTerminalReason::Reached && reference.reached {
        (
            CampaignReplayVerdict::Proved,
            reference.first_hit_tick,
            reference.boundary_fingerprint.clone(),
        )
    } else if !reference.reached {
        (CampaignReplayVerdict::ObjectiveMiss, None, None)
    } else {
        (CampaignReplayVerdict::Failed, None, None)
    }
}

fn proposer_kind(kind: TournamentProposerKind) -> CampaignProposer {
    match kind {
        TournamentProposerKind::IncumbentMutation => CampaignProposer::Scripted,
        TournamentProposerKind::BlindExploration => CampaignProposer::Random,
        TournamentProposerKind::Structured => CampaignProposer::Structured,
        TournamentProposerKind::Learned => CampaignProposer::Learned,
    }
}

fn read_json<T: serde::de::DeserializeOwned>(
    path: &Path,
    label: &str,
) -> Result<T, CampaignPlanError> {
    serde_json::from_slice(
        &fs::read(path).map_err(|error| plan_error(format!("cannot read {label}: {error}")))?,
    )
    .map_err(|error| plan_error(format!("cannot decode {label}: {error}")))
}

fn first_campaign_blocker(
    evaluation: &serde_json::Value,
    campaign_root: &Path,
) -> Result<Option<CampaignBlocker>, CampaignPlanError> {
    let attempts = evaluation["attempts"]
        .as_array()
        .ok_or_else(|| plan_error("campaign evaluation omitted attempts"))?;
    let canonical_root = campaign_root
        .canonicalize()
        .map_err(|error| plan_error(format!("cannot resolve campaign root: {error}")))?;
    for attempt in attempts {
        let Some(terminal_name) = attempt["harness_terminal"].as_str() else {
            continue;
        };
        if !matches!(
            terminal_name,
            "unsupported" | "capability_mismatch" | "identity_mismatch"
        ) {
            continue;
        }
        let artifact = attempt["harness_result"]
            .as_str()
            .ok_or_else(|| plan_error("blocked campaign attempt omitted harness result path"))?;
        let artifact = PathBuf::from(artifact);
        let canonical_artifact = artifact.canonicalize().map_err(|error| {
            plan_error(format!(
                "cannot resolve blocked campaign artifact {}: {error}",
                artifact.display()
            ))
        })?;
        if !canonical_artifact.starts_with(&canonical_root) || !canonical_artifact.is_file() {
            return Err(plan_error(
                "blocked campaign artifact escapes the campaign output root",
            ));
        }
        let result: HarnessRunResult = read_json(&canonical_artifact, "blocked harness result")?;
        if result.terminal.name() != terminal_name {
            return Err(plan_error(
                "campaign evaluation terminal contradicts its harness result",
            ));
        }
        let (kind, value) = if let Some(fact) = result.detail.missing_query_facts.first() {
            ("fact", fact.clone())
        } else if let Some(capability) = result.detail.missing_capabilities.first() {
            ("capability", capability.clone())
        } else if let Some(issue) = result.detail.observation_issues.first() {
            ("observation_family", issue.family.clone())
        } else {
            ("terminal", terminal_name.to_owned())
        };
        return Ok(Some(CampaignBlocker {
            terminal: result.terminal,
            kind: kind.into(),
            value,
            message: result.detail.message,
            artifact: canonical_artifact,
        }));
    }
    Ok(None)
}

fn repository_relative_string(
    repository_root: &Path,
    path: &Path,
    label: &str,
) -> Result<String, CampaignPlanError> {
    let relative = path
        .strip_prefix(repository_root)
        .map_err(|_| plan_error(format!("{label} must remain beneath the repository root")))?;
    let value = relative
        .to_str()
        .ok_or_else(|| plan_error(format!("{label} is not UTF-8")))?
        .replace(std::path::MAIN_SEPARATOR, "/");
    if value.is_empty() {
        return Err(plan_error(format!("{label} must not be empty")));
    }
    Ok(value)
}

fn seed_artifact(seed: &ObjectiveSeed) -> Option<&super::objective_suite::ArtifactReference> {
    match seed {
        ObjectiveSeed::Neutral => None,
        ObjectiveSeed::Tape { artifact }
        | ObjectiveSeed::TapeSource { artifact }
        | ObjectiveSeed::Controller { artifact } => Some(artifact),
    }
}

pub(super) fn required_capabilities(
    case: &super::objective_suite::ObjectiveSuiteCase,
) -> Vec<String> {
    let mut capabilities = BTreeSet::from([
        "gameplay-trace-v5".to_string(),
        "milestone-program-v1.5".to_string(),
        "scenario-fixture-v1".to_string(),
        "typed-fact-response-v1".to_string(),
    ]);
    capabilities.insert(match case.boot {
        ObjectiveBoot::Process => "process-boot".into(),
        ObjectiveBoot::Stage { .. } => "stage-boot".into(),
    });
    capabilities.insert(match case.seed {
        ObjectiveSeed::Neutral | ObjectiveSeed::Tape { .. } | ObjectiveSeed::TapeSource { .. } => {
            "input-tape-v3".into()
        }
        ObjectiveSeed::Controller { .. } => "input-controller-v1.4".into(),
    });
    for family in &case.observation_requirements.families {
        capabilities.insert(format!(
            "observation-family:{}/v{}",
            family.id, family.minimum_version
        ));
    }
    capabilities.into_iter().collect()
}

fn resolve_existing_file(
    root: &Path,
    path: &Path,
    label: &str,
) -> Result<PathBuf, CampaignPlanError> {
    let joined = if path.is_absolute() {
        path.to_owned()
    } else {
        root.join(path)
    };
    let canonical = joined.canonicalize().map_err(|error| {
        plan_error(format!(
            "cannot resolve {label} {}: {error}",
            joined.display()
        ))
    })?;
    if !canonical.starts_with(root) || !canonical.is_file() {
        return Err(plan_error(format!(
            "{label} must resolve to a file beneath the repository"
        )));
    }
    Ok(canonical)
}

fn canonical_relative_output(path: &Path) -> Result<&Path, CampaignPlanError> {
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::CurDir
                    | Component::ParentDir
                    | Component::RootDir
                    | Component::Prefix(_)
            )
        })
        || path.components().next() != Some(Component::Normal("build".as_ref()))
        || path.components().count() < 2
    {
        return Err(plan_error(
            "campaign output must be a canonical repository-relative path beneath build/",
        ));
    }
    Ok(path)
}

#[derive(Debug)]
pub struct CampaignPlanError(String);

impl fmt::Display for CampaignPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for CampaignPlanError {}

fn plan_error(message: impl Into<String>) -> CampaignPlanError {
    CampaignPlanError(message.into())
}
