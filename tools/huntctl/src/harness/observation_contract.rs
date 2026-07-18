//! Versioned observation requirements and worker admission for harness objectives.

use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fmt;

pub const OBJECTIVE_OBSERVATION_REQUIREMENTS_SCHEMA_V1: &str =
    "dusklight-objective-observation-requirements/v1";
pub const OBSERVATION_INVENTORY_SCHEMA_V1: &str = "dusklight-observation-inventory/v1";
const MAX_FAMILIES: usize = 32;
const MAX_FACTS: usize = 128;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectiveObservationRequirements {
    pub schema: String,
    pub families: Vec<ObservationFamilyRequirement>,
    pub facts: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationFamilyRequirement {
    pub id: String,
    pub minimum_version: u16,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationInventory {
    pub schema: String,
    pub families: Vec<ObservationFamilyAvailability>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationFamilyAvailability {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<u16>,
    pub status: ObservationFamilyStatus,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationFamilyStatus {
    Present,
    Absent,
    NotSampled,
    Unavailable,
    Truncated,
    Stale,
    Invalid,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationAdmission {
    pub supported: bool,
    pub issues: Vec<ObservationAdmissionIssue>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationAdmissionIssue {
    pub family: String,
    pub minimum_version: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual_version: Option<u16>,
    pub kind: ObservationAdmissionIssueKind,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationAdmissionIssueKind {
    Missing,
    VersionTooOld,
    NotSampled,
    Unavailable,
    Truncated,
    Stale,
    Invalid,
}

impl ObjectiveObservationRequirements {
    pub fn validate(&self) -> Result<(), ObservationContractError> {
        if self.schema != OBJECTIVE_OBSERVATION_REQUIREMENTS_SCHEMA_V1 {
            return Err(observation_error(
                "unsupported objective observation-requirements schema",
            ));
        }
        if self.families.is_empty()
            || self.families.len() > MAX_FAMILIES
            || !self.families.windows(2).all(|pair| pair[0].id < pair[1].id)
        {
            return Err(observation_error(
                "observation families must be nonempty, bounded, unique, and id-sorted",
            ));
        }
        for family in &self.families {
            validate_id("observation family", &family.id)?;
            if family.minimum_version == 0 {
                return Err(observation_error(
                    "observation family minimum version must be nonzero",
                ));
            }
        }
        if self.facts.is_empty()
            || self.facts.len() > MAX_FACTS
            || !self.facts.windows(2).all(|pair| pair[0] < pair[1])
        {
            return Err(observation_error(
                "observation facts must be nonempty, bounded, unique, and sorted",
            ));
        }
        for fact in &self.facts {
            validate_fact(fact)?;
            let family = family_for_fact(fact).ok_or_else(|| {
                observation_error(format!("observation fact {fact} has no declared family"))
            })?;
            if self
                .families
                .binary_search_by_key(&family, |requirement| requirement.id.as_str())
                .is_err()
            {
                return Err(observation_error(format!(
                    "observation fact {fact} requires undeclared family {family}"
                )));
            }
        }
        Ok(())
    }

    pub fn assess(
        &self,
        inventory: &ObservationInventory,
    ) -> Result<ObservationAdmission, ObservationContractError> {
        self.validate()?;
        inventory.validate()?;
        let mut issues = Vec::new();
        for requirement in &self.families {
            let availability = inventory
                .families
                .binary_search_by_key(&requirement.id.as_str(), |family| family.id.as_str())
                .ok()
                .map(|index| &inventory.families[index]);
            let Some(availability) = availability else {
                issues.push(ObservationAdmissionIssue {
                    family: requirement.id.clone(),
                    minimum_version: requirement.minimum_version,
                    actual_version: None,
                    kind: ObservationAdmissionIssueKind::Missing,
                });
                continue;
            };
            let kind = match availability.status {
                ObservationFamilyStatus::Present | ObservationFamilyStatus::Absent
                    if availability
                        .version
                        .is_some_and(|version| version < requirement.minimum_version) =>
                {
                    Some(ObservationAdmissionIssueKind::VersionTooOld)
                }
                ObservationFamilyStatus::Present | ObservationFamilyStatus::Absent => None,
                ObservationFamilyStatus::NotSampled => {
                    Some(ObservationAdmissionIssueKind::NotSampled)
                }
                ObservationFamilyStatus::Unavailable => {
                    Some(ObservationAdmissionIssueKind::Unavailable)
                }
                ObservationFamilyStatus::Truncated => {
                    Some(ObservationAdmissionIssueKind::Truncated)
                }
                ObservationFamilyStatus::Stale => Some(ObservationAdmissionIssueKind::Stale),
                ObservationFamilyStatus::Invalid => Some(ObservationAdmissionIssueKind::Invalid),
            };
            if let Some(kind) = kind {
                issues.push(ObservationAdmissionIssue {
                    family: requirement.id.clone(),
                    minimum_version: requirement.minimum_version,
                    actual_version: availability.version,
                    kind,
                });
            }
        }
        Ok(ObservationAdmission {
            supported: issues.is_empty(),
            issues,
        })
    }

    pub fn facts_for_issues(&self, issues: &[ObservationAdmissionIssue]) -> Vec<String> {
        self.facts
            .iter()
            .filter(|fact| {
                family_for_fact(fact)
                    .is_some_and(|family| issues.iter().any(|issue| issue.family == family))
            })
            .cloned()
            .collect()
    }
}

impl ObservationInventory {
    pub fn validate(&self) -> Result<(), ObservationContractError> {
        if self.schema != OBSERVATION_INVENTORY_SCHEMA_V1 {
            return Err(observation_error(
                "unsupported observation-inventory schema",
            ));
        }
        if self.families.len() > MAX_FAMILIES
            || !self.families.windows(2).all(|pair| pair[0].id < pair[1].id)
        {
            return Err(observation_error(
                "observation inventory must be bounded, unique, and id-sorted",
            ));
        }
        for family in &self.families {
            validate_id("observation family", &family.id)?;
            if family.version == Some(0)
                || matches!(
                    family.status,
                    ObservationFamilyStatus::Present | ObservationFamilyStatus::Absent
                ) && family.version.is_none()
            {
                return Err(observation_error(
                    "present or absent observation families require a nonzero version",
                ));
            }
        }
        Ok(())
    }
}

impl ObservationAdmissionIssue {
    pub fn validate(&self) -> Result<(), ObservationContractError> {
        validate_id("observation issue family", &self.family)?;
        if self.minimum_version == 0 || self.actual_version == Some(0) {
            return Err(observation_error(
                "observation issue versions must be nonzero when present",
            ));
        }
        match self.kind {
            ObservationAdmissionIssueKind::Missing if self.actual_version.is_none() => Ok(()),
            ObservationAdmissionIssueKind::VersionTooOld
                if self
                    .actual_version
                    .is_some_and(|version| version < self.minimum_version) =>
            {
                Ok(())
            }
            ObservationAdmissionIssueKind::NotSampled
            | ObservationAdmissionIssueKind::Unavailable
            | ObservationAdmissionIssueKind::Truncated
            | ObservationAdmissionIssueKind::Stale
            | ObservationAdmissionIssueKind::Invalid => Ok(()),
            _ => Err(observation_error(
                "observation issue kind contradicts its version evidence",
            )),
        }
    }
}

pub fn family_for_fact(fact: &str) -> Option<&'static str> {
    if fact.starts_with("boundary.") || fact.starts_with("tape.") {
        Some("core")
    } else if fact.starts_with("stage.") || fact.starts_with("next_stage.") {
        Some("stage")
    } else if fact.starts_with("player.interaction.")
        || fact.starts_with("player.timer.")
        || fact == "player.mode_flags"
    {
        Some("player_action")
    } else if fact.starts_with("player.") {
        Some("player_motion")
    } else if fact.starts_with("event.") {
        Some("event")
    } else if fact.starts_with("rng.") {
        Some("rng")
    } else if fact.starts_with("collision.") {
        Some("player_background_collision")
    } else if fact.starts_with("actor.placed.") {
        Some("selected_actors")
    } else if fact.starts_with("flag.") {
        Some("flags")
    } else if fact.starts_with("goal.") {
        Some("goal_progress")
    } else {
        None
    }
}

fn validate_id(label: &str, value: &str) -> Result<(), ObservationContractError> {
    if value.is_empty()
        || value.len() > 96
        || value != value.trim()
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'-' | b'_' | b'.' | b'/')
        })
    {
        return Err(observation_error(format!(
            "{label} is not a canonical identifier"
        )));
    }
    Ok(())
}

fn validate_fact(value: &str) -> Result<(), ObservationContractError> {
    if value.is_empty()
        || value.len() > 192
        || value.starts_with('.')
        || value.ends_with('.')
        || value.bytes().any(|byte| {
            !(byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'_' | b'.' | b'[' | b']' | b'-'))
        })
    {
        return Err(observation_error(
            "observation fact is not a canonical fact path",
        ));
    }
    Ok(())
}

#[derive(Debug)]
pub struct ObservationContractError(String);

impl fmt::Display for ObservationContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for ObservationContractError {}

fn observation_error(message: impl Into<String>) -> ObservationContractError {
    ObservationContractError(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn requirements() -> ObjectiveObservationRequirements {
        ObjectiveObservationRequirements {
            schema: OBJECTIVE_OBSERVATION_REQUIREMENTS_SCHEMA_V1.into(),
            families: vec![
                ObservationFamilyRequirement {
                    id: "player_action".into(),
                    minimum_version: 2,
                },
                ObservationFamilyRequirement {
                    id: "stage".into(),
                    minimum_version: 1,
                },
            ],
            facts: vec![
                "player.interaction.talk_partner.set_id".into(),
                "stage.name".into(),
            ],
        }
    }

    #[test]
    fn accepts_present_or_semantically_absent_complete_families() {
        let inventory = ObservationInventory {
            schema: OBSERVATION_INVENTORY_SCHEMA_V1.into(),
            families: vec![
                ObservationFamilyAvailability {
                    id: "player_action".into(),
                    version: Some(2),
                    status: ObservationFamilyStatus::Absent,
                },
                ObservationFamilyAvailability {
                    id: "stage".into(),
                    version: Some(1),
                    status: ObservationFamilyStatus::Present,
                },
            ],
        };
        let admission = requirements().assess(&inventory).unwrap();
        assert!(admission.supported);
        assert!(admission.issues.is_empty());
    }

    #[test]
    fn reports_truncation_old_versions_and_missing_families() {
        let mut requirements = requirements();
        requirements.families.insert(
            0,
            ObservationFamilyRequirement {
                id: "event".into(),
                minimum_version: 1,
            },
        );
        let inventory = ObservationInventory {
            schema: OBSERVATION_INVENTORY_SCHEMA_V1.into(),
            families: vec![
                ObservationFamilyAvailability {
                    id: "player_action".into(),
                    version: Some(1),
                    status: ObservationFamilyStatus::Present,
                },
                ObservationFamilyAvailability {
                    id: "stage".into(),
                    version: Some(1),
                    status: ObservationFamilyStatus::Truncated,
                },
            ],
        };
        let admission = requirements.assess(&inventory).unwrap();
        assert!(!admission.supported);
        assert_eq!(
            admission
                .issues
                .iter()
                .map(|issue| issue.kind)
                .collect::<Vec<_>>(),
            [
                ObservationAdmissionIssueKind::Missing,
                ObservationAdmissionIssueKind::VersionTooOld,
                ObservationAdmissionIssueKind::Truncated,
            ]
        );
        assert_eq!(
            requirements.facts_for_issues(&admission.issues),
            [
                "player.interaction.talk_partner.set_id".to_string(),
                "stage.name".to_string(),
            ]
        );
    }

    #[test]
    fn rejects_a_fact_without_its_family() {
        let mut requirements = requirements();
        requirements.families.remove(0);
        assert!(requirements.validate().is_err());
    }
}
