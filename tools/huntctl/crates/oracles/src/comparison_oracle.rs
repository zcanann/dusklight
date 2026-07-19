//! Corpus- and cross-run semantic comparison oracles.

use crate::semantic_oracle::{OracleDisposition, OraclePolarity};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::error::Error;
use std::fmt;

pub const COMPARISON_ORACLE_SCHEMA_V1: &str = "dusklight-comparison-oracles/v1";
pub const COMPARISON_EVIDENCE_SCHEMA_V1: &str = "dusklight-comparison-evidence/v1";
const MAX_COMPARISON_ORACLES: usize = 64;
const MAX_EVENTS_PER_RUN: usize = 100_000;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComparisonOracleProgram {
    pub schema: String,
    pub oracles: Vec<ComparisonOracle>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComparisonOracle {
    pub name: String,
    pub polarity: OraclePolarity,
    pub target: ComparisonTarget,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ComparisonTarget {
    HeadfulHeadlessDivergence,
    ControlTreatmentDifference,
    NovelSemanticEventSignature {
        /// Empty means every run in the evidence artifact.
        #[serde(default)]
        roles: Vec<ComparisonRunRole>,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonRunRole {
    Headful,
    Headless,
    Control,
    Treatment,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComparisonEvidence {
    pub schema: String,
    /// SHA-256 identity of the corpus that produced `known_event_signatures`.
    pub catalog_identity: String,
    #[serde(default)]
    pub known_event_signatures: Vec<String>,
    pub runs: Vec<ComparisonRunEvidence>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComparisonRunEvidence {
    pub label: String,
    pub role: ComparisonRunRole,
    /// True only when the complete semantic event stream was retained.
    pub complete: bool,
    #[serde(default)]
    pub final_boundary_identity: Option<String>,
    #[serde(default)]
    pub events: Vec<SemanticEventSignature>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticEventSignature {
    pub simulation_tick: u64,
    #[serde(default)]
    pub tape_frame: Option<u64>,
    pub event_kind: String,
    /// SHA-256 of the canonical typed event facts, excluding display text.
    pub signature: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ComparisonOracleReport {
    pub schema: &'static str,
    pub catalog_identity: String,
    pub results: Vec<ComparisonOracleResult>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ComparisonOracleResult {
    pub name: String,
    pub polarity: OraclePolarity,
    pub disposition: OracleDisposition,
    pub first_match: Option<ComparisonMatch>,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ComparisonMatch {
    pub facts: ComparisonFacts,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ComparisonFacts {
    RunDivergence {
        left_label: String,
        right_label: String,
        event_index: usize,
        left_event: Option<SemanticEventSignature>,
        right_event: Option<SemanticEventSignature>,
        left_boundary_identity: Option<String>,
        right_boundary_identity: Option<String>,
    },
    NovelSemanticEventSignature {
        run_label: String,
        role: ComparisonRunRole,
        event_index: usize,
        event: SemanticEventSignature,
        catalog_identity: String,
    },
}

impl ComparisonOracleProgram {
    pub fn validate(&self) -> Result<(), ComparisonOracleError> {
        if self.schema != COMPARISON_ORACLE_SCHEMA_V1 {
            return Err(ComparisonOracleError::new(
                "unsupported comparison-oracle schema",
            ));
        }
        if self.oracles.is_empty() || self.oracles.len() > MAX_COMPARISON_ORACLES {
            return Err(ComparisonOracleError::new(
                "comparison-oracle count is outside 1..=64",
            ));
        }
        let mut names = HashSet::new();
        for oracle in &self.oracles {
            if !valid_text(&oracle.name) || !names.insert(&oracle.name) {
                return Err(ComparisonOracleError::new(
                    "comparison-oracle names must be unique and bounded",
                ));
            }
            if let ComparisonTarget::NovelSemanticEventSignature { roles } = &oracle.target {
                let unique = roles.iter().copied().collect::<HashSet<_>>();
                if unique.len() != roles.len() {
                    return Err(ComparisonOracleError::new(
                        "duplicate novel-signature role selector",
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn evaluate(
        &self,
        evidence: &ComparisonEvidence,
    ) -> Result<ComparisonOracleReport, ComparisonOracleError> {
        self.validate()?;
        evidence.validate()?;
        let results = self
            .oracles
            .iter()
            .map(|oracle| evaluate_one(oracle, evidence))
            .collect();
        Ok(ComparisonOracleReport {
            schema: "dusklight-comparison-oracle-results/v1",
            catalog_identity: evidence.catalog_identity.clone(),
            results,
        })
    }
}

impl ComparisonEvidence {
    pub fn validate(&self) -> Result<(), ComparisonOracleError> {
        if self.schema != COMPARISON_EVIDENCE_SCHEMA_V1 || !valid_digest(&self.catalog_identity) {
            return Err(ComparisonOracleError::new(
                "invalid comparison-evidence schema or catalog identity",
            ));
        }
        let known = self.known_event_signatures.iter().collect::<HashSet<_>>();
        if known.len() != self.known_event_signatures.len()
            || self
                .known_event_signatures
                .iter()
                .any(|digest| !valid_digest(digest))
        {
            return Err(ComparisonOracleError::new(
                "invalid or duplicate known event signature",
            ));
        }
        let mut labels = HashSet::new();
        let mut roles = HashSet::new();
        for run in &self.runs {
            if !valid_text(&run.label)
                || !labels.insert(&run.label)
                || !roles.insert(run.role)
                || run.events.len() > MAX_EVENTS_PER_RUN
                || run
                    .final_boundary_identity
                    .as_ref()
                    .is_some_and(|digest| !valid_digest(digest))
            {
                return Err(ComparisonOracleError::new(
                    "invalid, duplicate, or oversized comparison run",
                ));
            }
            let mut previous_tick = None;
            for event in &run.events {
                if !valid_text(&event.event_kind)
                    || !valid_digest(&event.signature)
                    || previous_tick.is_some_and(|tick| event.simulation_tick < tick)
                {
                    return Err(ComparisonOracleError::new(
                        "invalid or unordered semantic event signature",
                    ));
                }
                previous_tick = Some(event.simulation_tick);
            }
        }
        Ok(())
    }
}

fn evaluate_one(
    oracle: &ComparisonOracle,
    evidence: &ComparisonEvidence,
) -> ComparisonOracleResult {
    let (first_match, coverage) = match &oracle.target {
        ComparisonTarget::HeadfulHeadlessDivergence => compare_roles(
            evidence,
            ComparisonRunRole::Headful,
            ComparisonRunRole::Headless,
        ),
        ComparisonTarget::ControlTreatmentDifference => compare_roles(
            evidence,
            ComparisonRunRole::Control,
            ComparisonRunRole::Treatment,
        ),
        ComparisonTarget::NovelSemanticEventSignature { roles } => {
            find_novel_signature(evidence, roles)
        }
    };
    let (disposition, reason) = match (oracle.polarity, first_match.is_some(), coverage) {
        (OraclePolarity::Reached, true, _) => {
            (OracleDisposition::Satisfied, "difference was observed")
        }
        (OraclePolarity::Reached, false, true) => (
            OracleDisposition::Violated,
            "complete evidence contains no matching difference",
        ),
        (OraclePolarity::Reached, false, false) => (
            OracleDisposition::Indeterminate,
            "comparison evidence is incomplete or missing a required role",
        ),
        (OraclePolarity::Avoided, true, _) => (
            OracleDisposition::Violated,
            "forbidden difference was observed",
        ),
        (OraclePolarity::Avoided, false, true) => (
            OracleDisposition::Satisfied,
            "complete evidence proves equivalence to the declared comparison",
        ),
        (OraclePolarity::Avoided, false, false) => (
            OracleDisposition::Indeterminate,
            "avoidance requires complete comparison evidence",
        ),
    };
    ComparisonOracleResult {
        name: oracle.name.clone(),
        polarity: oracle.polarity,
        disposition,
        first_match,
        reason: reason.into(),
    }
}

fn compare_roles(
    evidence: &ComparisonEvidence,
    left_role: ComparisonRunRole,
    right_role: ComparisonRunRole,
) -> (Option<ComparisonMatch>, bool) {
    let left = evidence.runs.iter().find(|run| run.role == left_role);
    let right = evidence.runs.iter().find(|run| run.role == right_role);
    let (Some(left), Some(right)) = (left, right) else {
        return (None, false);
    };
    let compared = left.events.len().max(right.events.len());
    for index in 0..compared {
        if left.events.get(index) != right.events.get(index) {
            return (
                Some(divergence(left, right, index)),
                left.complete && right.complete,
            );
        }
    }
    if left.final_boundary_identity != right.final_boundary_identity {
        return (
            Some(divergence(left, right, compared)),
            left.complete && right.complete,
        );
    }
    (None, left.complete && right.complete)
}

fn divergence(
    left: &ComparisonRunEvidence,
    right: &ComparisonRunEvidence,
    index: usize,
) -> ComparisonMatch {
    ComparisonMatch {
        facts: ComparisonFacts::RunDivergence {
            left_label: left.label.clone(),
            right_label: right.label.clone(),
            event_index: index,
            left_event: left.events.get(index).cloned(),
            right_event: right.events.get(index).cloned(),
            left_boundary_identity: left.final_boundary_identity.clone(),
            right_boundary_identity: right.final_boundary_identity.clone(),
        },
    }
}

fn find_novel_signature(
    evidence: &ComparisonEvidence,
    roles: &[ComparisonRunRole],
) -> (Option<ComparisonMatch>, bool) {
    let selected = evidence
        .runs
        .iter()
        .filter(|run| roles.is_empty() || roles.contains(&run.role))
        .collect::<Vec<_>>();
    let coverage = !selected.is_empty() && selected.iter().all(|run| run.complete);
    let known = evidence
        .known_event_signatures
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    for run in selected {
        if let Some((event_index, event)) = run
            .events
            .iter()
            .enumerate()
            .find(|(_, event)| !known.contains(event.signature.as_str()))
        {
            return (
                Some(ComparisonMatch {
                    facts: ComparisonFacts::NovelSemanticEventSignature {
                        run_label: run.label.clone(),
                        role: run.role,
                        event_index,
                        event: event.clone(),
                        catalog_identity: evidence.catalog_identity.clone(),
                    },
                }),
                coverage,
            );
        }
    }
    (None, coverage)
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_text(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}

#[derive(Debug)]
pub struct ComparisonOracleError(String);

impl ComparisonOracleError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for ComparisonOracleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Error for ComparisonOracleError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(byte: char) -> String {
        std::iter::repeat_n(byte, 64).collect()
    }

    fn run(label: &str, role: ComparisonRunRole, signatures: &[char]) -> ComparisonRunEvidence {
        ComparisonRunEvidence {
            label: label.into(),
            role,
            complete: true,
            final_boundary_identity: Some(digest('f')),
            events: signatures
                .iter()
                .enumerate()
                .map(|(index, byte)| SemanticEventSignature {
                    simulation_tick: index as u64 + 1,
                    tape_frame: Some(index as u64),
                    event_kind: "procedure_change".into(),
                    signature: digest(*byte),
                })
                .collect(),
        }
    }

    #[test]
    fn comparison_oracles_find_first_divergence_and_novel_signature() {
        let evidence = ComparisonEvidence {
            schema: COMPARISON_EVIDENCE_SCHEMA_V1.into(),
            catalog_identity: digest('c'),
            known_event_signatures: vec![digest('1')],
            runs: vec![
                run("gui", ComparisonRunRole::Headful, &['1', '2']),
                run("null", ComparisonRunRole::Headless, &['1', '3']),
                run("baseline", ComparisonRunRole::Control, &['1']),
                run("intervention", ComparisonRunRole::Treatment, &['1', '4']),
            ],
        };
        let program = ComparisonOracleProgram {
            schema: COMPARISON_ORACLE_SCHEMA_V1.into(),
            oracles: vec![
                ComparisonOracle {
                    name: "renderer-diverged".into(),
                    polarity: OraclePolarity::Reached,
                    target: ComparisonTarget::HeadfulHeadlessDivergence,
                },
                ComparisonOracle {
                    name: "treatment-differed".into(),
                    polarity: OraclePolarity::Reached,
                    target: ComparisonTarget::ControlTreatmentDifference,
                },
                ComparisonOracle {
                    name: "novel-treatment-event".into(),
                    polarity: OraclePolarity::Reached,
                    target: ComparisonTarget::NovelSemanticEventSignature {
                        roles: vec![ComparisonRunRole::Treatment],
                    },
                },
            ],
        };
        let report = program.evaluate(&evidence).unwrap();
        assert!(
            report
                .results
                .iter()
                .all(|result| result.disposition == OracleDisposition::Satisfied)
        );
        assert!(matches!(
            report.results[0].first_match.as_ref().unwrap().facts,
            ComparisonFacts::RunDivergence { event_index: 1, .. }
        ));
        assert!(matches!(
            report.results[2].first_match.as_ref().unwrap().facts,
            ComparisonFacts::NovelSemanticEventSignature { event_index: 1, .. }
        ));
    }

    #[test]
    fn avoided_comparison_requires_complete_streams() {
        let mut evidence = ComparisonEvidence {
            schema: COMPARISON_EVIDENCE_SCHEMA_V1.into(),
            catalog_identity: digest('c'),
            known_event_signatures: vec![digest('1')],
            runs: vec![
                run("gui", ComparisonRunRole::Headful, &['1']),
                run("null", ComparisonRunRole::Headless, &['1']),
            ],
        };
        let program = ComparisonOracleProgram {
            schema: COMPARISON_ORACLE_SCHEMA_V1.into(),
            oracles: vec![ComparisonOracle {
                name: "renderer-equivalent".into(),
                polarity: OraclePolarity::Avoided,
                target: ComparisonTarget::HeadfulHeadlessDivergence,
            }],
        };
        assert_eq!(
            program.evaluate(&evidence).unwrap().results[0].disposition,
            OracleDisposition::Satisfied
        );
        evidence.runs[1].complete = false;
        assert_eq!(
            program.evaluate(&evidence).unwrap().results[0].disposition,
            OracleDisposition::Indeterminate
        );
    }

    #[test]
    fn checked_in_comparison_fixtures_evaluate() {
        let program: ComparisonOracleProgram = serde_json::from_str(include_str!(
            "../../../../../tests/fixtures/automation/comparison_oracles.json"
        ))
        .unwrap();
        let evidence: ComparisonEvidence = serde_json::from_str(include_str!(
            "../../../../../tests/fixtures/automation/comparison_evidence.json"
        ))
        .unwrap();
        let report = program.evaluate(&evidence).unwrap();
        assert!(
            report
                .results
                .iter()
                .all(|result| result.disposition == OracleDisposition::Satisfied)
        );
    }
}
