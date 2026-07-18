//! Versioned inputs for the small native objective-conformance suite.

use crate::artifact::Digest;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::path::{Component, Path, PathBuf};

pub const OBJECTIVE_CONFORMANCE_SUITE_SCHEMA_V1: &str = "dusklight-objective-conformance-suite/v1";
const MAX_CASES: usize = 32;
const MAX_REQUIRED_FACTS: usize = 128;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectiveConformanceSuite {
    pub schema: String,
    pub cases: Vec<ObjectiveConformanceCase>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectiveConformanceCase {
    pub id: String,
    pub boot: ConformanceBoot,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scenario_fixture: Option<ConformanceArtifact>,
    pub objective_program: ConformanceArtifact,
    pub required_query_facts: Vec<String>,
    pub seed: ConformanceSeed,
    pub budget: ConformanceBudget,
    pub expected_terminal: ConformanceTerminalClass,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ConformanceBoot {
    Process,
    Stage {
        stage: String,
        room: i8,
        point: i16,
        layer: i8,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        save_slot: Option<u8>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConformanceArtifact {
    /// Repository-relative source path. It is descriptive; `sha256` is identity.
    pub path: PathBuf,
    pub sha256: Digest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ConformanceSeed {
    Tape { artifact: ConformanceArtifact },
    Controller { artifact: ConformanceArtifact },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConformanceBudget {
    pub logical_ticks: u64,
    pub host_timeout_seconds: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConformanceTerminalClass {
    Reached,
    Exhausted,
    Impossible,
    Unsupported,
    Timeout,
    Crash,
    Hang,
    TargetLost,
    Nondeterministic,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObjectiveConformanceError(String);

impl fmt::Display for ObjectiveConformanceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for ObjectiveConformanceError {}

impl ObjectiveConformanceSuite {
    pub fn validate(&self) -> Result<(), ObjectiveConformanceError> {
        if self.schema != OBJECTIVE_CONFORMANCE_SUITE_SCHEMA_V1 {
            return Err(error("unsupported objective-conformance suite schema"));
        }
        if self.cases.is_empty() || self.cases.len() > MAX_CASES {
            return Err(error("objective-conformance suite requires 1..=32 cases"));
        }

        let mut ids = BTreeSet::new();
        for case in &self.cases {
            if !valid_identifier(&case.id) || !ids.insert(case.id.as_str()) {
                return Err(error("case ids must be unique lowercase identifiers"));
            }
            case.validate()?;
        }
        Ok(())
    }
}

impl ObjectiveConformanceCase {
    fn validate(&self) -> Result<(), ObjectiveConformanceError> {
        match &self.boot {
            ConformanceBoot::Process => {}
            ConformanceBoot::Stage {
                stage, save_slot, ..
            } => {
                if stage.is_empty()
                    || stage.len() > 16
                    || !stage
                        .bytes()
                        .all(|byte| byte.is_ascii_graphic() && byte != b',')
                {
                    return Err(error("stage boot requires a canonical stage id"));
                }
                if save_slot.is_some_and(|slot| !(1..=3).contains(&slot)) {
                    return Err(error("stage boot save_slot must be in 1..=3"));
                }
                if self.scenario_fixture.is_none() {
                    return Err(error("stage boot requires an explicit scenario fixture"));
                }
            }
        }

        if let Some(fixture) = &self.scenario_fixture {
            fixture.validate("scenario fixture")?;
        }
        self.objective_program.validate("objective program")?;
        match &self.seed {
            ConformanceSeed::Tape { artifact } => artifact.validate("tape seed")?,
            ConformanceSeed::Controller { artifact } => artifact.validate("controller seed")?,
        }

        if self.required_query_facts.is_empty()
            || self.required_query_facts.len() > MAX_REQUIRED_FACTS
        {
            return Err(error("case requires 1..=128 query facts"));
        }
        if self
            .required_query_facts
            .iter()
            .any(|fact| !valid_fact_id(fact))
            || self
                .required_query_facts
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
        {
            return Err(error(
                "required query facts must be canonical, sorted, and unique",
            ));
        }
        if self.budget.logical_ticks == 0 || self.budget.host_timeout_seconds == 0 {
            return Err(error("case time budgets must be nonzero"));
        }
        Ok(())
    }
}

impl ConformanceArtifact {
    fn validate(&self, label: &str) -> Result<(), ObjectiveConformanceError> {
        if !portable_relative_path(&self.path) {
            return Err(error(format!(
                "{label} path must be a normalized repository-relative path"
            )));
        }
        if self.sha256 == Digest::ZERO {
            return Err(error(format!("{label} sha256 must be nonzero")));
        }
        Ok(())
    }
}

fn portable_relative_path(path: &Path) -> bool {
    let mut components = path.components();
    let Some(Component::Normal(_)) = components.next() else {
        return false;
    };
    components.all(|component| matches!(component, Component::Normal(_)))
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        })
}

fn valid_fact_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'-' | b'_' | b'.' | b'/')
        })
}

fn error(message: impl Into<String>) -> ObjectiveConformanceError {
    ObjectiveConformanceError(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn artifact(path: &str, byte: u8) -> ConformanceArtifact {
        ConformanceArtifact {
            path: path.into(),
            sha256: Digest([byte; 32]),
        }
    }

    fn suite() -> ObjectiveConformanceSuite {
        ObjectiveConformanceSuite {
            schema: OBJECTIVE_CONFORMANCE_SUITE_SCHEMA_V1.into(),
            cases: vec![ObjectiveConformanceCase {
                id: "stage-ready".into(),
                boot: ConformanceBoot::Stage {
                    stage: "F_SP103".into(),
                    room: 1,
                    point: 1,
                    layer: 3,
                    save_slot: None,
                },
                scenario_fixture: Some(artifact(
                    "tests/fixtures/automation/fsp103_practice.fixture.json",
                    1,
                )),
                objective_program: artifact(
                    "tests/fixtures/automation/rich_predicate.milestones",
                    2,
                ),
                required_query_facts: vec![
                    "player.location.stage/v1".into(),
                    "player.position/v1".into(),
                ],
                seed: ConformanceSeed::Tape {
                    artifact: artifact("tests/fixtures/automation/fsp103_next_map_seed.tas", 3),
                },
                budget: ConformanceBudget {
                    logical_ticks: 300,
                    host_timeout_seconds: 30,
                },
                expected_terminal: ConformanceTerminalClass::Reached,
            }],
        }
    }

    #[test]
    fn complete_suite_round_trips_and_validates() {
        let suite = suite();
        suite.validate().unwrap();
        let bytes = serde_json::to_vec(&suite).unwrap();
        let decoded: ObjectiveConformanceSuite = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(decoded, suite);
        decoded.validate().unwrap();
    }

    #[test]
    fn stage_boot_requires_fixture_identity() {
        let mut suite = suite();
        suite.cases[0].scenario_fixture = None;
        assert_eq!(
            suite.validate().unwrap_err().to_string(),
            "stage boot requires an explicit scenario fixture"
        );
    }

    #[test]
    fn artifact_identity_rejects_zero_digest_and_path_escape() {
        let mut suite = suite();
        suite.cases[0].objective_program.sha256 = Digest::ZERO;
        assert!(
            suite
                .validate()
                .unwrap_err()
                .to_string()
                .contains("nonzero")
        );

        suite.cases[0].objective_program = artifact("../objective.milestones", 2);
        assert!(
            suite
                .validate()
                .unwrap_err()
                .to_string()
                .contains("repository-relative")
        );
    }

    #[test]
    fn query_fact_contract_is_sorted_and_unique() {
        let mut suite = suite();
        suite.cases[0].required_query_facts.reverse();
        assert!(suite.validate().unwrap_err().to_string().contains("sorted"));

        suite.cases[0].required_query_facts = vec!["player.position/v1".into(); 2];
        assert!(suite.validate().unwrap_err().to_string().contains("unique"));
    }
}
