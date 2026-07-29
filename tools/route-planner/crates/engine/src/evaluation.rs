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

mod assessments;

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
