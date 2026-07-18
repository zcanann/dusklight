//! Bounded open-question campaigns over raw semantic novelty descriptors.

use super::archive::DiscoveryArchivePartitionKey;
use super::{ContactFact, SemanticNoveltyDescriptor};
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

pub const OPEN_QUESTION_CAMPAIGN_SCHEMA: &str = "dusklight-open-question-campaign/v1";
pub const OPEN_QUESTION_ASSESSMENT_SCHEMA: &str = "dusklight-open-question-campaign-assessment/v1";
pub const MAX_OPEN_QUESTION_EPISODES: u64 = 1_000_000;
pub const MAX_OPEN_QUESTION_FACTS: usize = 65_536;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenQuestion {
    ProduceUnseenProcedureContactPair,
    CrossCollisionWithoutSceneTransition,
    ContactChangeWithoutSemanticTransition,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct OpenQuestionCampaignSpec {
    pub schema: &'static str,
    pub name: String,
    pub partition: DiscoveryArchivePartitionKey,
    pub question: OpenQuestion,
    pub maximum_episodes: u64,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct ProcedureContactPair {
    pub procedure: u16,
    pub contact: ContactFact,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ContactChangeWithoutTransition {
    pub unchanged_state: super::SemanticState,
    pub before: Option<super::ContactState>,
    pub after: Option<super::ContactState>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum OpenQuestionEvidence {
    UnseenProcedureContactPairs {
        pairs: Vec<ProcedureContactPair>,
    },
    CollisionExitWithoutSceneTransition {
        contacts: Vec<ContactFact>,
    },
    ContactChangesWithoutSemanticTransition {
        changes: Vec<ContactChangeWithoutTransition>,
    },
    NoMatch,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct OpenQuestionAssessment {
    pub schema: &'static str,
    pub campaign_identity: String,
    pub descriptor_identity: String,
    pub observed_episodes_before: u64,
    pub matched: bool,
    pub evidence: OpenQuestionEvidence,
    pub proposal_signal_only: bool,
    pub promotion_authority: bool,
}

#[derive(Clone, Debug)]
pub struct OpenQuestionCampaign {
    spec: OpenQuestionCampaignSpec,
    identity: String,
    observed_episodes: u64,
    observed_procedure_contact_pairs: BTreeSet<ProcedureContactPair>,
}

#[derive(Debug)]
pub struct OpenQuestionCampaignError(String);

impl fmt::Display for OpenQuestionCampaignError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for OpenQuestionCampaignError {}

impl OpenQuestionCampaign {
    pub fn new(spec: OpenQuestionCampaignSpec) -> Result<Self, OpenQuestionCampaignError> {
        validate_spec(&spec)?;
        let identity = campaign_identity(&spec);
        Ok(Self {
            spec,
            identity,
            observed_episodes: 0,
            observed_procedure_contact_pairs: BTreeSet::new(),
        })
    }

    pub fn identity(&self) -> &str {
        &self.identity
    }

    pub fn assess(
        &self,
        descriptor: &SemanticNoveltyDescriptor,
    ) -> Result<OpenQuestionAssessment, OpenQuestionCampaignError> {
        if self.observed_episodes >= self.spec.maximum_episodes {
            return Err(OpenQuestionCampaignError(
                "open-question campaign episode budget is exhausted".into(),
            ));
        }
        let evidence = match self.spec.question {
            OpenQuestion::ProduceUnseenProcedureContactPair => {
                let pairs = procedure_contact_pairs(descriptor)
                    .difference(&self.observed_procedure_contact_pairs)
                    .cloned()
                    .collect::<Vec<_>>();
                if pairs.is_empty() {
                    OpenQuestionEvidence::NoMatch
                } else {
                    OpenQuestionEvidence::UnseenProcedureContactPairs { pairs }
                }
            }
            OpenQuestion::CrossCollisionWithoutSceneTransition => {
                let contacts = collision_exit_contacts(descriptor);
                let has_scene_transition = descriptor.state_transitions.iter().any(|transition| {
                    transition.from.stage != transition.to.stage
                        || transition.from.room != transition.to.room
                        || transition.from.layer != transition.to.layer
                });
                if contacts.is_empty() || has_scene_transition {
                    OpenQuestionEvidence::NoMatch
                } else {
                    OpenQuestionEvidence::CollisionExitWithoutSceneTransition { contacts }
                }
            }
            OpenQuestion::ContactChangeWithoutSemanticTransition => {
                let changes = descriptor
                    .state_combinations
                    .windows(2)
                    .filter_map(|pair| {
                        let before = &pair[0];
                        let after = &pair[1];
                        (before.state == after.state && before.contact != after.contact).then(
                            || ContactChangeWithoutTransition {
                                unchanged_state: before.state.clone(),
                                before: before.contact.clone(),
                                after: after.contact.clone(),
                            },
                        )
                    })
                    .collect::<Vec<_>>();
                if changes.is_empty() {
                    OpenQuestionEvidence::NoMatch
                } else {
                    OpenQuestionEvidence::ContactChangesWithoutSemanticTransition { changes }
                }
            }
        };
        Ok(OpenQuestionAssessment {
            schema: OPEN_QUESTION_ASSESSMENT_SCHEMA,
            campaign_identity: self.identity.clone(),
            descriptor_identity: descriptor.identity(),
            observed_episodes_before: self.observed_episodes,
            matched: !matches!(evidence, OpenQuestionEvidence::NoMatch),
            evidence,
            proposal_signal_only: true,
            promotion_authority: false,
        })
    }

    pub fn record(
        &mut self,
        descriptor: &SemanticNoveltyDescriptor,
    ) -> Result<(), OpenQuestionCampaignError> {
        if self.observed_episodes >= self.spec.maximum_episodes {
            return Err(OpenQuestionCampaignError(
                "open-question campaign episode budget is exhausted".into(),
            ));
        }
        let pairs = procedure_contact_pairs(descriptor);
        let new_pair_count = pairs
            .iter()
            .filter(|pair| !self.observed_procedure_contact_pairs.contains(*pair))
            .count();
        if self.observed_procedure_contact_pairs.len() + new_pair_count > MAX_OPEN_QUESTION_FACTS {
            return Err(OpenQuestionCampaignError(format!(
                "open-question campaign exceeds {MAX_OPEN_QUESTION_FACTS} semantic facts"
            )));
        }
        self.observed_procedure_contact_pairs.extend(pairs);
        self.observed_episodes = self.observed_episodes.saturating_add(1);
        Ok(())
    }

    pub fn assess_and_record(
        &mut self,
        descriptor: &SemanticNoveltyDescriptor,
    ) -> Result<OpenQuestionAssessment, OpenQuestionCampaignError> {
        let assessment = self.assess(descriptor)?;
        self.record(descriptor)?;
        Ok(assessment)
    }
}

fn procedure_contact_pairs(
    descriptor: &SemanticNoveltyDescriptor,
) -> BTreeSet<ProcedureContactPair> {
    descriptor
        .state_combinations
        .iter()
        .flat_map(|combination| {
            let procedure = combination.state.player_procedure;
            combination
                .contact
                .iter()
                .flat_map(|contact| &contact.surfaces)
                .filter_map(move |contact| {
                    procedure.map(|procedure| ProcedureContactPair {
                        procedure,
                        contact: contact.clone(),
                    })
                })
        })
        .collect()
}

fn collision_exit_contacts(descriptor: &SemanticNoveltyDescriptor) -> Vec<ContactFact> {
    descriptor
        .contact_sequence
        .iter()
        .flat_map(|contact| &contact.surfaces)
        .filter(|contact| contact.destination.is_some())
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn validate_spec(spec: &OpenQuestionCampaignSpec) -> Result<(), OpenQuestionCampaignError> {
    if spec.schema != OPEN_QUESTION_CAMPAIGN_SCHEMA
        || spec.name.trim().is_empty()
        || spec.name.len() > 192
        || spec.name.chars().any(char::is_control)
        || spec.maximum_episodes == 0
        || spec.maximum_episodes > MAX_OPEN_QUESTION_EPISODES
        || spec.partition.scenario_name.trim().is_empty()
    {
        return Err(OpenQuestionCampaignError(
            "open-question campaign spec is invalid or unbounded".into(),
        ));
    }
    for (label, digest) in [
        ("scenario", &spec.partition.scenario_identity),
        ("fidelity", &spec.partition.fidelity_identity),
    ] {
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(OpenQuestionCampaignError(format!(
                "campaign {label} identity is not lowercase SHA-256"
            )));
        }
    }
    Ok(())
}

fn campaign_identity(spec: &OpenQuestionCampaignSpec) -> String {
    let encoded = serde_json::to_vec(spec).expect("campaign spec is serializable");
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight-open-question-campaign/v1\0");
    hasher.update((encoded.len() as u64).to_le_bytes());
    hasher.update(encoded);
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::semantic_novelty::archive::{DiscoveryArchivePartitionKey, DiscoveryFidelity};
    use crate::semantic_novelty::{
        ContactState, EventFact, FlagState, SemanticState, SemanticStateCombination,
    };

    fn contact(destination: bool) -> ContactFact {
        ContactFact {
            kind: "wall",
            wall_slot: 0,
            backing: Some("dzb"),
            bg_index: Some(1),
            poly_index: Some(2),
            material_row: Some(3),
            group_row: Some(4),
            raw_exit_id: Some(5),
            source_room: Some(1),
            destination: destination.then(|| ("F_SP105".into(), 0, 0, 0)),
        }
    }

    fn descriptor(procedure: u16, destination: bool) -> SemanticNoveltyDescriptor {
        let contact = ContactState {
            background_flags: Some(1),
            surfaces: vec![contact(destination)],
        };
        let state = SemanticState {
            stage: "F_SP104".into(),
            room: 1,
            layer: 0,
            point: 0,
            player_procedure: Some(procedure),
        };
        SemanticNoveltyDescriptor {
            schema: super::super::SEMANTIC_NOVELTY_SCHEMA,
            procedure_sequence: vec![Some(procedure)],
            event_sequence: vec![EventFact {
                event_id: -1,
                mode: 0,
                status: 0,
                map_tool_id: 0,
                name_hash: None,
            }],
            state_transitions: Vec::new(),
            contact_sequence: vec![contact.clone()],
            actor_relationship_sequence: Vec::new(),
            flag_sequence: vec![FlagState {
                record_flags: 0,
                player_mode_flags: None,
                event_status: 0,
                event_mode: 0,
                goal_configured: None,
                goal_reached: None,
            }],
            state_combinations: vec![SemanticStateCombination {
                state,
                event: EventFact {
                    event_id: -1,
                    mode: 0,
                    status: 0,
                    map_tool_id: 0,
                    name_hash: None,
                },
                contact: Some(contact),
                actor_relationships: None,
                flags: FlagState {
                    record_flags: 0,
                    player_mode_flags: None,
                    event_status: 0,
                    event_mode: 0,
                    goal_configured: None,
                    goal_reached: None,
                },
            }],
            kinematic_extrema: None,
            boundary_fingerprints: Vec::new(),
        }
    }

    fn campaign(question: OpenQuestion) -> OpenQuestionCampaign {
        OpenQuestionCampaign::new(OpenQuestionCampaignSpec {
            schema: OPEN_QUESTION_CAMPAIGN_SCHEMA,
            name: "contact experiment".into(),
            partition: DiscoveryArchivePartitionKey {
                scenario_name: "intro".into(),
                scenario_identity: "11".repeat(32),
                fidelity: DiscoveryFidelity::Headless,
                fidelity_identity: "22".repeat(32),
            },
            question,
            maximum_episodes: 100,
        })
        .unwrap()
    }

    #[test]
    fn procedure_contact_pair_is_unseen_only_before_recording() {
        let descriptor = descriptor(7, false);
        let mut campaign = campaign(OpenQuestion::ProduceUnseenProcedureContactPair);
        let first = campaign.assess_and_record(&descriptor).unwrap();
        assert!(first.matched);
        assert!(matches!(
            first.evidence,
            OpenQuestionEvidence::UnseenProcedureContactPairs { ref pairs } if pairs.len() == 1
        ));
        assert!(!campaign.assess(&descriptor).unwrap().matched);
        assert!(!first.promotion_authority);
    }

    #[test]
    fn collision_exit_without_scene_transition_matches_raw_question() {
        let campaign = campaign(OpenQuestion::CrossCollisionWithoutSceneTransition);
        let assessment = campaign.assess(&descriptor(3, true)).unwrap();
        assert!(assessment.matched);
        assert!(matches!(
            assessment.evidence,
            OpenQuestionEvidence::CollisionExitWithoutSceneTransition { ref contacts }
                if contacts.len() == 1
        ));
    }

    #[test]
    fn actual_scene_transition_disqualifies_crossing_without_transition() {
        let mut descriptor = descriptor(3, true);
        let from = descriptor.state_combinations[0].state.clone();
        descriptor
            .state_transitions
            .push(super::super::StateTransitionFact {
                from: from.clone(),
                to: super::super::SemanticState {
                    stage: "F_SP105".into(),
                    ..from
                },
            });
        let campaign = campaign(OpenQuestion::CrossCollisionWithoutSceneTransition);
        assert!(!campaign.assess(&descriptor).unwrap().matched);
    }

    #[test]
    fn contact_change_with_identical_semantic_state_is_reported() {
        let mut descriptor = descriptor(3, false);
        let mut changed = descriptor.state_combinations[0].clone();
        changed.contact.as_mut().unwrap().background_flags = Some(9);
        descriptor.state_combinations.push(changed);
        let campaign = campaign(OpenQuestion::ContactChangeWithoutSemanticTransition);
        assert!(matches!(
            campaign.assess(&descriptor).unwrap().evidence,
            OpenQuestionEvidence::ContactChangesWithoutSemanticTransition { ref changes }
                if changes.len() == 1
        ));
    }
}
