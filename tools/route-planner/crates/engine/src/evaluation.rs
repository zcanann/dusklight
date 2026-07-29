//! Loss-aware predicate evaluation and transition readiness classification.

use crate::identity::{ConfigurationValue, ContextSelector, EquivalenceSet, ExactContext};
use crate::logic::{
    ComparisonOperator, ContextScope, FactCatalog, PredicateExpression, RawFactBinding,
    TruthStatus, ValueReference,
};
use crate::state::{
    ActorLifecycle, ComponentKind, ComponentPayload, ExecutionContext, PlaneRelation, PlayerForm,
    PlayerMount, RuntimeFileOrigin, SpatialConnectionStatus, SpatialVolumeShape, StateComponent,
    StateValue,
};
use crate::transition::{
    ActorReconstructionRule, CandidateTransition, FeasibilityObligation, GateRule,
    InteractionPosition, ObligationDetail, ReaderRule, TemporalRequirement, VolumeReference,
    WitnessedMicrotrace, WriterRule,
};
use crate::transition::{Obstruction, ObstructionResolver, Technique};
use crate::{PlannerContractError, validate_stable_id};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvaluatedTruth {
    True,
    False,
    Unknown,
}

impl EvaluatedTruth {
    fn not(self) -> Self {
        match self {
            Self::True => Self::False,
            Self::False => Self::True,
            Self::Unknown => Self::Unknown,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EvidencePolicy {
    pub allow_contested: bool,
    pub allow_hypothetical: bool,
}

impl EvidencePolicy {
    pub const ESTABLISHED_ONLY: Self = Self {
        allow_contested: false,
        allow_hypothetical: false,
    };

    pub const RESEARCH: Self = Self {
        allow_contested: true,
        allow_hypothetical: true,
    };

    pub fn permits(self, truth: TruthStatus) -> bool {
        match truth {
            TruthStatus::Established => true,
            TruthStatus::Contested => self.allow_contested,
            TruthStatus::Hypothetical => self.allow_hypothetical,
            TruthStatus::Unknown => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FeasibilityMode {
    UpperBound,
    Modeled,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TransitionClassification {
    Inapplicable,
    GuardBlocked,
    FeasibilityUnknown,
    Obstructed,
    Executable,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TransitionAssessment {
    pub transition_id: String,
    pub classification: TransitionClassification,
    pub scope_applies: bool,
    pub evidence_permitted: bool,
    pub hard_guard: EvaluatedTruth,
    pub outstanding_obligation_ids: Vec<String>,
    pub unknown_obligation_ids: Vec<String>,
    pub unknown_requirement_ids: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObligationClassification {
    Inapplicable,
    EvidenceUnknown,
    Satisfied,
    Unsatisfied,
    EvaluationUnknown,
    Unmodeled,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObligationAssessment {
    pub obligation_id: String,
    pub classification: ObligationClassification,
    pub predicate: Option<EvaluatedTruth>,
    pub supporting_microtrace_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GateAssessment {
    pub gate_id: String,
    pub scope_applies: bool,
    pub evidence_permitted: bool,
    pub active: EvaluatedTruth,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WriterClassification {
    Inapplicable,
    Inactive,
    ActivationUnknown,
    GateBlocked,
    GateUnknown,
    Executable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WriterAssessment {
    pub writer_id: String,
    pub classification: WriterClassification,
    pub scope_applies: bool,
    pub evidence_permitted: bool,
    pub activation: EvaluatedTruth,
    pub active_gate_ids: Vec<String>,
    pub unknown_gate_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReaderAssessment {
    pub reader_id: String,
    pub scope_applies: bool,
    pub evidence_permitted: bool,
    pub source_value: Option<StateValue>,
    pub interpretation: Option<EvaluatedTruth>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleClassification {
    Inapplicable,
    EvidenceUnknown,
    Inactive,
    ActivationUnknown,
    Active,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObstructionAssessment {
    pub obstruction_id: String,
    pub classification: RuleClassification,
    pub activation: EvaluatedTruth,
    pub obligation_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolverAssessment {
    pub resolver_id: String,
    pub obstruction_id: String,
    pub classification: RuleClassification,
    pub applicability: EvaluatedTruth,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TechniqueAssessment {
    pub technique_id: String,
    pub classification: RuleClassification,
    pub prerequisites: EvaluatedTruth,
    pub discharged_obligation_ids: Vec<String>,
    pub introduced_obligation_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReconstructionAssessment {
    pub reconstruction_rule_id: String,
    pub classification: RuleClassification,
    pub activation: EvaluatedTruth,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeasibilityResolution {
    pub claimed_obligation_ids: BTreeSet<String>,
    pub discharged_obligation_ids: BTreeSet<String>,
    pub unknown_obligation_ids: BTreeSet<String>,
    pub supporting_microtrace_ids: BTreeSet<String>,
    pub active_obstruction_ids: Vec<String>,
    pub unknown_obstruction_ids: Vec<String>,
    pub applied_resolver_ids: Vec<String>,
    pub applicable_technique_ids: Vec<String>,
}

#[derive(Clone, Copy, Debug)]
pub struct FeasibilitySelection<'a> {
    pub resolver_ids: &'a BTreeSet<String>,
    pub technique_ids: &'a BTreeSet<String>,
    pub already_discharged: &'a BTreeSet<String>,
    pub microtraces: &'a [WitnessedMicrotrace],
}

/// Evaluates facts and guards against one immutable snapshot. Missing values,
/// unknown raw bits, unsupported equivalence selectors, and disallowed evidence
/// all stay `Unknown`; none are coerced to false.
pub struct PredicateEvaluator<'a> {
    snapshot: &'a crate::snapshot::StateSnapshot,
    facts: &'a FactCatalog,
    exact_context: ExactContext,
    equivalence_sets: BTreeMap<&'a str, &'a EquivalenceSet>,
    gate_states: BTreeMap<String, bool>,
    policy: EvidencePolicy,
}

impl<'a> PredicateEvaluator<'a> {
    fn world_execution_active(&self) -> bool {
        matches!(
            self.snapshot.environment.execution_context,
            ExecutionContext::World
        )
    }

    fn world_location(&self) -> Option<&crate::state::SceneLocation> {
        self.world_execution_active()
            .then_some(&self.snapshot.environment.location)
    }

    fn world_player(&self) -> Option<&crate::state::PlayerState> {
        self.world_execution_active()
            .then_some(&self.snapshot.environment.player)
    }

    fn component_readable(&self, component: &StateComponent) -> bool {
        component.component_kind != ComponentKind::ActorInstance || self.world_execution_active()
    }

    pub fn new(
        snapshot: &'a crate::snapshot::StateSnapshot,
        facts: &'a FactCatalog,
        equivalence_sets: &'a [EquivalenceSet],
        gate_states: &BTreeMap<String, bool>,
        policy: EvidencePolicy,
    ) -> Result<Self, PlannerContractError> {
        snapshot.validate()?;
        facts.validate()?;
        let exact_context = snapshot.environment.runtime_configuration.exact_context()?;
        let mut sets = BTreeMap::new();
        for set in equivalence_sets {
            set.validate()?;
            if sets.insert(set.id.as_str(), set).is_some() {
                return Err(PlannerContractError::new(
                    "equivalence_sets",
                    "contains a duplicate ID",
                ));
            }
        }
        for id in gate_states.keys() {
            validate_stable_id("gate_states.id", id)?;
        }
        Ok(Self {
            snapshot,
            facts,
            exact_context,
            equivalence_sets: sets,
            gate_states: gate_states.clone(),
            policy,
        })
    }

    pub fn evaluate(&self, expression: &PredicateExpression) -> EvaluatedTruth {
        let mut fact_stack = BTreeSet::new();
        let mut memo = BTreeMap::new();
        self.evaluate_inner(expression, &mut fact_stack, &mut memo)
    }

    pub fn scope_applies(&self, scope: &ContextScope) -> bool {
        scope.selectors.iter().any(|selector| match selector {
            ContextSelector::Exact { context } => context == &self.exact_context,
            ContextSelector::Equivalent { equivalence_set_id } => self
                .equivalence_sets
                .get(equivalence_set_id.as_str())
                .is_some_and(|set| set.proves(&self.exact_context)),
        })
    }

    pub fn assess_transition(
        &self,
        transition: &CandidateTransition,
        discharged_obligation_ids: &BTreeSet<String>,
        unknown_obligation_ids: &BTreeSet<String>,
        mode: FeasibilityMode,
    ) -> TransitionAssessment {
        let scope_applies = self.scope_applies(&transition.scope);
        let evidence_permitted = self.policy.permits(transition.evidence.truth);
        let hard_guard = if scope_applies && evidence_permitted {
            self.evaluate(&transition.activation.hard_guards)
        } else {
            EvaluatedTruth::Unknown
        };
        let outstanding_obligation_ids = transition
            .activation
            .physical_obligation_ids
            .iter()
            .filter(|id| !discharged_obligation_ids.contains(*id))
            .cloned()
            .collect::<Vec<_>>();
        let unknown_requirement_ids = transition
            .activation
            .unknown_requirements
            .iter()
            .map(|requirement| requirement.id.clone())
            .collect::<Vec<_>>();
        let unknown_obligation_ids = transition
            .activation
            .physical_obligation_ids
            .iter()
            .filter(|id| unknown_obligation_ids.contains(*id))
            .cloned()
            .collect::<Vec<_>>();
        let classification = if !scope_applies {
            TransitionClassification::Inapplicable
        } else if hard_guard == EvaluatedTruth::False {
            TransitionClassification::GuardBlocked
        } else if !evidence_permitted
            || hard_guard == EvaluatedTruth::Unknown
            || (mode == FeasibilityMode::Modeled && !unknown_obligation_ids.is_empty())
            || !unknown_requirement_ids.is_empty()
        {
            TransitionClassification::FeasibilityUnknown
        } else if mode == FeasibilityMode::Modeled && !outstanding_obligation_ids.is_empty() {
            TransitionClassification::Obstructed
        } else {
            TransitionClassification::Executable
        };
        TransitionAssessment {
            transition_id: transition.id.clone(),
            classification,
            scope_applies,
            evidence_permitted,
            hard_guard,
            outstanding_obligation_ids,
            unknown_obligation_ids,
            unknown_requirement_ids,
        }
    }

    pub fn assess_obligation(
        &self,
        obligation: &FeasibilityObligation,
        microtraces: &[WitnessedMicrotrace],
    ) -> ObligationAssessment {
        let mut supporting_microtrace_ids = Vec::new();
        let (classification, predicate) = if !self.scope_applies(&obligation.scope) {
            (ObligationClassification::Inapplicable, None)
        } else if !self.policy.permits(obligation.evidence.truth) {
            (ObligationClassification::EvidenceUnknown, None)
        } else {
            match &obligation.detail {
                ObligationDetail::Predicate { predicate } => {
                    let result = self.evaluate(predicate);
                    (
                        match result {
                            EvaluatedTruth::True => ObligationClassification::Satisfied,
                            EvaluatedTruth::False => ObligationClassification::Unsatisfied,
                            EvaluatedTruth::Unknown => ObligationClassification::EvaluationUnknown,
                        },
                        Some(result),
                    )
                }
                ObligationDetail::Interaction {
                    actor_instance_id,
                    required_volumes,
                    excluded_volumes,
                    pose_predicate,
                    temporal_requirement,
                    ..
                } => {
                    let pose = self.evaluate(pose_predicate);
                    let actor = self.interaction_actor_loaded(actor_instance_id);
                    let spatial = required_volumes
                        .iter()
                        .map(|volume| self.player_inside_volume(volume))
                        .chain(
                            excluded_volumes
                                .iter()
                                .map(|volume| self.player_inside_volume(volume).not()),
                        )
                        .fold(EvaluatedTruth::True, and_evaluated_truth);
                    let temporal = temporal_requirement
                        .as_ref()
                        .map_or((EvaluatedTruth::True, Vec::new()), |requirement| {
                            self.assess_temporal(requirement, microtraces)
                        });
                    supporting_microtrace_ids = temporal.1;
                    let combined = and_evaluated_truth(
                        and_evaluated_truth(and_evaluated_truth(pose, actor), spatial),
                        temporal.0,
                    );
                    (classify_obligation_truth(combined), Some(combined))
                }
                ObligationDetail::CompoundInteraction {
                    actor_instance_id,
                    branches,
                    temporal_requirement,
                    ..
                } => {
                    let actor = self.interaction_actor_loaded(actor_instance_id);
                    let branch_result = branches
                        .iter()
                        .map(|branch| match self.evaluate(&branch.when) {
                            EvaluatedTruth::False => EvaluatedTruth::False,
                            EvaluatedTruth::Unknown => EvaluatedTruth::Unknown,
                            EvaluatedTruth::True => branch
                                .volume_tests
                                .iter()
                                .map(|test| {
                                    let result = self.interaction_position_inside_volume(
                                        test.position,
                                        &test.volume,
                                    );
                                    if test.must_be_inside {
                                        result
                                    } else {
                                        result.not()
                                    }
                                })
                                .fold(self.evaluate(&branch.pose_predicate), and_evaluated_truth),
                        })
                        .fold(EvaluatedTruth::False, or_evaluated_truth);
                    let temporal = temporal_requirement
                        .as_ref()
                        .map_or((EvaluatedTruth::True, Vec::new()), |requirement| {
                            self.assess_temporal(requirement, microtraces)
                        });
                    supporting_microtrace_ids = temporal.1;
                    let combined =
                        and_evaluated_truth(and_evaluated_truth(actor, branch_result), temporal.0);
                    (classify_obligation_truth(combined), Some(combined))
                }
                ObligationDetail::Temporal {
                    requirement,
                    precondition,
                } => {
                    let precondition = self.evaluate(precondition);
                    let temporal = self.assess_temporal(requirement, microtraces);
                    supporting_microtrace_ids = temporal.1;
                    let combined = and_evaluated_truth(precondition, temporal.0);
                    (classify_obligation_truth(combined), Some(combined))
                }
                ObligationDetail::Geometry {
                    approach_id,
                    source_region_id,
                    destination_region_id,
                } => (
                    match self.spatial_connection(
                        approach_id,
                        source_region_id,
                        destination_region_id,
                    ) {
                        Some(SpatialConnectionStatus::Traversable) => {
                            ObligationClassification::Satisfied
                        }
                        Some(SpatialConnectionStatus::Blocked) => {
                            ObligationClassification::Unsatisfied
                        }
                        None => ObligationClassification::EvaluationUnknown,
                    },
                    None,
                ),
                ObligationDetail::PlaneSide { plane_id, relation } => {
                    let result = self.player_on_plane_side(plane_id, *relation);
                    (
                        match result {
                            EvaluatedTruth::True => ObligationClassification::Satisfied,
                            EvaluatedTruth::False => ObligationClassification::Unsatisfied,
                            EvaluatedTruth::Unknown => ObligationClassification::EvaluationUnknown,
                        },
                        Some(result),
                    )
                }
                ObligationDetail::Facing {
                    yaw,
                    target_yaw,
                    maximum_delta,
                } => {
                    let result = match self.resolve_value(yaw) {
                        Some(StateValue::Signed(value)) => {
                            i16::try_from(value).ok().map(|observed| {
                                observed.wrapping_sub(*target_yaw).unsigned_abs() <= *maximum_delta
                            })
                        }
                        _ => None,
                    };
                    (
                        match result {
                            Some(true) => ObligationClassification::Satisfied,
                            Some(false) => ObligationClassification::Unsatisfied,
                            None => ObligationClassification::EvaluationUnknown,
                        },
                        result.map(|value| {
                            if value {
                                EvaluatedTruth::True
                            } else {
                                EvaluatedTruth::False
                            }
                        }),
                    )
                }
                ObligationDetail::Unresolved { .. } => (ObligationClassification::Unmodeled, None),
            }
        };
        if classification != ObligationClassification::Satisfied {
            supporting_microtrace_ids.clear();
        }
        ObligationAssessment {
            obligation_id: obligation.id.clone(),
            classification,
            predicate,
            supporting_microtrace_ids,
        }
    }

    fn assess_temporal(
        &self,
        requirement: &TemporalRequirement,
        microtraces: &[WitnessedMicrotrace],
    ) -> (EvaluatedTruth, Vec<String>) {
        let mut matched = false;
        let mut uncertain = false;
        let mut supporting = Vec::new();
        for trace in microtraces
            .iter()
            .filter(|trace| self.scope_applies(&trace.scope) && trace.witnesses(requirement))
        {
            matched = true;
            if !self.policy.permits(trace.evidence.truth) {
                uncertain = true;
                continue;
            }
            match self.evaluate(&trace.precondition) {
                EvaluatedTruth::True => supporting.push(trace.id.clone()),
                EvaluatedTruth::Unknown => uncertain = true,
                EvaluatedTruth::False => {}
            }
        }
        if !supporting.is_empty() {
            (EvaluatedTruth::True, supporting)
        } else if uncertain || !matched {
            (EvaluatedTruth::Unknown, Vec::new())
        } else {
            (EvaluatedTruth::False, Vec::new())
        }
    }

    fn player_inside_volume(&self, reference: &VolumeReference) -> EvaluatedTruth {
        if !self.world_execution_active() {
            return EvaluatedTruth::Unknown;
        }
        self.position_inside_volume(self.snapshot.environment.player.position, reference)
    }

    fn interaction_position_inside_volume(
        &self,
        position: InteractionPosition,
        reference: &VolumeReference,
    ) -> EvaluatedTruth {
        if !self.world_execution_active() {
            return EvaluatedTruth::Unknown;
        }
        let position = match position {
            InteractionPosition::Player => Some(self.snapshot.environment.player.position),
            InteractionPosition::PlayerAttention => {
                self.snapshot.environment.player.attention_position
            }
        };
        position.map_or(EvaluatedTruth::Unknown, |position| {
            self.position_inside_volume(position, reference)
        })
    }

    fn position_inside_volume(
        &self,
        position: [f32; 3],
        reference: &VolumeReference,
    ) -> EvaluatedTruth {
        let Some(volume) = self
            .snapshot
            .environment
            .spatial_volumes
            .iter()
            .find(|volume| {
                volume.object_id == reference.object_id && volume.volume_id == reference.volume_id
            })
        else {
            return EvaluatedTruth::Unknown;
        };
        match &volume.shape {
            SpatialVolumeShape::AxisAlignedBox { minimum, maximum } => {
                if position
                    .iter()
                    .zip(minimum.iter().zip(maximum))
                    .all(|(value, (minimum, maximum))| value >= minimum && value <= maximum)
                {
                    EvaluatedTruth::True
                } else {
                    EvaluatedTruth::False
                }
            }
            SpatialVolumeShape::Sphere { center, radius } => {
                let squared_distance = position
                    .iter()
                    .zip(center)
                    .map(|(value, center)| {
                        let delta = f64::from(*value) - f64::from(*center);
                        delta * delta
                    })
                    .sum::<f64>();
                if squared_distance <= f64::from(*radius).powi(2) {
                    EvaluatedTruth::True
                } else {
                    EvaluatedTruth::False
                }
            }
            SpatialVolumeShape::VerticalCylinder {
                center_xz,
                minimum_y,
                maximum_y,
                radius,
            } => {
                let delta_x = f64::from(position[0]) - f64::from(center_xz[0]);
                let delta_z = f64::from(position[2]) - f64::from(center_xz[1]);
                if position[1] >= *minimum_y
                    && position[1] <= *maximum_y
                    && delta_x * delta_x + delta_z * delta_z <= f64::from(*radius).powi(2)
                {
                    EvaluatedTruth::True
                } else {
                    EvaluatedTruth::False
                }
            }
            SpatialVolumeShape::YawOrientedRectangle {
                origin_xz,
                yaw,
                minimum_local_xz,
                maximum_local_xz,
            } => {
                let delta_x = f64::from(position[0]) - f64::from(origin_xz[0]);
                let delta_z = f64::from(position[2]) - f64::from(origin_xz[1]);
                let radians = f64::from(*yaw) * std::f64::consts::TAU / 65536.0;
                let (sin, cos) = radians.sin_cos();
                // This is the inverse of the game's actor-local +Y yaw:
                // world +Z is (sin(yaw), cos(yaw)) in the X/Z plane.
                let local_x = cos * delta_x - sin * delta_z;
                let local_z = sin * delta_x + cos * delta_z;
                if local_x >= f64::from(minimum_local_xz[0])
                    && local_x <= f64::from(maximum_local_xz[0])
                    && local_z >= f64::from(minimum_local_xz[1])
                    && local_z <= f64::from(maximum_local_xz[1])
                {
                    EvaluatedTruth::True
                } else {
                    EvaluatedTruth::False
                }
            }
            SpatialVolumeShape::YawOrientedStrip {
                origin_xz,
                yaw,
                axis,
                minimum,
                maximum,
            } => {
                let delta_x = f64::from(position[0]) - f64::from(origin_xz[0]);
                let delta_z = f64::from(position[2]) - f64::from(origin_xz[1]);
                let radians = f64::from(*yaw) * std::f64::consts::TAU / 65536.0;
                let (sin, cos) = radians.sin_cos();
                let local = match axis {
                    crate::state::SpatialLocalAxis::X => cos * delta_x - sin * delta_z,
                    crate::state::SpatialLocalAxis::Z => sin * delta_x + cos * delta_z,
                };
                if local >= f64::from(*minimum) && local <= f64::from(*maximum) {
                    EvaluatedTruth::True
                } else {
                    EvaluatedTruth::False
                }
            }
        }
    }

    fn spatial_connection(
        &self,
        approach_id: &str,
        source_region_id: &str,
        destination_region_id: &str,
    ) -> Option<SpatialConnectionStatus> {
        if !self.world_execution_active() {
            return None;
        }
        self.snapshot
            .environment
            .spatial_connections
            .iter()
            .find(|connection| {
                connection.approach_id == approach_id
                    && connection.source_region_id == source_region_id
                    && connection.destination_region_id == destination_region_id
            })
            .map(|connection| connection.status)
    }

    fn player_on_plane_side(&self, plane_id: &str, relation: PlaneRelation) -> EvaluatedTruth {
        if !self.world_execution_active() {
            return EvaluatedTruth::Unknown;
        }
        let Some(plane) = self
            .snapshot
            .environment
            .spatial_planes
            .iter()
            .find(|plane| plane.plane_id == plane_id)
        else {
            return EvaluatedTruth::Unknown;
        };
        let signed_distance = plane
            .normal
            .iter()
            .zip(self.snapshot.environment.player.position)
            .map(|(normal, coordinate)| f64::from(*normal) * f64::from(coordinate))
            .sum::<f64>()
            + f64::from(plane.offset);
        let satisfied = match relation {
            PlaneRelation::Positive => signed_distance > 0.0,
            PlaneRelation::NonNegative => signed_distance >= 0.0,
            PlaneRelation::Negative => signed_distance < 0.0,
            PlaneRelation::NonPositive => signed_distance <= 0.0,
        };
        if satisfied {
            EvaluatedTruth::True
        } else {
            EvaluatedTruth::False
        }
    }

    fn interaction_actor_loaded(&self, instance_id: &str) -> EvaluatedTruth {
        if !self.world_execution_active() {
            return EvaluatedTruth::Unknown;
        }
        match self
            .snapshot
            .environment
            .live_world_objects
            .iter()
            .find(|object| object.instance_id == instance_id)
            .map(|object| object.lifecycle)
        {
            Some(ActorLifecycle::Loaded) => EvaluatedTruth::True,
            Some(
                ActorLifecycle::Unloading | ActorLifecycle::Unloaded | ActorLifecycle::Destroyed,
            ) => EvaluatedTruth::False,
            None => EvaluatedTruth::Unknown,
        }
    }

    pub fn assess_gate(&self, gate: &GateRule) -> GateAssessment {
        let scope_applies = self.scope_applies(&gate.scope);
        let evidence_permitted = self.policy.permits(gate.evidence.truth);
        let active = if scope_applies && evidence_permitted {
            self.evaluate(&gate.active_when)
        } else {
            EvaluatedTruth::Unknown
        };
        GateAssessment {
            gate_id: gate.id.clone(),
            scope_applies,
            evidence_permitted,
            active,
        }
    }

    pub fn assess_writer(&self, writer: &WriterRule, gates: &[GateRule]) -> WriterAssessment {
        let scope_applies = self.scope_applies(&writer.scope);
        let evidence_permitted = self.policy.permits(writer.evidence.truth);
        let activation = if scope_applies && evidence_permitted {
            self.evaluate(&writer.activation)
        } else {
            EvaluatedTruth::Unknown
        };
        let mut active_gate_ids = Vec::new();
        let mut unknown_gate_ids = Vec::new();
        for gate in gates.iter().filter(|gate| {
            gate.blocked_writer_ids
                .iter()
                .any(|writer_id| writer_id == &writer.id)
        }) {
            let assessment = self.assess_gate(gate);
            match assessment.active {
                EvaluatedTruth::True => active_gate_ids.push(gate.id.clone()),
                EvaluatedTruth::Unknown => unknown_gate_ids.push(gate.id.clone()),
                EvaluatedTruth::False => {}
            }
        }
        let classification = if !scope_applies {
            WriterClassification::Inapplicable
        } else if activation == EvaluatedTruth::False {
            WriterClassification::Inactive
        } else if !evidence_permitted || activation == EvaluatedTruth::Unknown {
            WriterClassification::ActivationUnknown
        } else if !active_gate_ids.is_empty() {
            WriterClassification::GateBlocked
        } else if !unknown_gate_ids.is_empty() {
            WriterClassification::GateUnknown
        } else {
            WriterClassification::Executable
        };
        WriterAssessment {
            writer_id: writer.id.clone(),
            classification,
            scope_applies,
            evidence_permitted,
            activation,
            active_gate_ids,
            unknown_gate_ids,
        }
    }

    pub fn assess_reader(&self, reader: &ReaderRule) -> ReaderAssessment {
        let scope_applies = self.scope_applies(&reader.scope);
        let evidence_permitted = self.policy.permits(reader.evidence.truth);
        let source_value = if scope_applies && evidence_permitted {
            self.resolve_value(&reader.source)
        } else {
            None
        };
        let interpretation = if scope_applies && evidence_permitted {
            reader.interpretation_fact_id.as_ref().map(|fact_id| {
                self.evaluate(&PredicateExpression::Fact {
                    fact_id: fact_id.clone(),
                })
            })
        } else {
            None
        };
        ReaderAssessment {
            reader_id: reader.id.clone(),
            scope_applies,
            evidence_permitted,
            source_value,
            interpretation,
        }
    }

    pub fn assess_obstruction(&self, obstruction: &Obstruction) -> ObstructionAssessment {
        let (classification, activation) = self.assess_rule(
            &obstruction.scope,
            obstruction.evidence.truth,
            &obstruction.active_when,
        );
        ObstructionAssessment {
            obstruction_id: obstruction.id.clone(),
            classification,
            activation,
            obligation_ids: obstruction.obligation_ids.clone(),
        }
    }

    pub fn assess_resolver(&self, resolver: &ObstructionResolver) -> ResolverAssessment {
        let (classification, applicability) = self.assess_rule(
            &resolver.scope,
            resolver.evidence.truth,
            &resolver.applicable_when,
        );
        ResolverAssessment {
            resolver_id: resolver.id.clone(),
            obstruction_id: resolver.obstruction_id.clone(),
            classification,
            applicability,
        }
    }

    pub fn assess_technique(&self, technique: &Technique) -> TechniqueAssessment {
        let (classification, prerequisites) = self.assess_rule(
            &technique.scope,
            technique.evidence.truth,
            &technique.prerequisites,
        );
        TechniqueAssessment {
            technique_id: technique.id.clone(),
            classification,
            prerequisites,
            discharged_obligation_ids: technique.discharged_obligation_ids.clone(),
            introduced_obligation_ids: technique.introduced_obligation_ids.clone(),
        }
    }

    pub fn assess_reconstruction(
        &self,
        rule: &ActorReconstructionRule,
    ) -> ReconstructionAssessment {
        let (classification, activation) =
            self.assess_rule(&rule.scope, rule.evidence.truth, &rule.instantiate_when);
        ReconstructionAssessment {
            reconstruction_rule_id: rule.id.clone(),
            classification,
            activation,
        }
    }

    /// Resolves only records relevant to one transition and approach. A
    /// resolver discharges the obligations named by its obstruction; a
    /// technique discharges only its explicit list. Neither deletes the
    /// obstruction or changes its underlying activation fact.
    pub fn resolve_feasibility(
        &self,
        transition: &CandidateTransition,
        obligations: &[FeasibilityObligation],
        obstructions: &[Obstruction],
        resolvers: &[ObstructionResolver],
        techniques: &[Technique],
        selection: FeasibilitySelection<'_>,
    ) -> FeasibilityResolution {
        let mut resolution = FeasibilityResolution {
            claimed_obligation_ids: selection.already_discharged.clone(),
            discharged_obligation_ids: BTreeSet::new(),
            unknown_obligation_ids: BTreeSet::new(),
            supporting_microtrace_ids: BTreeSet::new(),
            active_obstruction_ids: Vec::new(),
            unknown_obstruction_ids: Vec::new(),
            applied_resolver_ids: Vec::new(),
            applicable_technique_ids: Vec::new(),
        };

        for technique in techniques
            .iter()
            .filter(|technique| selection.technique_ids.contains(&technique.id))
        {
            let assessment = self.assess_technique(technique);
            if assessment.classification == RuleClassification::Active {
                resolution
                    .claimed_obligation_ids
                    .extend(assessment.discharged_obligation_ids);
                for introduced in assessment.introduced_obligation_ids {
                    resolution.claimed_obligation_ids.remove(&introduced);
                }
                resolution
                    .applicable_technique_ids
                    .push(technique.id.clone());
            }
        }

        for obstruction in obstructions.iter().filter(|obstruction| {
            obstruction.blocked_action_id == transition.id
                && obstruction.approach_id == transition.approach_id
        }) {
            let assessment = self.assess_obstruction(obstruction);
            match assessment.classification {
                RuleClassification::Active => {
                    resolution
                        .active_obstruction_ids
                        .push(obstruction.id.clone());
                    let applicable = resolvers
                        .iter()
                        .filter(|resolver| resolver.obstruction_id == obstruction.id)
                        .filter(|resolver| selection.resolver_ids.contains(&resolver.id))
                        .filter(|resolver| {
                            self.assess_resolver(resolver).classification
                                == RuleClassification::Active
                        })
                        .collect::<Vec<_>>();
                    if !applicable.is_empty() {
                        resolution
                            .claimed_obligation_ids
                            .extend(obstruction.obligation_ids.iter().cloned());
                        resolution
                            .applied_resolver_ids
                            .extend(applicable.into_iter().map(|resolver| resolver.id.clone()));
                    }
                }
                RuleClassification::EvidenceUnknown | RuleClassification::ActivationUnknown => {
                    resolution
                        .unknown_obstruction_ids
                        .push(obstruction.id.clone())
                }
                RuleClassification::Inapplicable | RuleClassification::Inactive => {}
            }
        }
        self.refresh_obligation_assessments(
            transition,
            obligations,
            selection.microtraces,
            &mut resolution,
        );
        resolution
    }

    pub fn refresh_obligation_assessments(
        &self,
        transition: &CandidateTransition,
        obligations: &[FeasibilityObligation],
        microtraces: &[WitnessedMicrotrace],
        resolution: &mut FeasibilityResolution,
    ) {
        resolution.discharged_obligation_ids = resolution.claimed_obligation_ids.clone();
        resolution.unknown_obligation_ids.clear();
        resolution.supporting_microtrace_ids.clear();
        for obligation_id in &transition.activation.physical_obligation_ids {
            if resolution.claimed_obligation_ids.contains(obligation_id) {
                continue;
            }
            let Some(obligation) = obligations
                .iter()
                .find(|record| record.id == *obligation_id)
            else {
                resolution
                    .unknown_obligation_ids
                    .insert(obligation_id.clone());
                continue;
            };
            let assessment = self.assess_obligation(obligation, microtraces);
            resolution
                .supporting_microtrace_ids
                .extend(assessment.supporting_microtrace_ids);
            match assessment.classification {
                ObligationClassification::Satisfied => {
                    resolution
                        .discharged_obligation_ids
                        .insert(obligation.id.clone());
                }
                ObligationClassification::Inapplicable
                | ObligationClassification::EvidenceUnknown
                | ObligationClassification::EvaluationUnknown
                | ObligationClassification::Unmodeled => {
                    resolution
                        .unknown_obligation_ids
                        .insert(obligation.id.clone());
                }
                ObligationClassification::Unsatisfied => {}
            }
        }
    }

    fn assess_rule(
        &self,
        scope: &ContextScope,
        truth: TruthStatus,
        expression: &PredicateExpression,
    ) -> (RuleClassification, EvaluatedTruth) {
        if !self.scope_applies(scope) {
            return (RuleClassification::Inapplicable, EvaluatedTruth::Unknown);
        }
        if !self.policy.permits(truth) {
            return (RuleClassification::EvidenceUnknown, EvaluatedTruth::Unknown);
        }
        let activation = self.evaluate(expression);
        let classification = match activation {
            EvaluatedTruth::True => RuleClassification::Active,
            EvaluatedTruth::False => RuleClassification::Inactive,
            EvaluatedTruth::Unknown => RuleClassification::ActivationUnknown,
        };
        (classification, activation)
    }

    fn evaluate_inner(
        &self,
        expression: &PredicateExpression,
        fact_stack: &mut BTreeSet<String>,
        memo: &mut BTreeMap<String, EvaluatedTruth>,
    ) -> EvaluatedTruth {
        match expression {
            PredicateExpression::True => EvaluatedTruth::True,
            PredicateExpression::False => EvaluatedTruth::False,
            PredicateExpression::Compare {
                left,
                operator,
                right,
            } => match (self.resolve_value(left), self.resolve_value(right)) {
                (Some(left), Some(right)) => compare_values(&left, *operator, &right),
                _ => EvaluatedTruth::Unknown,
            },
            PredicateExpression::Fact { fact_id } => self.evaluate_fact(fact_id, fact_stack, memo),
            PredicateExpression::All { terms } => {
                let mut unknown = false;
                for term in terms {
                    match self.evaluate_inner(term, fact_stack, memo) {
                        EvaluatedTruth::False => return EvaluatedTruth::False,
                        EvaluatedTruth::Unknown => unknown = true,
                        EvaluatedTruth::True => {}
                    }
                }
                if unknown {
                    EvaluatedTruth::Unknown
                } else {
                    EvaluatedTruth::True
                }
            }
            PredicateExpression::Any { terms } => {
                let mut unknown = false;
                for term in terms {
                    match self.evaluate_inner(term, fact_stack, memo) {
                        EvaluatedTruth::True => return EvaluatedTruth::True,
                        EvaluatedTruth::Unknown => unknown = true,
                        EvaluatedTruth::False => {}
                    }
                }
                if unknown {
                    EvaluatedTruth::Unknown
                } else {
                    EvaluatedTruth::False
                }
            }
            PredicateExpression::Not { term } => self.evaluate_inner(term, fact_stack, memo).not(),
        }
    }

    fn evaluate_fact(
        &self,
        fact_id: &str,
        fact_stack: &mut BTreeSet<String>,
        memo: &mut BTreeMap<String, EvaluatedTruth>,
    ) -> EvaluatedTruth {
        if let Some(value) = memo.get(fact_id) {
            return *value;
        }
        if !fact_stack.insert(fact_id.into()) {
            return EvaluatedTruth::Unknown;
        }
        let value = if let Ok(index) = self
            .facts
            .aliases
            .binary_search_by_key(&fact_id, |alias| alias.id.as_str())
        {
            let alias = &self.facts.aliases[index];
            if !self.scope_applies(&alias.scope) || !self.policy.permits(alias.evidence.truth) {
                EvaluatedTruth::Unknown
            } else {
                self.evaluate_raw_binding(&alias.raw)
            }
        } else if let Ok(index) = self
            .facts
            .derived_facts
            .binary_search_by_key(&fact_id, |fact| fact.id.as_str())
        {
            let fact = &self.facts.derived_facts[index];
            if !self.scope_applies(&fact.scope) || !self.policy.permits(fact.evidence.truth) {
                EvaluatedTruth::Unknown
            } else {
                self.evaluate_inner(&fact.rule, fact_stack, memo)
            }
        } else {
            EvaluatedTruth::Unknown
        };
        fact_stack.remove(fact_id);
        memo.insert(fact_id.into(), value);
        value
    }

    fn evaluate_raw_binding(&self, binding: &RawFactBinding) -> EvaluatedTruth {
        let Some(resolved_binding) = binding.binding.resolve(&self.snapshot.environment) else {
            return EvaluatedTruth::Unknown;
        };
        let matches = self
            .snapshot
            .environment
            .components
            .iter()
            .filter(|component| {
                component.component_kind == binding.component_kind
                    && component.binding == resolved_binding
                    && matches!(component.payload, ComponentPayload::Raw { .. })
                    && self.component_readable(component)
            })
            .collect::<Vec<_>>();
        let [component] = matches.as_slice() else {
            return EvaluatedTruth::Unknown;
        };
        let ComponentPayload::Raw { bytes, known_mask } = &component.payload else {
            return EvaluatedTruth::Unknown;
        };
        let Ok(offset) = usize::try_from(binding.byte_offset) else {
            return EvaluatedTruth::Unknown;
        };
        let Some(end) = offset.checked_add(binding.mask.len()) else {
            return EvaluatedTruth::Unknown;
        };
        if end > bytes.len() || end > known_mask.len() {
            return EvaluatedTruth::Unknown;
        }
        for index in 0..binding.mask.len() {
            let mask = binding.mask[index];
            if known_mask[offset + index] & mask != mask {
                return EvaluatedTruth::Unknown;
            }
            if bytes[offset + index] & mask != binding.expected[index] & mask {
                return EvaluatedTruth::False;
            }
        }
        EvaluatedTruth::True
    }

    pub fn resolve_value(&self, reference: &ValueReference) -> Option<StateValue> {
        match reference {
            ValueReference::Literal { value } => Some(value.clone()),
            ValueReference::ComponentField {
                component_id,
                field,
            } => {
                let component = self
                    .snapshot
                    .environment
                    .components
                    .iter()
                    .find(|component| component.id == *component_id)?;
                if !self.component_readable(component) {
                    return None;
                }
                structured_field(component, field)
            }
            ValueReference::ComponentBytes {
                component_id,
                field,
                byte_offset,
                byte_width,
                mask,
            } => {
                let component = self
                    .snapshot
                    .environment
                    .components
                    .iter()
                    .find(|component| component.id == *component_id)?;
                if !self.component_readable(component) {
                    return None;
                }
                let StateValue::Bytes(bytes) = structured_field(component, field)? else {
                    return None;
                };
                byte_vector_bits(&bytes, *byte_offset, *byte_width, *mask).map(StateValue::Unsigned)
            }
            ValueReference::BoundComponentField {
                component_kind,
                binding,
                field,
            } => {
                let resolved_binding = binding.resolve(&self.snapshot.environment)?;
                let mut matches = self
                    .snapshot
                    .environment
                    .components
                    .iter()
                    .filter(|component| {
                        component.component_kind == *component_kind
                            && component.binding == resolved_binding
                            && matches!(component.payload, ComponentPayload::Structured { .. })
                            && self.component_readable(component)
                    });
                let component = matches.next()?;
                if matches.next().is_some() {
                    return None;
                }
                structured_field(component, field)
            }
            ValueReference::RawBits {
                component_id,
                byte_offset,
                byte_width,
                mask,
            } => {
                let component = self
                    .snapshot
                    .environment
                    .components
                    .iter()
                    .find(|component| component.id == *component_id)?;
                if !self.component_readable(component) {
                    return None;
                }
                raw_bits(component, *byte_offset, *byte_width, *mask).map(StateValue::Unsigned)
            }
            ValueReference::BoundRawBits {
                component_kind,
                binding,
                byte_offset,
                byte_width,
                mask,
            } => {
                let resolved_binding = binding.resolve(&self.snapshot.environment)?;
                let mut matches = self
                    .snapshot
                    .environment
                    .components
                    .iter()
                    .filter(|component| {
                        component.component_kind == *component_kind
                            && component.binding == resolved_binding
                            && matches!(component.payload, ComponentPayload::Raw { .. })
                            && self.component_readable(component)
                    });
                let component = matches.next()?;
                if matches.next().is_some() {
                    return None;
                }
                raw_bits(component, *byte_offset, *byte_width, *mask).map(StateValue::Unsigned)
            }
            ValueReference::RuntimeLanguage => Some(StateValue::Text(
                self.snapshot
                    .environment
                    .runtime_configuration
                    .language
                    .clone(),
            )),
            ValueReference::ActiveRuntimeFileOrigin => {
                match &self.snapshot.environment.active_runtime_file.origin {
                    RuntimeFileOrigin::TitleFile0 => Some(StateValue::Text("title_file_0".into())),
                    RuntimeFileOrigin::LoadedSlot { .. } => {
                        Some(StateValue::Text("loaded_slot".into()))
                    }
                    RuntimeFileOrigin::NewFile => Some(StateValue::Text("new_file".into())),
                    RuntimeFileOrigin::Other { id } => {
                        Some(StateValue::Text(format!("other:{id}")))
                    }
                    RuntimeFileOrigin::Unknown => None,
                }
            }
            ValueReference::ExecutionProcess => {
                match &self.snapshot.environment.execution_context {
                    crate::state::ExecutionContext::Process { process_name, .. } => {
                        Some(StateValue::Text(process_name.clone()))
                    }
                    crate::state::ExecutionContext::World
                    | crate::state::ExecutionContext::Unknown => None,
                }
            }
            ValueReference::WorldExecutionActive => {
                match self.snapshot.environment.execution_context {
                    crate::state::ExecutionContext::World => Some(StateValue::Boolean(true)),
                    crate::state::ExecutionContext::Process { .. } => {
                        Some(StateValue::Boolean(false))
                    }
                    crate::state::ExecutionContext::Unknown => None,
                }
            }
            ValueReference::PendingWorldLoadStage => {
                pending_world_load(&self.snapshot.environment.execution_context)
                    .map(|location| StateValue::Text(location.stage.clone()))
            }
            ValueReference::PendingWorldLoadRoom => {
                pending_world_load(&self.snapshot.environment.execution_context)
                    .map(|location| StateValue::Signed(location.room.into()))
            }
            ValueReference::PendingWorldLoadLayer => {
                pending_world_load(&self.snapshot.environment.execution_context)
                    .map(|location| StateValue::Signed(location.layer.into()))
            }
            ValueReference::PendingWorldLoadSpawn => {
                pending_world_load(&self.snapshot.environment.execution_context)
                    .map(|location| StateValue::Signed(location.spawn.into()))
            }
            ValueReference::PhysicalSlotImageAvailable { slot } => {
                if self
                    .snapshot
                    .environment
                    .physical_slots
                    .iter()
                    .any(|physical_slot| physical_slot.slot == *slot)
                {
                    Some(StateValue::Boolean(true))
                } else {
                    self.snapshot
                        .environment
                        .physical_slot_observations
                        .iter()
                        .find(|observation| observation.slot == *slot)
                        .and_then(|observation| match observation.content_status {
                            crate::state::CaptureStatus::Absent => Some(StateValue::Boolean(false)),
                            crate::state::CaptureStatus::NotSampled
                            | crate::state::CaptureStatus::Present
                            | crate::state::CaptureStatus::Unavailable => None,
                        })
                }
            }
            ValueReference::RuntimeSetting { key } => self
                .snapshot
                .environment
                .runtime_configuration
                .settings
                .get(key)
                .map(configuration_value),
            ValueReference::LocationStage => self
                .world_location()
                .map(|location| StateValue::Text(location.stage.clone())),
            ValueReference::LocationRoom => self
                .world_location()
                .map(|location| StateValue::Signed(location.room.into())),
            ValueReference::LocationLayer => self
                .world_location()
                .map(|location| StateValue::Signed(location.layer.into())),
            ValueReference::LocationSpawn => self
                .world_location()
                .map(|location| StateValue::Signed(location.spawn.into())),
            ValueReference::PlayerForm => self
                .world_player()
                .and_then(|player| player_form_value(&player.form)),
            ValueReference::PlayerMount => self
                .world_player()?
                .mount
                .as_ref()
                .and_then(player_mount_value),
            ValueReference::PlayerControl => {
                self.world_player()?.has_control.map(StateValue::Boolean)
            }
            ValueReference::PlayerRotationX => self
                .world_player()
                .map(|player| StateValue::Signed(player.rotation[0].into())),
            ValueReference::PlayerRotationY => self
                .world_player()
                .map(|player| StateValue::Signed(player.rotation[1].into())),
            ValueReference::PlayerRotationZ => self
                .world_player()
                .map(|player| StateValue::Signed(player.rotation[2].into())),
            ValueReference::PlayerAction => self
                .world_player()
                .map(|player| StateValue::Text(player.action.clone())),
            ValueReference::ActorField { instance_id, field } => {
                self.world_location()?;
                self.snapshot
                    .environment
                    .live_world_objects
                    .iter()
                    .find(|actor| actor.instance_id == *instance_id)?
                    .fields
                    .get(field)
                    .cloned()
            }
            ValueReference::GateState { gate_id } => self
                .gate_states
                .get(gate_id)
                .copied()
                .map(StateValue::Boolean),
            ValueReference::FlowNode { flow_component_id } => structured_field(
                self.snapshot
                    .environment
                    .components
                    .iter()
                    .find(|component| component.id == *flow_component_id)?,
                "node_id",
            ),
        }
    }
}

fn structured_field(component: &StateComponent, field: &str) -> Option<StateValue> {
    let ComponentPayload::Structured { fields } = &component.payload else {
        return None;
    };
    fields.get(field).cloned()
}

fn pending_world_load(context: &ExecutionContext) -> Option<&crate::state::SceneLocation> {
    let ExecutionContext::Process {
        pending_world_load: Some(location),
        ..
    } = context
    else {
        return None;
    };
    Some(location)
}

fn raw_bits(
    component: &StateComponent,
    byte_offset: u32,
    byte_width: u8,
    mask: u64,
) -> Option<u64> {
    let ComponentPayload::Raw { bytes, known_mask } = &component.payload else {
        return None;
    };
    let offset = usize::try_from(byte_offset).ok()?;
    let width = usize::from(byte_width);
    let end = offset.checked_add(width)?;
    if width == 0 || width > 8 || end > bytes.len() || end > known_mask.len() {
        return None;
    }
    let mut value = 0_u64;
    let mut known = 0_u64;
    for index in 0..width {
        value |= u64::from(bytes[offset + index]) << (index * 8);
        known |= u64::from(known_mask[offset + index]) << (index * 8);
    }
    (known & mask == mask).then_some(value & mask)
}

fn byte_vector_bits(bytes: &[u8], byte_offset: u32, byte_width: u8, mask: u64) -> Option<u64> {
    let offset = usize::try_from(byte_offset).ok()?;
    let width = usize::from(byte_width);
    let end = offset.checked_add(width)?;
    if width == 0 || width > 8 || end > bytes.len() {
        return None;
    }
    let mut value = 0_u64;
    for index in 0..width {
        value |= u64::from(bytes[offset + index]) << (index * 8);
    }
    Some(value & mask)
}

fn configuration_value(value: &ConfigurationValue) -> StateValue {
    match value {
        ConfigurationValue::Boolean(value) => StateValue::Boolean(*value),
        ConfigurationValue::Integer(value) => StateValue::Signed(*value),
        ConfigurationValue::Text(value) => StateValue::Text(value.clone()),
    }
}

fn player_form_value(form: &PlayerForm) -> Option<StateValue> {
    match form {
        PlayerForm::Human => Some(StateValue::Text("human".into())),
        PlayerForm::Wolf => Some(StateValue::Text("wolf".into())),
        PlayerForm::Other { id } => Some(StateValue::Text(id.clone())),
        PlayerForm::Unknown => None,
    }
}

fn player_mount_value(mount: &PlayerMount) -> Option<StateValue> {
    match mount {
        PlayerMount::Epona => Some(StateValue::Text("epona".into())),
        PlayerMount::Boar => Some(StateValue::Text("boar".into())),
        PlayerMount::Other { id } => Some(StateValue::Text(id.clone())),
        PlayerMount::Unknown => None,
    }
}

fn and_evaluated_truth(left: EvaluatedTruth, right: EvaluatedTruth) -> EvaluatedTruth {
    match (left, right) {
        (EvaluatedTruth::False, _) | (_, EvaluatedTruth::False) => EvaluatedTruth::False,
        (EvaluatedTruth::Unknown, _) | (_, EvaluatedTruth::Unknown) => EvaluatedTruth::Unknown,
        (EvaluatedTruth::True, EvaluatedTruth::True) => EvaluatedTruth::True,
    }
}

fn or_evaluated_truth(left: EvaluatedTruth, right: EvaluatedTruth) -> EvaluatedTruth {
    match (left, right) {
        (EvaluatedTruth::True, _) | (_, EvaluatedTruth::True) => EvaluatedTruth::True,
        (EvaluatedTruth::Unknown, _) | (_, EvaluatedTruth::Unknown) => EvaluatedTruth::Unknown,
        _ => EvaluatedTruth::False,
    }
}

fn classify_obligation_truth(truth: EvaluatedTruth) -> ObligationClassification {
    match truth {
        EvaluatedTruth::True => ObligationClassification::Satisfied,
        EvaluatedTruth::False => ObligationClassification::Unsatisfied,
        EvaluatedTruth::Unknown => ObligationClassification::EvaluationUnknown,
    }
}

fn compare_values(
    left: &StateValue,
    operator: ComparisonOperator,
    right: &StateValue,
) -> EvaluatedTruth {
    let result = match operator {
        ComparisonOperator::Equal | ComparisonOperator::NotEqual => {
            let equal = values_equal(left, right);
            return match (operator, equal) {
                (_, None) => EvaluatedTruth::Unknown,
                (ComparisonOperator::Equal, Some(true))
                | (ComparisonOperator::NotEqual, Some(false)) => EvaluatedTruth::True,
                _ => EvaluatedTruth::False,
            };
        }
        ComparisonOperator::LessThan
        | ComparisonOperator::LessThanOrEqual
        | ComparisonOperator::GreaterThan
        | ComparisonOperator::GreaterThanOrEqual => {
            compare_order(left, right).map(|ordering| match operator {
                ComparisonOperator::LessThan => ordering == Ordering::Less,
                ComparisonOperator::LessThanOrEqual => ordering != Ordering::Greater,
                ComparisonOperator::GreaterThan => ordering == Ordering::Greater,
                ComparisonOperator::GreaterThanOrEqual => ordering != Ordering::Less,
                _ => unreachable!(),
            })
        }
        ComparisonOperator::ContainsBits => match (left, right) {
            (StateValue::Unsigned(left), StateValue::Unsigned(right)) => {
                Some(left & right == *right)
            }
            (StateValue::Bytes(left), StateValue::Bytes(right)) if left.len() == right.len() => {
                Some(
                    left.iter()
                        .zip(right)
                        .all(|(left, right)| left & right == *right),
                )
            }
            _ => None,
        },
    };
    match result {
        Some(true) => EvaluatedTruth::True,
        Some(false) => EvaluatedTruth::False,
        None => EvaluatedTruth::Unknown,
    }
}

fn values_equal(left: &StateValue, right: &StateValue) -> Option<bool> {
    match (left, right) {
        (StateValue::Signed(left), StateValue::Unsigned(right)) => {
            Some(*left >= 0 && *left as u64 == *right)
        }
        (StateValue::Unsigned(left), StateValue::Signed(right)) => {
            Some(*right >= 0 && *left == *right as u64)
        }
        (StateValue::Boolean(left), StateValue::Boolean(right)) => Some(left == right),
        (StateValue::Signed(left), StateValue::Signed(right)) => Some(left == right),
        (StateValue::Unsigned(left), StateValue::Unsigned(right)) => Some(left == right),
        (StateValue::Text(left), StateValue::Text(right)) => Some(left == right),
        (StateValue::Bytes(left), StateValue::Bytes(right)) => Some(left == right),
        _ => None,
    }
}

fn compare_order(left: &StateValue, right: &StateValue) -> Option<Ordering> {
    match (left, right) {
        (StateValue::Signed(left), StateValue::Signed(right)) => Some(left.cmp(right)),
        (StateValue::Unsigned(left), StateValue::Unsigned(right)) => Some(left.cmp(right)),
        (StateValue::Signed(left), StateValue::Unsigned(right)) => {
            if *left < 0 {
                Some(Ordering::Less)
            } else {
                Some((*left as u64).cmp(right))
            }
        }
        (StateValue::Unsigned(left), StateValue::Signed(right)) => {
            if *right < 0 {
                Some(Ordering::Greater)
            } else {
                Some(left.cmp(&(*right as u64)))
            }
        }
        (StateValue::Text(left), StateValue::Text(right)) => Some(left.cmp(right)),
        _ => None,
    }
}

#[cfg(test)]
#[path = "evaluation_tests.rs"]
mod tests;
