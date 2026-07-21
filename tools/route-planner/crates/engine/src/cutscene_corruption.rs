//! Conservative research hypothesis for actor-corruption cutscene failure.
//!
//! The hypothesis can produce only a named resource-failure predicate. It has
//! no scene-location, runtime-flag, or return-place effect and remains unknown
//! until a witnessed corruption trace establishes its failure site and prefix.

use crate::artifact::Digest;
use crate::cutscene_outer::{CutsceneOuterRuntimeProfile, ResolvedCutsceneOuterEvent};
use crate::identity::{ContentIdentity, ContextSelector, ExactContext, RuntimeConfiguration};
use crate::logic::{
    ComparisonOperator, ContextScope, EvidenceKind, EvidenceRecord, PredicateExpression,
    RuleEvidence, TruthStatus, ValueReference,
};
use crate::state::StateValue;
use crate::transition::{
    ActivationContract, CandidateTransition, ComponentFieldTarget, StateOperation, TransitionKind,
    UnknownRequirement,
};
use crate::{PlannerContractError, canonical_json, validate_stable_id};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

pub const CUTSCENE_CORRUPTION_HYPOTHESIS_SCHEMA: &str =
    "dusklight.route-planner.cutscene-corruption-hypothesis/v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CutsceneCorruptionHypothesis {
    pub schema: String,
    pub id: String,
    pub content_sha256: Digest,
    pub runtime_configuration_sha256: Digest,
    pub outer_event_sha256: Digest,
    pub outer_runtime_profile_sha256: Digest,
    pub source_flow_component_id: String,
    pub source_flow_node: String,
    pub produced_failure_field: String,
    pub produced_failure_value: String,
    pub producer: CandidateTransition,
    pub coverage: CutsceneCorruptionCoverage,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CutsceneCorruptionCoverageStatus {
    Resolved,
    Unresolved,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CutsceneCorruptionCoverage {
    pub indirect_failure_predicate_model: CutsceneCorruptionCoverageStatus,
    pub direct_location_effect_absent: CutsceneCorruptionCoverageStatus,
    pub direct_return_place_effect_absent: CutsceneCorruptionCoverageStatus,
    pub actual_failure_site: CutsceneCorruptionCoverageStatus,
    pub all_stb_lookups_missing_production: CutsceneCorruptionCoverageStatus,
    pub last_completed_operation_and_prefix: CutsceneCorruptionCoverageStatus,
    pub runtime_dispatch_flags: CutsceneCorruptionCoverageStatus,
}

impl CutsceneCorruptionHypothesis {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != CUTSCENE_CORRUPTION_HYPOTHESIS_SCHEMA {
            return Err(PlannerContractError::new(
                "cutscene_corruption_hypothesis.schema",
                "is unsupported",
            ));
        }
        validate_stable_id("cutscene_corruption_hypothesis.id", &self.id)?;
        for (field, value) in [
            ("source_flow_component_id", &self.source_flow_component_id),
            ("source_flow_node", &self.source_flow_node),
            ("produced_failure_field", &self.produced_failure_field),
            ("produced_failure_value", &self.produced_failure_value),
        ] {
            validate_stable_id(&format!("cutscene_corruption_hypothesis.{field}"), value)?;
        }
        if [
            self.content_sha256,
            self.runtime_configuration_sha256,
            self.outer_event_sha256,
            self.outer_runtime_profile_sha256,
        ]
        .contains(&Digest::ZERO)
        {
            return Err(PlannerContractError::new(
                "cutscene_corruption_hypothesis.identity",
                "must retain nonzero exact input identities",
            ));
        }
        self.producer.validate()?;
        let expected_scope = ContextScope {
            selectors: vec![ContextSelector::Exact {
                context: ExactContext {
                    content_sha256: self.content_sha256,
                    runtime_configuration_sha256: self.runtime_configuration_sha256,
                },
            }],
        };
        let expected_guard = PredicateExpression::Compare {
            left: ValueReference::FlowNode {
                flow_component_id: self.source_flow_component_id.clone(),
            },
            operator: ComparisonOperator::Equal,
            right: ValueReference::Literal {
                value: StateValue::Text(self.source_flow_node.clone()),
            },
        };
        let expected_effect = StateOperation::Write {
            target: ComponentFieldTarget {
                component_id: self.source_flow_component_id.clone(),
                field: self.produced_failure_field.clone(),
            },
            value: StateValue::Text(self.produced_failure_value.clone()),
        };
        if self.producer.scope != expected_scope
            || self.producer.transition_kind != TransitionKind::ResourceLoadFailure
            || self.producer.activation.hard_guards != expected_guard
            || !self.producer.activation.physical_obligation_ids.is_empty()
            || self.producer.activation.effects != [expected_effect]
            || self.producer.evidence.truth != TruthStatus::Unknown
        {
            return Err(PlannerContractError::new(
                "cutscene_corruption_hypothesis.producer",
                "must produce only the unknown failure predicate in the exact context",
            ));
        }
        let expected_unknowns = [
            "unknown.cutscene-corruption.actual-failure-site",
            "unknown.cutscene-corruption.all-stb-lookups-missing",
            "unknown.cutscene-corruption.last-completed-operation",
        ];
        if self.producer.activation.unknown_requirements.len() != expected_unknowns.len()
            || self
                .producer
                .activation
                .unknown_requirements
                .iter()
                .zip(expected_unknowns)
                .any(|(unknown, expected)| {
                    unknown.id != expected || unknown.evidence.truth != TruthStatus::Unknown
                })
        {
            return Err(PlannerContractError::new(
                "cutscene_corruption_hypothesis.producer.unknown_requirements",
                "must retain the unresolved site, predicate, and prefix boundaries",
            ));
        }
        let expected_coverage = CutsceneCorruptionCoverage {
            indirect_failure_predicate_model: CutsceneCorruptionCoverageStatus::Resolved,
            direct_location_effect_absent: CutsceneCorruptionCoverageStatus::Resolved,
            direct_return_place_effect_absent: CutsceneCorruptionCoverageStatus::Resolved,
            actual_failure_site: CutsceneCorruptionCoverageStatus::Unresolved,
            all_stb_lookups_missing_production: CutsceneCorruptionCoverageStatus::Unresolved,
            last_completed_operation_and_prefix: CutsceneCorruptionCoverageStatus::Unresolved,
            runtime_dispatch_flags: CutsceneCorruptionCoverageStatus::Unresolved,
        };
        if self.coverage != expected_coverage {
            return Err(PlannerContractError::new(
                "cutscene_corruption_hypothesis.coverage",
                "must not promote unwitnessed corruption behavior",
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
        let hypothesis: Self = serde_json::from_slice(bytes)?;
        hypothesis.validate()?;
        if hypothesis.canonical_bytes()? != bytes {
            return Err(PlannerContractError::new(
                "cutscene_corruption_hypothesis",
                "is not canonical JSON",
            ));
        }
        Ok(hypothesis)
    }
}

pub fn compile_actor_corruption_hypothesis(
    content: &ContentIdentity,
    runtime: &RuntimeConfiguration,
    outer_event: &ResolvedCutsceneOuterEvent,
    outer_profile: &CutsceneOuterRuntimeProfile,
) -> Result<CutsceneCorruptionHypothesis, PlannerContractError> {
    content.validate()?;
    runtime.validate()?;
    outer_event.validate()?;
    outer_profile.validate()?;
    let content_sha256 = content.digest()?;
    let runtime_configuration_sha256 = runtime.digest()?;
    let outer_event_sha256 = outer_event.digest()?;
    let outer_runtime_profile_sha256 = outer_profile.digest()?;
    if runtime.content_sha256 != content_sha256
        || outer_event.content_sha256 != content_sha256
        || outer_event.runtime_configuration_sha256 != runtime_configuration_sha256
        || outer_event.runtime_profile_sha256 != outer_runtime_profile_sha256
        || outer_profile.content_sha256 != content_sha256
    {
        return Err(PlannerContractError::new(
            "cutscene_corruption_hypothesis.identity",
            "does not match the exact content, runtime, outer event, and outer profile",
        ));
    }
    let binding = &outer_profile.state_binding;
    let report = EvidenceRecord {
        id: "community.actor-corruption.post-zelda".into(),
        kind: EvidenceKind::CommunityReported,
        source_sha256: None,
        note: "Actor corruption is reported to skip the post-Zelda cutscene, but the exact failed resource, completed prefix, runtime flags, and return-place suffix are not yet witnessed.".into(),
    };
    let unknown = |id: &str, description: &str| UnknownRequirement {
        id: id.into(),
        description: description.into(),
        evidence: RuleEvidence {
            truth: TruthStatus::Unknown,
            records: vec![report.clone()],
        },
    };
    let producer = CandidateTransition {
        id: "transition.cutscene.demo07_02.actor-corruption.failure-hypothesis".into(),
        label: "Hypothesize actor corruption produces the all-STB-lookups-missing predicate".into(),
        scope: ContextScope {
            selectors: vec![ContextSelector::Exact {
                context: ExactContext {
                    content_sha256,
                    runtime_configuration_sha256,
                },
            }],
        },
        transition_kind: TransitionKind::ResourceLoadFailure,
        approach_id: "cutscene.demo07_02.actor-corruption".into(),
        activation: ActivationContract {
            hard_guards: PredicateExpression::Compare {
                left: ValueReference::FlowNode {
                    flow_component_id: binding.flow_component_id.clone(),
                },
                operator: ComparisonOperator::Equal,
                right: ValueReference::Literal {
                    value: StateValue::Text(binding.package_play_node.clone()),
                },
            },
            physical_obligation_ids: Vec::new(),
            effects: vec![StateOperation::Write {
                target: ComponentFieldTarget {
                    component_id: binding.flow_component_id.clone(),
                    field: binding.stb_lookup_result_field.clone(),
                },
                value: StateValue::Text(binding.all_stb_lookups_missing_value.clone()),
            }],
            unknown_requirements: vec![
                unknown(
                    "unknown.cutscene-corruption.actual-failure-site",
                    "Identify the exact corrupted actor/resource and failure site.",
                ),
                unknown(
                    "unknown.cutscene-corruption.all-stb-lookups-missing",
                    "Verify that the observed setup causes every STB lookup to miss rather than a later partial actor-resource failure.",
                ),
                unknown(
                    "unknown.cutscene-corruption.last-completed-operation",
                    "Witness the last completed operation and every prefix write before failure.",
                ),
            ],
        },
        evidence: RuleEvidence {
            truth: TruthStatus::Unknown,
            records: vec![report],
        },
    };
    let hypothesis = CutsceneCorruptionHypothesis {
        schema: CUTSCENE_CORRUPTION_HYPOTHESIS_SCHEMA.into(),
        id: "gz2e01-demo07-02-actor-corruption".into(),
        content_sha256,
        runtime_configuration_sha256,
        outer_event_sha256,
        outer_runtime_profile_sha256,
        source_flow_component_id: binding.flow_component_id.clone(),
        source_flow_node: binding.package_play_node.clone(),
        produced_failure_field: binding.stb_lookup_result_field.clone(),
        produced_failure_value: binding.all_stb_lookups_missing_value.clone(),
        producer,
        coverage: CutsceneCorruptionCoverage {
            indirect_failure_predicate_model: CutsceneCorruptionCoverageStatus::Resolved,
            direct_location_effect_absent: CutsceneCorruptionCoverageStatus::Resolved,
            direct_return_place_effect_absent: CutsceneCorruptionCoverageStatus::Resolved,
            actual_failure_site: CutsceneCorruptionCoverageStatus::Unresolved,
            all_stb_lookups_missing_production: CutsceneCorruptionCoverageStatus::Unresolved,
            last_completed_operation_and_prefix: CutsceneCorruptionCoverageStatus::Unresolved,
            runtime_dispatch_flags: CutsceneCorruptionCoverageStatus::Unresolved,
        },
    };
    hypothesis.validate()?;
    Ok(hypothesis)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evaluation::EvidencePolicy;

    #[test]
    fn hypothesis_rejects_a_direct_location_effect() {
        let mut hypothesis = fixture();
        hypothesis
            .producer
            .activation
            .effects
            .push(StateOperation::SetLocation {
                location: crate::state::SceneLocation {
                    stage: "F_SP116".into(),
                    room: 0,
                    layer: 8,
                    spawn: 20,
                },
            });
        assert_eq!(
            hypothesis.validate().unwrap_err().field(),
            "cutscene_corruption_hypothesis.producer"
        );
    }

    #[test]
    fn unknown_producer_is_not_admitted_by_established_or_research_evidence_policy() {
        let hypothesis = fixture();
        assert!(!EvidencePolicy::ESTABLISHED_ONLY.permits(hypothesis.producer.evidence.truth));
        assert!(!EvidencePolicy::RESEARCH.permits(hypothesis.producer.evidence.truth));
        assert_eq!(hypothesis.producer.activation.unknown_requirements.len(), 3);
    }

    fn fixture() -> CutsceneCorruptionHypothesis {
        let report = EvidenceRecord {
            id: "community.actor-corruption.post-zelda".into(),
            kind: EvidenceKind::CommunityReported,
            source_sha256: None,
            note: "Reported but unwitnessed corruption behavior.".into(),
        };
        let unknown = |id: &str| UnknownRequirement {
            id: id.into(),
            description: "Unresolved evidence boundary.".into(),
            evidence: RuleEvidence {
                truth: TruthStatus::Unknown,
                records: vec![report.clone()],
            },
        };
        CutsceneCorruptionHypothesis {
            schema: CUTSCENE_CORRUPTION_HYPOTHESIS_SCHEMA.into(),
            id: "fixture-corruption".into(),
            content_sha256: Digest([1; 32]),
            runtime_configuration_sha256: Digest([2; 32]),
            outer_event_sha256: Digest([3; 32]),
            outer_runtime_profile_sha256: Digest([4; 32]),
            source_flow_component_id: "cutscene.runtime".into(),
            source_flow_node: "package.play".into(),
            produced_failure_field: "package.stb-result".into(),
            produced_failure_value: "all-lookups-missing".into(),
            producer: CandidateTransition {
                id: "transition.fixture.corruption".into(),
                label: "Hypothesize corruption failure".into(),
                scope: ContextScope {
                    selectors: vec![ContextSelector::Exact {
                        context: ExactContext {
                            content_sha256: Digest([1; 32]),
                            runtime_configuration_sha256: Digest([2; 32]),
                        },
                    }],
                },
                transition_kind: TransitionKind::ResourceLoadFailure,
                approach_id: "cutscene.fixture.corruption".into(),
                activation: ActivationContract {
                    hard_guards: PredicateExpression::Compare {
                        left: ValueReference::FlowNode {
                            flow_component_id: "cutscene.runtime".into(),
                        },
                        operator: ComparisonOperator::Equal,
                        right: ValueReference::Literal {
                            value: StateValue::Text("package.play".into()),
                        },
                    },
                    physical_obligation_ids: Vec::new(),
                    effects: vec![StateOperation::Write {
                        target: ComponentFieldTarget {
                            component_id: "cutscene.runtime".into(),
                            field: "package.stb-result".into(),
                        },
                        value: StateValue::Text("all-lookups-missing".into()),
                    }],
                    unknown_requirements: vec![
                        unknown("unknown.cutscene-corruption.actual-failure-site"),
                        unknown("unknown.cutscene-corruption.all-stb-lookups-missing"),
                        unknown("unknown.cutscene-corruption.last-completed-operation"),
                    ],
                },
                evidence: RuleEvidence {
                    truth: TruthStatus::Unknown,
                    records: vec![report],
                },
            },
            coverage: CutsceneCorruptionCoverage {
                indirect_failure_predicate_model: CutsceneCorruptionCoverageStatus::Resolved,
                direct_location_effect_absent: CutsceneCorruptionCoverageStatus::Resolved,
                direct_return_place_effect_absent: CutsceneCorruptionCoverageStatus::Resolved,
                actual_failure_site: CutsceneCorruptionCoverageStatus::Unresolved,
                all_stb_lookups_missing_production: CutsceneCorruptionCoverageStatus::Unresolved,
                last_completed_operation_and_prefix: CutsceneCorruptionCoverageStatus::Unresolved,
                runtime_dispatch_flags: CutsceneCorruptionCoverageStatus::Unresolved,
            },
        }
    }
}
