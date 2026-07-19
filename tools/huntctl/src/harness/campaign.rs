//! Read-only planning for the top-level objective campaign command.

use super::objective_suite::{
    ExpectedTerminalClass, ObjectiveBoot, ObjectiveCaseRole, ObjectiveSeed, ObjectiveSuite,
};
use crate::artifact::Digest;
use serde::Serialize;
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::str::FromStr;

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
            episodes: output_root.join("episodes"),
            finalists: output_root.join("finalists"),
            replays: output_root.join("replays"),
            report: output_root.join("report.json"),
        },
    })
}

fn seed_artifact(seed: &ObjectiveSeed) -> Option<&super::objective_suite::ArtifactReference> {
    match seed {
        ObjectiveSeed::Neutral => None,
        ObjectiveSeed::Tape { artifact }
        | ObjectiveSeed::TapeSource { artifact }
        | ObjectiveSeed::Controller { artifact } => Some(artifact),
    }
}

fn required_capabilities(case: &super::objective_suite::ObjectiveSuiteCase) -> Vec<String> {
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
