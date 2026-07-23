//! Canonical coverage of every candidate transition's feasibility stages.

use crate::PlannerContractError;
use crate::artifact::Digest;
use crate::transition::{MechanicsCatalog, ObligationStage, StateOperation, TransitionKind};
use crate::{canonical_json, require_canonical_json_bytes, validate_stable_id};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;

pub const OBLIGATION_COVERAGE_SCHEMA: &str = "dusklight.route-planner.obligation-coverage/v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObligationCoverageReport {
    pub schema: String,
    pub mechanics_catalog_sha256: Digest,
    pub transitions: Vec<TransitionObligationCoverage>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TransitionObligationCoverage {
    pub transition_id: String,
    pub transition_kind: TransitionKind,
    pub reach_obligation_ids: Vec<String>,
    pub activation_obligation_ids: Vec<String>,
    pub effect_obligation_ids: Vec<String>,
    pub interruption_obligation_ids: Vec<String>,
    pub unknown_requirement_ids: Vec<String>,
    pub effect_operation_count: usize,
    pub interruption_action_ids: Vec<String>,
    pub interruption_microtrace_ids: Vec<String>,
}

impl ObligationCoverageReport {
    pub fn build(mechanics: &MechanicsCatalog) -> Result<Self, PlannerContractError> {
        mechanics.validate()?;
        let mut transitions = Vec::with_capacity(mechanics.transitions.len());
        for transition in &mechanics.transitions {
            let mut reach_obligation_ids = Vec::new();
            let mut activation_obligation_ids = Vec::new();
            let mut effect_obligation_ids = Vec::new();
            let mut interruption_obligation_ids = Vec::new();
            for obligation_id in &transition.activation.physical_obligation_ids {
                let obligation = mechanics
                    .obligations
                    .iter()
                    .find(|obligation| obligation.id == *obligation_id)
                    .expect("validated catalogs contain every referenced obligation");
                match obligation.stage {
                    ObligationStage::Reach => reach_obligation_ids.push(obligation_id.clone()),
                    ObligationStage::Activate => {
                        activation_obligation_ids.push(obligation_id.clone())
                    }
                    ObligationStage::Effect => effect_obligation_ids.push(obligation_id.clone()),
                    ObligationStage::Interrupt => {
                        interruption_obligation_ids.push(obligation_id.clone())
                    }
                }
            }
            let mut unknown_requirement_ids = transition
                .activation
                .unknown_requirements
                .iter()
                .map(|requirement| requirement.id.clone())
                .collect::<Vec<_>>();
            unknown_requirement_ids.sort();
            unknown_requirement_ids.dedup();
            let mut interruption_action_ids = transition
                .activation
                .effects
                .iter()
                .filter_map(|operation| match operation {
                    StateOperation::Interrupt { action_id, .. } => Some(action_id.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            interruption_action_ids.sort();
            interruption_action_ids.dedup();
            let mut interruption_microtrace_ids = Vec::new();
            for obligation_id in &interruption_obligation_ids {
                let obligation = mechanics
                    .obligations
                    .iter()
                    .find(|obligation| obligation.id == *obligation_id)
                    .expect("validated catalogs contain every referenced obligation");
                if let crate::transition::ObligationDetail::Temporal { requirement, .. } =
                    &obligation.detail
                {
                    interruption_microtrace_ids.extend(
                        mechanics
                            .microtraces
                            .iter()
                            .filter(|microtrace| microtrace.witnesses(requirement))
                            .map(|microtrace| microtrace.id.clone()),
                    );
                }
            }
            interruption_microtrace_ids.sort();
            interruption_microtrace_ids.dedup();
            transitions.push(TransitionObligationCoverage {
                transition_id: transition.id.clone(),
                transition_kind: transition.transition_kind,
                reach_obligation_ids,
                activation_obligation_ids,
                effect_obligation_ids,
                interruption_obligation_ids,
                unknown_requirement_ids,
                effect_operation_count: transition.activation.effects.len(),
                interruption_action_ids,
                interruption_microtrace_ids,
            });
        }
        let report = Self {
            schema: OBLIGATION_COVERAGE_SCHEMA.into(),
            mechanics_catalog_sha256: mechanics.digest()?,
            transitions,
        };
        report.validate()?;
        Ok(report)
    }

    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != OBLIGATION_COVERAGE_SCHEMA
            || self.mechanics_catalog_sha256 == Digest::ZERO
            || self
                .transitions
                .windows(2)
                .any(|pair| pair[0].transition_id >= pair[1].transition_id)
        {
            return Err(PlannerContractError::new(
                "obligation_coverage",
                "has invalid schema, catalog identity, or transition order",
            ));
        }
        for row in &self.transitions {
            validate_stable_id("obligation_coverage.transition_id", &row.transition_id)?;
            for ids in [
                &row.reach_obligation_ids,
                &row.activation_obligation_ids,
                &row.effect_obligation_ids,
                &row.interruption_obligation_ids,
                &row.unknown_requirement_ids,
                &row.interruption_action_ids,
                &row.interruption_microtrace_ids,
            ] {
                validate_sorted_ids(ids)?;
            }
            let stage_ids = [
                &row.reach_obligation_ids,
                &row.activation_obligation_ids,
                &row.effect_obligation_ids,
                &row.interruption_obligation_ids,
            ]
            .into_iter()
            .flat_map(|ids| ids.iter())
            .collect::<BTreeSet<_>>();
            let stage_count = row.reach_obligation_ids.len()
                + row.activation_obligation_ids.len()
                + row.effect_obligation_ids.len()
                + row.interruption_obligation_ids.len();
            if stage_ids.len() != stage_count
                || (!row.effect_obligation_ids.is_empty() && row.effect_operation_count == 0)
            {
                return Err(PlannerContractError::new(
                    "obligation_coverage.transitions",
                    "contains duplicate or inapplicable staged obligations",
                ));
            }
        }
        Ok(())
    }

    pub fn validate_against(
        &self,
        mechanics: &MechanicsCatalog,
    ) -> Result<(), PlannerContractError> {
        self.validate()?;
        if self != &Self::build(mechanics)? {
            return Err(PlannerContractError::new(
                "obligation_coverage",
                "does not exactly cover the supplied mechanics catalog",
            ));
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PlannerContractError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn digest(&self) -> Result<Digest, PlannerContractError> {
        Ok(Digest(Sha256::digest(self.canonical_bytes()?).into()))
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, PlannerContractError> {
        let report: Self = serde_json::from_slice(bytes)?;
        report.validate()?;
        require_canonical_json_bytes("obligation_coverage", bytes, &report.canonical_bytes()?)?;
        Ok(report)
    }
}

fn validate_sorted_ids(ids: &[String]) -> Result<(), PlannerContractError> {
    for id in ids {
        validate_stable_id("obligation_coverage.ids", id)?;
    }
    if ids.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(PlannerContractError::new(
            "obligation_coverage.ids",
            "must be unique and sorted",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{ContextSelector, ExactContext};
    use crate::logic::{ContextScope, EvidenceRecord, RuleEvidence, TruthStatus};
    use crate::transition::{
        ActivationContract, CandidateTransition, FeasibilityObligation, MECHANICS_CATALOG_SCHEMA,
        ObligationDetail, ObligationKind, TemporalRequirement, TemporalWindow, UnknownRequirement,
    };

    fn evidence() -> RuleEvidence {
        RuleEvidence {
            truth: TruthStatus::Established,
            records: vec![EvidenceRecord {
                id: "evidence.fixture".into(),
                kind: crate::logic::EvidenceKind::SourceAudited,
                source_sha256: Some(Digest([7; 32])),
                note: "Fixture".into(),
            }],
        }
    }

    fn scope() -> ContextScope {
        ContextScope {
            selectors: vec![ContextSelector::Exact {
                context: ExactContext {
                    content_sha256: Digest([1; 32]),
                    runtime_configuration_sha256: Digest([2; 32]),
                },
            }],
        }
    }

    fn catalog() -> MechanicsCatalog {
        let timing = TemporalWindow {
            earliest_frame: 4,
            latest_frame: 4,
            required_input: Some("map".into()),
        };
        MechanicsCatalog {
            schema: MECHANICS_CATALOG_SCHEMA.into(),
            transitions: vec![CandidateTransition {
                id: "transition.interrupted-write".into(),
                label: "Interrupt after the state write".into(),
                scope: scope(),
                transition_kind: TransitionKind::MessageAction,
                approach_id: "approach.dialogue".into(),
                activation: ActivationContract {
                    hard_guards: crate::logic::PredicateExpression::True,
                    physical_obligation_ids: vec![
                        "obligation.activate".into(),
                        "obligation.effect".into(),
                        "obligation.interrupt".into(),
                        "obligation.reach".into(),
                    ],
                    effects: vec![StateOperation::Interrupt {
                        action_id: "dialogue.fixture".into(),
                        window: timing.clone(),
                    }],
                    unknown_requirements: vec![UnknownRequirement {
                        id: "unknown.cleanup".into(),
                        description: "Cleanup after the observed boundary is unknown.".into(),
                        evidence: evidence(),
                    }],
                },
                evidence: evidence(),
            }],
            obligations: vec![
                FeasibilityObligation {
                    id: "obligation.activate".into(),
                    label: "Activate the interaction".into(),
                    scope: scope(),
                    obligation_kind: ObligationKind::Interaction,
                    stage: ObligationStage::Activate,
                    detail: ObligationDetail::Unresolved {
                        research_question: "Can the interaction activate?".into(),
                    },
                    evidence: evidence(),
                },
                FeasibilityObligation {
                    id: "obligation.effect".into(),
                    label: "Commit the write".into(),
                    scope: scope(),
                    obligation_kind: ObligationKind::MessageState,
                    stage: ObligationStage::Effect,
                    detail: ObligationDetail::Unresolved {
                        research_question: "Does the write commit before interruption?".into(),
                    },
                    evidence: evidence(),
                },
                FeasibilityObligation {
                    id: "obligation.interrupt".into(),
                    label: "Hit the interruption window".into(),
                    scope: scope(),
                    obligation_kind: ObligationKind::Timing,
                    stage: ObligationStage::Interrupt,
                    detail: ObligationDetail::Temporal {
                        requirement: TemporalRequirement {
                            action_id: "dialogue.fixture".into(),
                            window: timing,
                        },
                        precondition: crate::logic::PredicateExpression::True,
                    },
                    evidence: evidence(),
                },
                FeasibilityObligation {
                    id: "obligation.reach".into(),
                    label: "Reach the interaction".into(),
                    scope: scope(),
                    obligation_kind: ObligationKind::Geometry,
                    stage: ObligationStage::Reach,
                    detail: ObligationDetail::Geometry {
                        approach_id: "approach.dialogue".into(),
                        source_region_id: "region.source".into(),
                        destination_region_id: "region.dialogue".into(),
                    },
                    evidence: evidence(),
                },
            ],
            writers: Vec::new(),
            gates: Vec::new(),
            readers: Vec::new(),
            reconstruction_rules: Vec::new(),
            obstructions: Vec::new(),
            resolvers: Vec::new(),
            techniques: Vec::new(),
            microtraces: Vec::new(),
            goals: Vec::new(),
        }
    }

    #[test]
    fn covers_every_transition_stage_and_round_trips_canonically() {
        let report = ObligationCoverageReport::build(&catalog()).unwrap();
        let row = &report.transitions[0];
        assert_eq!(row.reach_obligation_ids, ["obligation.reach"]);
        assert_eq!(row.activation_obligation_ids, ["obligation.activate"]);
        assert_eq!(row.effect_obligation_ids, ["obligation.effect"]);
        assert_eq!(row.interruption_obligation_ids, ["obligation.interrupt"]);
        assert_eq!(row.interruption_action_ids, ["dialogue.fixture"]);
        let bytes = report.canonical_bytes().unwrap();
        let decoded = ObligationCoverageReport::decode_canonical(&bytes).unwrap();
        decoded.validate_against(&catalog()).unwrap();
    }

    #[test]
    fn rejects_cross_approach_and_inapplicable_stage_bindings() {
        let mut wrong_approach = catalog();
        let ObligationDetail::Geometry { approach_id, .. } =
            &mut wrong_approach.obligations[3].detail
        else {
            unreachable!();
        };
        *approach_id = "approach.other".into();
        assert_eq!(
            wrong_approach.validate().unwrap_err().field(),
            "transitions.activation.physical_obligation_ids"
        );

        let mut no_interrupt = catalog();
        no_interrupt.transitions[0].activation.effects.clear();
        assert_eq!(
            no_interrupt.validate().unwrap_err().field(),
            "transitions.activation.physical_obligation_ids"
        );
    }
}
