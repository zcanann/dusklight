//! Human-readable inspection of one authenticated objective and optional run.

use super::observation_contract::ObservationAdmissionIssueKind;
use super::run_contract::{HarnessRunRequest, HarnessRunResult};
use crate::artifact::Digest;
use crate::milestone_dsl::{self, EvaluationPhase};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::Path;

pub fn inspect_objective(
    request: &HarnessRunRequest,
    repository_root: &Path,
    result: Option<(&HarnessRunResult, &Path)>,
) -> Result<String, ObjectiveInspectionError> {
    request
        .validate_files(repository_root)
        .map_err(|error| inspection_error(format!("invalid run request: {error}")))?;
    if let Some((result, artifact_root)) = result {
        result
            .validate_files(request, artifact_root)
            .map_err(|error| inspection_error(format!("invalid run result: {error}")))?;
    }

    let source = fs::read_to_string(repository_root.join(&request.objective.source.path))
        .map_err(|error| inspection_error(format!("cannot read objective source: {error}")))?;
    let program = milestone_dsl::parse(&source)
        .map_err(|error| inspection_error(format!("cannot parse objective source: {error}")))?;
    let compiled = milestone_dsl::compile(&program)
        .map_err(|error| inspection_error(format!("cannot compile objective source: {error}")))?;
    let definition = program
        .definitions
        .iter()
        .find(|definition| definition.name == request.objective.goal)
        .ok_or_else(|| inspection_error("objective goal is absent from its source program"))?;
    let definition_identity = compiled
        .definitions
        .iter()
        .find(|identity| identity.name == request.objective.goal)
        .ok_or_else(|| inspection_error("compiled objective goal identity is absent"))?;

    let mut output = String::new();
    push_line(
        &mut output,
        format!("Objective: {}", request.objective.goal),
    );
    push_line(
        &mut output,
        format!("Source: {}", request.objective.source.path),
    );
    push_line(
        &mut output,
        format!("Program SHA-256: {}", request.objective.program_sha256),
    );
    push_line(
        &mut output,
        format!("Definition SHA-256: {}", Digest(definition_identity.sha256)),
    );
    push_line(
        &mut output,
        format!(
            "Evaluation: {} phase, stable {} tick(s), {} ordered continuation(s)",
            match definition.phase {
                EvaluationPhase::PreInput => "pre-input",
                EvaluationPhase::PostSim => "post-simulation",
            },
            definition.stable_ticks,
            definition.then.len(),
        ),
    );
    output.push_str("Required observation families:\n");
    for family in &request.observation_requirements.families {
        push_line(
            &mut output,
            format!("  - {} >= v{}", family.id, family.minimum_version),
        );
    }
    output.push_str("Required facts:\n");
    for fact in &request.observation_requirements.facts {
        push_line(&mut output, format!("  - {fact}"));
    }

    let mut missing = Vec::new();
    match result {
        None => {
            output.push_str("Progress: not run\n");
            output.push_str("First-hit boundary: none\n");
            missing.push("run result".to_string());
        }
        Some((result, _)) => {
            if result.objective.reached {
                push_line(
                    &mut output,
                    format!(
                        "Progress: reached at logical tick {}",
                        result.objective.first_hit_tick.unwrap_or_default()
                    ),
                );
            } else {
                push_line(
                    &mut output,
                    format!(
                        "Progress: {} after {} logical tick(s)",
                        result.terminal.name(),
                        result.timing.logical_ticks
                    ),
                );
            }
            if let Some(fingerprint) = &result.objective.boundary_fingerprint {
                push_line(
                    &mut output,
                    format!(
                        "First-hit boundary: {} {} {} ({})",
                        fingerprint.schema,
                        fingerprint.algorithm,
                        fingerprint.digest,
                        fingerprint.canonical_encoding
                    ),
                );
            } else {
                output.push_str("First-hit boundary: none\n");
            }
            for fact in &result.detail.missing_query_facts {
                missing.push(format!("query fact {fact}"));
            }
            for issue in &result.detail.observation_issues {
                missing.push(format!(
                    "observation family {} ({})",
                    issue.family,
                    issue_kind_name(issue.kind)
                ));
            }
            for capability in &result.detail.missing_capabilities {
                missing.push(format!("capability {capability}"));
            }
            if result.objective.reached && result.objective.evidence.is_none() {
                missing.push("objective evidence".into());
            }
            if result.objective.reached && result.objective.boundary_fingerprint.is_none() {
                missing.push("first-hit boundary fingerprint".into());
            }
            for (present, label) in [
                (result.artifacts.realized_input.is_some(), "realized input"),
                (result.artifacts.gameplay_trace.is_some(), "gameplay trace"),
                (
                    result.artifacts.objective_result.is_some(),
                    "objective result",
                ),
            ] {
                if !present {
                    missing.push(label.into());
                }
            }
            if !result.artifacts.complete {
                missing.push("complete artifact set".into());
            }
        }
    }
    output.push_str("Missing evidence:\n");
    if missing.is_empty() {
        output.push_str("  - none\n");
    } else {
        for item in missing {
            push_line(&mut output, format!("  - {item}"));
        }
    }
    output.push_str("Source objective:\n---\n");
    output.push_str(source.trim_end());
    output.push_str("\n---\n");
    Ok(output)
}

fn issue_kind_name(kind: ObservationAdmissionIssueKind) -> &'static str {
    match kind {
        ObservationAdmissionIssueKind::Missing => "missing",
        ObservationAdmissionIssueKind::VersionTooOld => "version too old",
        ObservationAdmissionIssueKind::NotSampled => "not sampled",
        ObservationAdmissionIssueKind::Unavailable => "unavailable",
        ObservationAdmissionIssueKind::Truncated => "truncated",
        ObservationAdmissionIssueKind::Stale => "stale",
        ObservationAdmissionIssueKind::Invalid => "invalid",
    }
}

fn push_line(output: &mut String, line: impl AsRef<str>) {
    output.push_str(line.as_ref());
    output.push('\n');
}

#[derive(Debug)]
pub struct ObjectiveInspectionError(String);

impl fmt::Display for ObjectiveInspectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for ObjectiveInspectionError {}

fn inspection_error(message: impl Into<String>) -> ObjectiveInspectionError {
    ObjectiveInspectionError(message.into())
}
