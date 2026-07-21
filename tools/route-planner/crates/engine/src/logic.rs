//! Ground facts, semantic aliases, derived predicates, and evidence scope.

use crate::artifact::Digest;
use crate::identity::ContextSelector;
use crate::state::{
    ComponentBinding, ComponentKind, StateValue, validate_binding, validate_component_kind,
};
use crate::{PlannerContractError, canonical_json, validate_label, validate_stable_id};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const FACT_CATALOG_SCHEMA: &str = "dusklight.route-planner.fact-catalog/v4";
pub const MAX_PREDICATE_DEPTH: usize = 64;
pub const MAX_PREDICATE_CHILDREN: usize = 4_096;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TruthStatus {
    Established,
    Contested,
    Hypothetical,
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    Extracted,
    SourceAudited,
    TraceObserved,
    RouteWitnessed,
    CommunityReported,
    Theorycraft,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRecord {
    pub id: String,
    pub kind: EvidenceKind,
    pub source_sha256: Option<Digest>,
    pub note: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuleEvidence {
    pub truth: TruthStatus,
    pub records: Vec<EvidenceRecord>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContextScope {
    pub selectors: Vec<ContextSelector>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonOperator {
    Equal,
    NotEqual,
    LessThan,
    LessThanOrEqual,
    GreaterThan,
    GreaterThanOrEqual,
    ContainsBits,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ValueReference {
    Literal {
        value: StateValue,
    },
    ComponentField {
        component_id: String,
        field: String,
    },
    BoundComponentField {
        component_kind: ComponentKind,
        binding: ComponentBinding,
        field: String,
    },
    RawBits {
        component_id: String,
        byte_offset: u32,
        byte_width: u8,
        mask: u64,
    },
    BoundRawBits {
        component_kind: ComponentKind,
        binding: ComponentBinding,
        byte_offset: u32,
        byte_width: u8,
        mask: u64,
    },
    RuntimeLanguage,
    RuntimeSetting {
        key: String,
    },
    LocationStage,
    LocationRoom,
    LocationLayer,
    LocationSpawn,
    PlayerForm,
    PlayerMount,
    PlayerControl,
    PlayerRotationX,
    PlayerRotationY,
    PlayerRotationZ,
    PlayerAction,
    ActorField {
        instance_id: String,
        field: String,
    },
    GateState {
        gate_id: String,
    },
    FlowNode {
        flow_component_id: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PredicateExpression {
    True,
    False,
    Compare {
        left: ValueReference,
        operator: ComparisonOperator,
        right: ValueReference,
    },
    Fact {
        fact_id: String,
    },
    All {
        terms: Vec<PredicateExpression>,
    },
    Any {
        terms: Vec<PredicateExpression>,
    },
    Not {
        term: Box<PredicateExpression>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawFactBinding {
    pub component_kind: ComponentKind,
    pub binding: ComponentBinding,
    pub byte_offset: u32,
    pub mask: Vec<u8>,
    pub expected: Vec<u8>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FriendlyAlias {
    pub id: String,
    pub label: String,
    pub scope: ContextScope,
    pub raw: RawFactBinding,
    pub evidence: RuleEvidence,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DerivedFact {
    pub id: String,
    pub label: String,
    pub scope: ContextScope,
    pub rule: PredicateExpression,
    pub evidence: RuleEvidence,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FactCatalog {
    pub schema: String,
    pub aliases: Vec<FriendlyAlias>,
    pub derived_facts: Vec<DerivedFact>,
}

impl RuleEvidence {
    pub fn validate(&self, field: &str) -> Result<(), PlannerContractError> {
        if self.records.len() > 256 {
            return Err(PlannerContractError::new(
                field,
                "must contain at most 256 evidence records",
            ));
        }
        if matches!(
            self.truth,
            TruthStatus::Established | TruthStatus::Contested
        ) && self.records.is_empty()
        {
            return Err(PlannerContractError::new(
                field,
                "established or contested truth requires evidence",
            ));
        }
        let mut ids = BTreeSet::new();
        for record in &self.records {
            validate_stable_id(&format!("{field}.id"), &record.id)?;
            validate_label(&format!("{field}.note"), &record.note)?;
            if record.source_sha256 == Some(Digest::ZERO) {
                return Err(PlannerContractError::new(
                    format!("{field}.source_sha256"),
                    "must be absent or nonzero",
                ));
            }
            if !ids.insert(record.id.as_str()) {
                return Err(PlannerContractError::new(
                    field,
                    "contains duplicate evidence IDs",
                ));
            }
        }
        Ok(())
    }
}

impl ContextScope {
    pub fn validate(&self, field: &str) -> Result<(), PlannerContractError> {
        if self.selectors.is_empty() || self.selectors.len() > 64 {
            return Err(PlannerContractError::new(
                field,
                "must contain between 1 and 64 exact/equivalence selectors",
            ));
        }
        let mut encoded = BTreeSet::new();
        for selector in &self.selectors {
            selector.validate()?;
            let key = serde_json::to_string(selector)?;
            if !encoded.insert(key) {
                return Err(PlannerContractError::new(
                    field,
                    "contains a duplicate context selector",
                ));
            }
        }
        Ok(())
    }
}

impl PredicateExpression {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        self.validate_at_depth(0)
    }

    pub fn referenced_facts(&self, output: &mut BTreeSet<String>) {
        match self {
            Self::Fact { fact_id } => {
                output.insert(fact_id.clone());
            }
            Self::All { terms } | Self::Any { terms } => {
                for term in terms {
                    term.referenced_facts(output);
                }
            }
            Self::Not { term } => term.referenced_facts(output),
            Self::True | Self::False | Self::Compare { .. } => {}
        }
    }

    fn validate_at_depth(&self, depth: usize) -> Result<(), PlannerContractError> {
        if depth > MAX_PREDICATE_DEPTH {
            return Err(PlannerContractError::new(
                "predicate",
                "exceeds maximum nesting depth",
            ));
        }
        match self {
            Self::Compare { left, right, .. } => {
                validate_value_reference(left)?;
                validate_value_reference(right)
            }
            Self::Fact { fact_id } => validate_stable_id("predicate.fact_id", fact_id),
            Self::All { terms } | Self::Any { terms } => {
                if terms.is_empty() || terms.len() > MAX_PREDICATE_CHILDREN {
                    return Err(PlannerContractError::new(
                        "predicate.terms",
                        "must contain between 1 and 4096 terms",
                    ));
                }
                for term in terms {
                    term.validate_at_depth(depth + 1)?;
                }
                Ok(())
            }
            Self::Not { term } => term.validate_at_depth(depth + 1),
            Self::True | Self::False => Ok(()),
        }
    }
}

impl FactCatalog {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != FACT_CATALOG_SCHEMA {
            return Err(PlannerContractError::new("schema", "is unsupported"));
        }
        if self.aliases.len() + self.derived_facts.len() > 65_536 {
            return Err(PlannerContractError::new(
                "facts",
                "catalog contains too many facts",
            ));
        }
        let mut all_ids = BTreeSet::new();
        let mut previous_alias = None;
        for alias in &self.aliases {
            validate_stable_id("aliases.id", &alias.id)?;
            validate_label("aliases.label", &alias.label)?;
            alias.scope.validate("aliases.scope")?;
            alias.evidence.validate("aliases.evidence")?;
            validate_raw_binding(&alias.raw)?;
            if !all_ids.insert(alias.id.as_str())
                || previous_alias.is_some_and(|prior: &str| prior >= alias.id.as_str())
            {
                return Err(PlannerContractError::new(
                    "aliases",
                    "must be unique and sorted by ID",
                ));
            }
            previous_alias = Some(alias.id.as_str());
        }
        let mut derived_by_id = BTreeMap::new();
        let mut previous_derived = None;
        for fact in &self.derived_facts {
            validate_stable_id("derived_facts.id", &fact.id)?;
            validate_label("derived_facts.label", &fact.label)?;
            fact.scope.validate("derived_facts.scope")?;
            fact.evidence.validate("derived_facts.evidence")?;
            fact.rule.validate()?;
            if !all_ids.insert(fact.id.as_str())
                || previous_derived.is_some_and(|prior: &str| prior >= fact.id.as_str())
            {
                return Err(PlannerContractError::new(
                    "derived_facts",
                    "must be unique and sorted by ID",
                ));
            }
            previous_derived = Some(fact.id.as_str());
            derived_by_id.insert(fact.id.as_str(), fact);
        }
        for fact in &self.derived_facts {
            let mut references = BTreeSet::new();
            fact.rule.referenced_facts(&mut references);
            for reference in references {
                if !all_ids.contains(reference.as_str()) {
                    return Err(PlannerContractError::new(
                        "derived_facts.rule",
                        format!("references unknown fact {reference}"),
                    ));
                }
            }
        }
        reject_derived_cycles(&derived_by_id)
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PlannerContractError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, PlannerContractError> {
        let catalog: Self = serde_json::from_slice(bytes)?;
        catalog.validate()?;
        if catalog.canonical_bytes()? != bytes {
            return Err(PlannerContractError::new(
                "fact_catalog",
                "is not canonical JSON",
            ));
        }
        Ok(catalog)
    }

    pub fn digest(&self) -> Result<Digest, PlannerContractError> {
        Ok(Digest(Sha256::digest(self.canonical_bytes()?).into()))
    }
}

fn validate_value_reference(reference: &ValueReference) -> Result<(), PlannerContractError> {
    match reference {
        ValueReference::Literal { value } => validate_literal(value),
        ValueReference::ComponentField {
            component_id,
            field,
        } => {
            validate_stable_id("value.component_id", component_id)?;
            validate_stable_id("value.field", field)
        }
        ValueReference::BoundComponentField {
            component_kind,
            binding,
            field,
        } => {
            validate_component_kind(component_kind)?;
            validate_binding(binding)?;
            validate_stable_id("value.field", field)
        }
        ValueReference::RawBits {
            component_id,
            byte_width,
            mask,
            ..
        } => {
            validate_stable_id("value.component_id", component_id)?;
            validate_raw_value_shape(*byte_width, *mask)
        }
        ValueReference::BoundRawBits {
            component_kind,
            binding,
            byte_width,
            mask,
            ..
        } => {
            validate_component_kind(component_kind)?;
            validate_binding(binding)?;
            validate_raw_value_shape(*byte_width, *mask)
        }
        ValueReference::RuntimeSetting { key } => validate_stable_id("value.key", key),
        ValueReference::ActorField { instance_id, field } => {
            validate_stable_id("value.instance_id", instance_id)?;
            validate_stable_id("value.field", field)
        }
        ValueReference::GateState { gate_id } => validate_stable_id("value.gate_id", gate_id),
        ValueReference::FlowNode { flow_component_id } => {
            validate_stable_id("value.flow_component_id", flow_component_id)
        }
        ValueReference::RuntimeLanguage
        | ValueReference::LocationStage
        | ValueReference::LocationRoom
        | ValueReference::LocationLayer
        | ValueReference::LocationSpawn
        | ValueReference::PlayerForm
        | ValueReference::PlayerMount
        | ValueReference::PlayerControl
        | ValueReference::PlayerRotationX
        | ValueReference::PlayerRotationY
        | ValueReference::PlayerRotationZ
        | ValueReference::PlayerAction => Ok(()),
    }
}

fn validate_raw_value_shape(byte_width: u8, mask: u64) -> Result<(), PlannerContractError> {
    if !(1..=8).contains(&byte_width) {
        return Err(PlannerContractError::new(
            "value.byte_width",
            "must be between 1 and 8",
        ));
    }
    let valid_mask = if byte_width == 8 {
        u64::MAX
    } else {
        (1_u64 << (u32::from(byte_width) * 8)) - 1
    };
    if mask == 0 || mask & !valid_mask != 0 {
        return Err(PlannerContractError::new(
            "value.mask",
            "must be nonzero and fit within byte_width",
        ));
    }
    Ok(())
}

fn validate_literal(value: &StateValue) -> Result<(), PlannerContractError> {
    match value {
        StateValue::Text(value) => validate_label("literal.text", value),
        StateValue::Bytes(value) if value.len() > 1024 * 1024 => Err(PlannerContractError::new(
            "literal.bytes",
            "must contain at most 1 MiB",
        )),
        _ => Ok(()),
    }
}

fn validate_raw_binding(binding: &RawFactBinding) -> Result<(), PlannerContractError> {
    if binding.mask.is_empty()
        || binding.mask.len() > 64
        || binding.mask.len() != binding.expected.len()
        || binding.mask.iter().all(|byte| *byte == 0)
        || binding
            .mask
            .iter()
            .zip(&binding.expected)
            .any(|(mask, expected)| expected & !mask != 0)
    {
        return Err(PlannerContractError::new(
            "aliases.raw",
            "mask/expected must be equal-length, 1-64 bytes, nonzero, and expected must fit mask",
        ));
    }
    Ok(())
}

fn reject_derived_cycles(facts: &BTreeMap<&str, &DerivedFact>) -> Result<(), PlannerContractError> {
    fn visit<'a>(
        id: &'a str,
        facts: &BTreeMap<&'a str, &'a DerivedFact>,
        visiting: &mut BTreeSet<&'a str>,
        complete: &mut BTreeSet<&'a str>,
    ) -> Result<(), PlannerContractError> {
        if complete.contains(id) {
            return Ok(());
        }
        if !visiting.insert(id) {
            return Err(PlannerContractError::new(
                "derived_facts",
                format!("contains a dependency cycle at {id}"),
            ));
        }
        let mut references = BTreeSet::new();
        facts[id].rule.referenced_facts(&mut references);
        for reference in references {
            if let Some((canonical, _)) = facts.get_key_value(reference.as_str()) {
                visit(canonical, facts, visiting, complete)?;
            }
        }
        visiting.remove(id);
        complete.insert(id);
        Ok(())
    }

    let mut complete = BTreeSet::new();
    for id in facts.keys().copied() {
        visit(id, facts, &mut BTreeSet::new(), &mut complete)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::ExactContext;

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

    fn evidence() -> RuleEvidence {
        RuleEvidence {
            truth: TruthStatus::Established,
            records: vec![EvidenceRecord {
                id: "source.save-layout".into(),
                kind: EvidenceKind::SourceAudited,
                source_sha256: Some(Digest([3; 32])),
                note: "Read directly from the save implementation.".into(),
            }],
        }
    }

    #[test]
    fn raw_alias_and_derived_fact_keep_physical_and_semantic_identity_separate() {
        let catalog = FactCatalog {
            schema: FACT_CATALOG_SCHEMA.into(),
            aliases: vec![FriendlyAlias {
                id: "story.faron.twilight".into(),
                label: "Faron is in twilight".into(),
                scope: scope(),
                raw: RawFactBinding {
                    component_kind: ComponentKind::PersistentSave,
                    binding: ComponentBinding::Global,
                    byte_offset: 4,
                    mask: vec![0x20],
                    expected: vec![0x20],
                },
                evidence: evidence(),
            }],
            derived_facts: vec![DerivedFact {
                id: "ability.charge-attack".into(),
                label: "Can charge an attack".into(),
                scope: scope(),
                rule: PredicateExpression::Fact {
                    fact_id: "story.faron.twilight".into(),
                },
                evidence: evidence(),
            }],
        };
        catalog.validate().unwrap();
        let bytes = catalog.canonical_bytes().unwrap();
        assert_eq!(FactCatalog::decode_canonical(&bytes).unwrap(), catalog);
        assert_ne!(catalog.digest().unwrap(), Digest::ZERO);
    }

    #[test]
    fn established_truth_requires_evidence_but_unknown_does_not_become_false() {
        let mut established = evidence();
        established.records.clear();
        assert_eq!(
            established.validate("evidence").unwrap_err().field(),
            "evidence"
        );

        RuleEvidence {
            truth: TruthStatus::Unknown,
            records: Vec::new(),
        }
        .validate("evidence")
        .unwrap();
    }

    #[test]
    fn missing_and_cyclic_fact_dependencies_fail_closed() {
        let mut catalog = FactCatalog {
            schema: FACT_CATALOG_SCHEMA.into(),
            aliases: Vec::new(),
            derived_facts: vec![DerivedFact {
                id: "a".into(),
                label: "A".into(),
                scope: scope(),
                rule: PredicateExpression::Fact {
                    fact_id: "missing".into(),
                },
                evidence: RuleEvidence {
                    truth: TruthStatus::Unknown,
                    records: Vec::new(),
                },
            }],
        };
        assert_eq!(
            catalog.validate().unwrap_err().field(),
            "derived_facts.rule"
        );

        catalog.derived_facts = vec![
            DerivedFact {
                id: "a".into(),
                label: "A".into(),
                scope: scope(),
                rule: PredicateExpression::Fact {
                    fact_id: "b".into(),
                },
                evidence: RuleEvidence {
                    truth: TruthStatus::Unknown,
                    records: Vec::new(),
                },
            },
            DerivedFact {
                id: "b".into(),
                label: "B".into(),
                scope: scope(),
                rule: PredicateExpression::Fact {
                    fact_id: "a".into(),
                },
                evidence: RuleEvidence {
                    truth: TruthStatus::Unknown,
                    records: Vec::new(),
                },
            },
        ];
        assert_eq!(catalog.validate().unwrap_err().field(), "derived_facts");
    }

    #[test]
    fn raw_bit_reference_must_fit_its_width() {
        let expression = PredicateExpression::Compare {
            left: ValueReference::RawBits {
                component_id: "save".into(),
                byte_offset: 0,
                byte_width: 1,
                mask: 0x100,
            },
            operator: ComparisonOperator::Equal,
            right: ValueReference::Literal {
                value: StateValue::Unsigned(0),
            },
        };
        assert_eq!(expression.validate().unwrap_err().field(), "value.mask");

        let bound = PredicateExpression::Compare {
            left: ValueReference::BoundRawBits {
                component_kind: ComponentKind::DungeonMemory,
                binding: ComponentBinding::Stage {
                    stage: "D_MN05".into(),
                },
                byte_offset: 0x1c,
                byte_width: 0,
                mask: 0xff,
            },
            operator: ComparisonOperator::GreaterThan,
            right: ValueReference::Literal {
                value: StateValue::Unsigned(0),
            },
        };
        assert_eq!(bound.validate().unwrap_err().field(), "value.byte_width");
    }
}
