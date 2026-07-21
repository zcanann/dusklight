//! Loss-aware predicate evaluation and transition readiness classification.

use crate::identity::{ConfigurationValue, ContextSelector, EquivalenceSet, ExactContext};
use crate::logic::{
    ComparisonOperator, ContextScope, FactCatalog, PredicateExpression, RawFactBinding,
    TruthStatus, ValueReference,
};
use crate::state::{ComponentPayload, PlayerForm, PlayerMount, StateComponent, StateValue};
use crate::transition::CandidateTransition;
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

    fn permits(self, truth: TruthStatus) -> bool {
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
            } => match (self.value(left), self.value(right)) {
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

    fn value(&self, reference: &ValueReference) -> Option<StateValue> {
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
}
