//! Loss-aware predicate evaluation and transition readiness classification.

use crate::identity::{ConfigurationValue, ContextSelector, EquivalenceSet, ExactContext};
use crate::logic::{
    ComparisonOperator, ContextScope, FactCatalog, PredicateExpression, RawFactBinding,
    TruthStatus, ValueReference,
};
use crate::state::{ComponentPayload, PlayerForm, PlayerMount, StateComponent, StateValue};
use crate::transition::{CandidateTransition, GateRule, ReaderRule, WriterRule};
use crate::transition::{Obstruction, ObstructionResolver, Technique};
use crate::{PlannerContractError, validate_stable_id};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransitionClassification {
    Inapplicable,
    GuardBlocked,
    FeasibilityUnknown,
    Obstructed,
    Executable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransitionAssessment {
    pub transition_id: String,
    pub classification: TransitionClassification,
    pub scope_applies: bool,
    pub evidence_permitted: bool,
    pub hard_guard: EvaluatedTruth,
    pub outstanding_obligation_ids: Vec<String>,
    pub unknown_requirement_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GateAssessment {
    pub gate_id: String,
    pub scope_applies: bool,
    pub evidence_permitted: bool,
    pub active: EvaluatedTruth,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
pub struct FeasibilityResolution {
    pub discharged_obligation_ids: BTreeSet<String>,
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
        let classification = if !scope_applies {
            TransitionClassification::Inapplicable
        } else if hard_guard == EvaluatedTruth::False {
            TransitionClassification::GuardBlocked
        } else if !evidence_permitted
            || hard_guard == EvaluatedTruth::Unknown
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
            unknown_requirement_ids,
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

    /// Resolves only records relevant to one transition and approach. A
    /// resolver discharges the obligations named by its obstruction; a
    /// technique discharges only its explicit list. Neither deletes the
    /// obstruction or changes its underlying activation fact.
    pub fn resolve_feasibility(
        &self,
        transition: &CandidateTransition,
        obstructions: &[Obstruction],
        resolvers: &[ObstructionResolver],
        techniques: &[Technique],
        selection: FeasibilitySelection<'_>,
    ) -> FeasibilityResolution {
        let mut resolution = FeasibilityResolution {
            discharged_obligation_ids: selection.already_discharged.clone(),
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
                    .discharged_obligation_ids
                    .extend(assessment.discharged_obligation_ids);
                for introduced in assessment.introduced_obligation_ids {
                    resolution.discharged_obligation_ids.remove(&introduced);
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
                            .discharged_obligation_ids
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
        resolution
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
        let matches = self
            .snapshot
            .environment
            .components
            .iter()
            .filter(|component| {
                component.component_kind == binding.component_kind
                    && component.binding == binding.binding
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
            } => structured_field(
                self.snapshot
                    .environment
                    .components
                    .iter()
                    .find(|component| component.id == *component_id)?,
                field,
            ),
            ValueReference::RawBits {
                component_id,
                byte_offset,
                byte_width,
                mask,
            } => raw_bits(
                self.snapshot
                    .environment
                    .components
                    .iter()
                    .find(|component| component.id == *component_id)?,
                *byte_offset,
                *byte_width,
                *mask,
            )
            .map(StateValue::Unsigned),
            ValueReference::RuntimeLanguage => Some(StateValue::Text(
                self.snapshot
                    .environment
                    .runtime_configuration
                    .language
                    .clone(),
            )),
            ValueReference::RuntimeSetting { key } => self
                .snapshot
                .environment
                .runtime_configuration
                .settings
                .get(key)
                .map(configuration_value),
            ValueReference::LocationStage => Some(StateValue::Text(
                self.snapshot.environment.location.stage.clone(),
            )),
            ValueReference::LocationRoom => Some(StateValue::Signed(
                self.snapshot.environment.location.room.into(),
            )),
            ValueReference::LocationLayer => Some(StateValue::Signed(
                self.snapshot.environment.location.layer.into(),
            )),
            ValueReference::LocationSpawn => Some(StateValue::Signed(
                self.snapshot.environment.location.spawn.into(),
            )),
            ValueReference::PlayerForm => player_form_value(&self.snapshot.environment.player.form),
            ValueReference::PlayerMount => self
                .snapshot
                .environment
                .player
                .mount
                .as_ref()
                .and_then(player_mount_value),
            ValueReference::PlayerControl => self
                .snapshot
                .environment
                .player
                .has_control
                .map(StateValue::Boolean),
            ValueReference::ActorField { instance_id, field } => self
                .snapshot
                .environment
                .live_world_objects
                .iter()
                .find(|actor| actor.instance_id == *instance_id)?
                .fields
                .get(field)
                .cloned(),
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
mod tests {
    use super::*;
    use crate::artifact::Digest;
    use crate::identity::{
        EQUIVALENCE_SET_SCHEMA, EquivalenceEvidence, EquivalenceEvidenceKind,
        RUNTIME_CONFIGURATION_SCHEMA, RuntimeConfiguration,
    };
    use crate::logic::{
        DerivedFact, EvidenceKind, EvidenceRecord, FACT_CATALOG_SCHEMA, FriendlyAlias, RuleEvidence,
    };
    use crate::snapshot::{STATE_SNAPSHOT_SCHEMA, StateSnapshot};
    use crate::state::{
        BackingAttachment, ComponentBinding, ComponentKind, ComponentPayload, ComponentProvenance,
        EXECUTION_ENVIRONMENT_SCHEMA, ExecutionEnvironment, PlayerForm, PlayerState,
        ProvenanceSourceKind, RuntimeFile, RuntimeFileLifecycle, RuntimeFileOrigin, SceneLocation,
        SemanticLifetime, SerializationOwner, StateComponent,
    };
    use crate::transition::{ActivationContract, TransitionKind, UnknownRequirement};

    fn evidence(truth: TruthStatus) -> RuleEvidence {
        RuleEvidence {
            truth,
            records: if matches!(truth, TruthStatus::Established | TruthStatus::Contested) {
                vec![EvidenceRecord {
                    id: "source.evaluator-test".into(),
                    kind: EvidenceKind::SourceAudited,
                    source_sha256: Some(Digest([9; 32])),
                    note: "Evaluator test evidence.".into(),
                }]
            } else {
                Vec::new()
            },
        }
    }

    fn component(known_mask: u8) -> StateComponent {
        StateComponent {
            id: "save-flags".into(),
            component_kind: ComponentKind::PersistentSave,
            payload: ComponentPayload::Raw {
                bytes: vec![0x20],
                known_mask: vec![known_mask],
            },
            binding: ComponentBinding::Global,
            lifetime: SemanticLifetime::RuntimeFile,
            serialization_owner: SerializationOwner::RuntimeFile {
                runtime_file_id: "file-0".into(),
            },
            provenance: vec![ComponentProvenance {
                source_kind: ProvenanceSourceKind::TraceObservation,
                source_id: "trace.test".into(),
                source_sha256: Some(Digest([8; 32])),
                transition_id: None,
            }],
        }
    }

    fn snapshot(known_mask: u8) -> StateSnapshot {
        StateSnapshot {
            schema: STATE_SNAPSHOT_SCHEMA.into(),
            id: "snapshot.evaluator".into(),
            sequence: 1,
            environment: ExecutionEnvironment {
                schema: EXECUTION_ENVIRONMENT_SCHEMA.into(),
                runtime_configuration: RuntimeConfiguration {
                    schema: RUNTIME_CONFIGURATION_SCHEMA.into(),
                    content_sha256: Digest([1; 32]),
                    language: "en".into(),
                    settings: BTreeMap::new(),
                },
                active_runtime_file: RuntimeFile {
                    id: "file-0".into(),
                    origin: RuntimeFileOrigin::TitleFile0,
                    backing: BackingAttachment::MemoryOnly,
                    allowed_serialization_targets: Vec::new(),
                    lifecycle: RuntimeFileLifecycle::Active,
                },
                physical_slots: Vec::new(),
                physical_slot_observations: Vec::new(),
                location: SceneLocation {
                    stage: "F_SP103".into(),
                    room: 0,
                    layer: 0,
                    spawn: 0,
                },
                player: PlayerState {
                    form: PlayerForm::Human,
                    mount: None,
                    position: [0.0; 3],
                    rotation: [0; 3],
                    has_control: Some(true),
                    action: "idle".into(),
                },
                components: vec![component(known_mask)],
                static_world_objects: Vec::new(),
                persisted_object_controls: Vec::new(),
                live_world_objects: Vec::new(),
            },
            semantic_observations: Vec::new(),
        }
    }

    fn scope(snapshot: &StateSnapshot) -> ContextScope {
        ContextScope {
            selectors: vec![ContextSelector::Exact {
                context: snapshot
                    .environment
                    .runtime_configuration
                    .exact_context()
                    .unwrap(),
            }],
        }
    }

    fn facts(snapshot: &StateSnapshot, alias_truth: TruthStatus) -> FactCatalog {
        let scope = scope(snapshot);
        FactCatalog {
            schema: FACT_CATALOG_SCHEMA.into(),
            aliases: vec![FriendlyAlias {
                id: "story.faron.twilight".into(),
                label: "Faron is in twilight".into(),
                scope: scope.clone(),
                raw: RawFactBinding {
                    component_kind: ComponentKind::PersistentSave,
                    binding: ComponentBinding::Global,
                    byte_offset: 0,
                    mask: vec![0x20],
                    expected: vec![0x20],
                },
                evidence: evidence(alias_truth),
            }],
            derived_facts: vec![DerivedFact {
                id: "world.faron.twilight-access".into(),
                label: "Faron twilight access state".into(),
                scope,
                rule: PredicateExpression::Fact {
                    fact_id: "story.faron.twilight".into(),
                },
                evidence: evidence(TruthStatus::Established),
            }],
        }
    }

    fn evaluator<'a>(
        snapshot: &'a StateSnapshot,
        facts: &'a FactCatalog,
        policy: EvidencePolicy,
    ) -> PredicateEvaluator<'a> {
        PredicateEvaluator::new(snapshot, facts, &[], &BTreeMap::new(), policy).unwrap()
    }

    fn fact(id: &str) -> PredicateExpression {
        PredicateExpression::Fact { fact_id: id.into() }
    }

    fn transition(snapshot: &StateSnapshot, guard: PredicateExpression) -> CandidateTransition {
        CandidateTransition {
            id: "transition.test".into(),
            label: "Test transition".into(),
            scope: scope(snapshot),
            transition_kind: TransitionKind::Door,
            approach_id: "approach.front".into(),
            activation: ActivationContract {
                hard_guards: guard,
                physical_obligation_ids: vec!["obligation.wall".into()],
                effects: Vec::new(),
                unknown_requirements: Vec::new(),
            },
            evidence: evidence(TruthStatus::Established),
        }
    }

    #[test]
    fn aliases_and_derived_facts_resolve_from_known_raw_state() {
        let snapshot = snapshot(0xff);
        let facts = facts(&snapshot, TruthStatus::Established);
        let evaluator = evaluator(&snapshot, &facts, EvidencePolicy::ESTABLISHED_ONLY);
        assert_eq!(
            evaluator.evaluate(&fact("story.faron.twilight")),
            EvaluatedTruth::True
        );
        assert_eq!(
            evaluator.evaluate(&fact("world.faron.twilight-access")),
            EvaluatedTruth::True
        );
    }

    #[test]
    fn missing_known_bits_and_disallowed_evidence_remain_unknown() {
        let unknown_snapshot = snapshot(0xdf);
        let established = facts(&unknown_snapshot, TruthStatus::Established);
        assert_eq!(
            evaluator(
                &unknown_snapshot,
                &established,
                EvidencePolicy::ESTABLISHED_ONLY,
            )
            .evaluate(&fact("story.faron.twilight")),
            EvaluatedTruth::Unknown
        );

        let observed_snapshot = snapshot(0xff);
        let hypothetical = facts(&observed_snapshot, TruthStatus::Hypothetical);
        assert_eq!(
            evaluator(
                &observed_snapshot,
                &hypothetical,
                EvidencePolicy::ESTABLISHED_ONLY,
            )
            .evaluate(&fact("story.faron.twilight")),
            EvaluatedTruth::Unknown
        );
        assert_eq!(
            evaluator(&observed_snapshot, &hypothetical, EvidencePolicy::RESEARCH,)
                .evaluate(&fact("story.faron.twilight")),
            EvaluatedTruth::True
        );
    }

    #[test]
    fn equivalence_scope_requires_an_explicit_evidenced_set() {
        let snapshot = snapshot(0xff);
        let context = snapshot
            .environment
            .runtime_configuration
            .exact_context()
            .unwrap();
        let scope = ContextScope {
            selectors: vec![ContextSelector::Equivalent {
                equivalence_set_id: "equivalence.sd".into(),
            }],
        };
        let facts = FactCatalog {
            schema: FACT_CATALOG_SCHEMA.into(),
            aliases: vec![FriendlyAlias {
                id: "story.faron.twilight".into(),
                label: "Faron is in twilight".into(),
                scope,
                raw: RawFactBinding {
                    component_kind: ComponentKind::PersistentSave,
                    binding: ComponentBinding::Global,
                    byte_offset: 0,
                    mask: vec![0x20],
                    expected: vec![0x20],
                },
                evidence: evidence(TruthStatus::Established),
            }],
            derived_facts: Vec::new(),
        };
        assert_eq!(
            PredicateEvaluator::new(
                &snapshot,
                &facts,
                &[],
                &BTreeMap::new(),
                EvidencePolicy::ESTABLISHED_ONLY,
            )
            .unwrap()
            .evaluate(&fact("story.faron.twilight")),
            EvaluatedTruth::Unknown
        );

        let mut contexts = vec![
            context,
            ExactContext {
                content_sha256: Digest([2; 32]),
                runtime_configuration_sha256: Digest([3; 32]),
            },
        ];
        contexts.sort();
        let equivalence = EquivalenceSet {
            schema: EQUIVALENCE_SET_SCHEMA.into(),
            id: "equivalence.sd".into(),
            semantic_scope: "story-flags".into(),
            contexts,
            evidence: vec![EquivalenceEvidence {
                kind: EquivalenceEvidenceKind::StaticDiff,
                source_id: "diff.sd".into(),
                source_sha256: Digest([4; 32]),
            }],
        };
        assert_eq!(
            PredicateEvaluator::new(
                &snapshot,
                &facts,
                &[equivalence],
                &BTreeMap::new(),
                EvidencePolicy::ESTABLISHED_ONLY,
            )
            .unwrap()
            .evaluate(&fact("story.faron.twilight")),
            EvaluatedTruth::True
        );
    }

    #[test]
    fn transition_assessment_separates_guards_obligations_and_unknowns() {
        let snapshot = snapshot(0xff);
        let facts = facts(&snapshot, TruthStatus::Established);
        let evaluator = evaluator(&snapshot, &facts, EvidencePolicy::ESTABLISHED_ONLY);
        let mut candidate = transition(&snapshot, fact("story.faron.twilight"));

        let upper =
            evaluator.assess_transition(&candidate, &BTreeSet::new(), FeasibilityMode::UpperBound);
        assert_eq!(upper.classification, TransitionClassification::Executable);
        assert_eq!(upper.outstanding_obligation_ids, vec!["obligation.wall"]);

        let modeled =
            evaluator.assess_transition(&candidate, &BTreeSet::new(), FeasibilityMode::Modeled);
        assert_eq!(modeled.classification, TransitionClassification::Obstructed);

        candidate.activation.hard_guards = PredicateExpression::False;
        assert_eq!(
            evaluator
                .assess_transition(&candidate, &BTreeSet::new(), FeasibilityMode::Modeled,)
                .classification,
            TransitionClassification::GuardBlocked
        );

        candidate.activation.hard_guards = PredicateExpression::True;
        candidate.activation.unknown_requirements = vec![UnknownRequirement {
            id: "unknown.trigger-semantics".into(),
            description: "The encoded exit does not establish activation physics.".into(),
            evidence: evidence(TruthStatus::Established),
        }];
        let assessment = evaluator.assess_transition(
            &candidate,
            &BTreeSet::from(["obligation.wall".into()]),
            FeasibilityMode::UpperBound,
        );
        assert_eq!(
            assessment.classification,
            TransitionClassification::FeasibilityUnknown
        );
        assert_eq!(
            assessment.unknown_requirement_ids,
            vec!["unknown.trigger-semantics"]
        );
    }

    #[test]
    fn writer_activation_and_gate_suppression_are_distinct_states() {
        let snapshot = snapshot(0xff);
        let facts = facts(&snapshot, TruthStatus::Established);
        let evaluator = evaluator(&snapshot, &facts, EvidencePolicy::ESTABLISHED_ONLY);
        let writer = WriterRule {
            id: "writer.return-place".into(),
            scope: scope(&snapshot),
            activation: PredicateExpression::True,
            operation: crate::transition::StateOperation::SetLocation {
                location: snapshot.environment.location.clone(),
            },
            evidence: evidence(TruthStatus::Established),
        };
        let mut gate = GateRule {
            id: "gate.no-teleport".into(),
            scope: scope(&snapshot),
            active_when: PredicateExpression::True,
            blocked_writer_ids: vec![writer.id.clone()],
            lifetime: SemanticLifetime::RuntimeFile,
            evidence: evidence(TruthStatus::Established),
        };

        let blocked = evaluator.assess_writer(&writer, &[gate.clone()]);
        assert_eq!(blocked.classification, WriterClassification::GateBlocked);
        assert_eq!(blocked.active_gate_ids, vec!["gate.no-teleport"]);

        gate.active_when = PredicateExpression::Fact {
            fact_id: "missing.gate-source".into(),
        };
        let uncertain = evaluator.assess_writer(&writer, &[gate.clone()]);
        assert_eq!(uncertain.classification, WriterClassification::GateUnknown);
        assert_eq!(uncertain.unknown_gate_ids, vec!["gate.no-teleport"]);

        gate.active_when = PredicateExpression::False;
        assert_eq!(
            evaluator.assess_writer(&writer, &[gate]).classification,
            WriterClassification::Executable
        );

        let mut inactive = writer;
        inactive.activation = PredicateExpression::False;
        assert_eq!(
            evaluator.assess_writer(&inactive, &[]).classification,
            WriterClassification::Inactive
        );
    }

    #[test]
    fn readers_keep_raw_source_and_friendly_interpretation_separate() {
        let snapshot = snapshot(0xff);
        let facts = facts(&snapshot, TruthStatus::Established);
        let evaluator = evaluator(&snapshot, &facts, EvidencePolicy::ESTABLISHED_ONLY);
        let reader = ReaderRule {
            id: "reader.savewarp".into(),
            scope: scope(&snapshot),
            source: ValueReference::LocationStage,
            consuming_transition_id: "transition.savewarp".into(),
            interpretation_fact_id: Some("story.faron.twilight".into()),
            evidence: evidence(TruthStatus::Established),
        };
        let assessment = evaluator.assess_reader(&reader);
        assert_eq!(
            assessment.source_value,
            Some(StateValue::Text("F_SP103".into()))
        );
        assert_eq!(assessment.interpretation, Some(EvaluatedTruth::True));
    }

    #[test]
    fn resolvers_discharge_obligations_without_deleting_active_obstructions() {
        let snapshot = snapshot(0xff);
        let facts = facts(&snapshot, TruthStatus::Established);
        let evaluator = evaluator(&snapshot, &facts, EvidencePolicy::ESTABLISHED_ONLY);
        let candidate = transition(&snapshot, PredicateExpression::True);
        let mut obstruction = Obstruction {
            id: "obstruction.npc-blocker".into(),
            label: "NPCs block the entrance".into(),
            scope: scope(&snapshot),
            blocked_action_id: candidate.id.clone(),
            approach_id: candidate.approach_id.clone(),
            active_when: PredicateExpression::True,
            obligation_ids: vec!["obligation.wall".into()],
            evidence: evidence(TruthStatus::Established),
        };
        let resolver = ObstructionResolver {
            id: "resolver.text-displacement".into(),
            label: "Displace the blocking text state".into(),
            scope: scope(&snapshot),
            obstruction_id: obstruction.id.clone(),
            resolution_kind: crate::transition::ResolutionKind::Bypass,
            applicable_when: fact("story.faron.twilight"),
            operations: Vec::new(),
            evidence: evidence(TruthStatus::Established),
        };

        let unresolved = evaluator.resolve_feasibility(
            &candidate,
            &[obstruction.clone()],
            &[],
            &[],
            FeasibilitySelection {
                resolver_ids: &BTreeSet::new(),
                technique_ids: &BTreeSet::new(),
                already_discharged: &BTreeSet::new(),
            },
        );
        assert_eq!(
            unresolved.active_obstruction_ids,
            vec!["obstruction.npc-blocker"]
        );
        assert!(
            !unresolved
                .discharged_obligation_ids
                .contains("obligation.wall")
        );

        let resolved = evaluator.resolve_feasibility(
            &candidate,
            &[obstruction.clone()],
            &[resolver],
            &[],
            FeasibilitySelection {
                resolver_ids: &BTreeSet::from(["resolver.text-displacement".into()]),
                technique_ids: &BTreeSet::new(),
                already_discharged: &BTreeSet::new(),
            },
        );
        assert_eq!(
            resolved.active_obstruction_ids,
            vec!["obstruction.npc-blocker"]
        );
        assert_eq!(
            resolved.applied_resolver_ids,
            vec!["resolver.text-displacement"]
        );
        assert!(
            resolved
                .discharged_obligation_ids
                .contains("obligation.wall")
        );

        obstruction.active_when = fact("missing.obstruction-state");
        let uncertain = evaluator.resolve_feasibility(
            &candidate,
            &[obstruction],
            &[],
            &[],
            FeasibilitySelection {
                resolver_ids: &BTreeSet::new(),
                technique_ids: &BTreeSet::new(),
                already_discharged: &BTreeSet::new(),
            },
        );
        assert_eq!(
            uncertain.unknown_obstruction_ids,
            vec!["obstruction.npc-blocker"]
        );
    }

    #[test]
    fn techniques_discharge_and_introduce_only_named_obligations() {
        let snapshot = snapshot(0xff);
        let facts = facts(&snapshot, TruthStatus::Established);
        let evaluator = evaluator(&snapshot, &facts, EvidencePolicy::ESTABLISHED_ONLY);
        let candidate = transition(&snapshot, PredicateExpression::True);
        let technique = Technique {
            id: "technique.epona-oob".into(),
            label: "Epona out of bounds".into(),
            scope: scope(&snapshot),
            prerequisites: fact("story.faron.twilight"),
            operations: Vec::new(),
            discharged_obligation_ids: vec!["obligation.wall".into()],
            introduced_obligation_ids: vec!["obligation.precise-movement".into()],
            cost: crate::transition::RouteCost {
                axes: BTreeMap::from([("difficulty".into(), 5)]),
            },
            evidence: evidence(TruthStatus::Established),
        };
        let resolution = evaluator.resolve_feasibility(
            &candidate,
            &[],
            &[],
            &[technique],
            FeasibilitySelection {
                resolver_ids: &BTreeSet::new(),
                technique_ids: &BTreeSet::from(["technique.epona-oob".into()]),
                already_discharged: &BTreeSet::from(["obligation.precise-movement".into()]),
            },
        );
        assert_eq!(
            resolution.applicable_technique_ids,
            vec!["technique.epona-oob"]
        );
        assert!(
            resolution
                .discharged_obligation_ids
                .contains("obligation.wall")
        );
        assert!(
            !resolution
                .discharged_obligation_ids
                .contains("obligation.precise-movement")
        );
    }
}
