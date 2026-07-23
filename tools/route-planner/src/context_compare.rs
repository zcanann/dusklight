//! Exact-context semantic comparison without nearest-build or locale fallback.

use crate::RuntimeEvidenceMode;
use crate::inspection::{InspectedFact, inspect_state};
use dusklight_route_planner::PlannerContractError;
use dusklight_route_planner::artifact::Digest;
use dusklight_route_planner::execution::PlannerExecutionState;
use dusklight_route_planner::identity::{EquivalenceSet, ExactContext, RuntimeConfiguration};
use dusklight_route_planner::refinement::ComposedPlannerCatalog;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const CONTEXT_COMPARISON_SCHEMA: &str =
    "dusklight.route-planner.semantic-context-comparison/v1";

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticContextComparison {
    pub schema: String,
    pub relation: ContextRelation,
    pub fallback_used: bool,
    pub evidence_mode: RuntimeEvidenceMode,
    pub left: ComparedContext,
    pub right: ComparedContext,
    pub facts: Vec<FactComparison>,
    pub mechanics: Vec<MechanicsRecordComparison>,
    pub summary: ComparisonSummary,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextRelation {
    ExactSame,
    SameContentDifferentRuntime,
    DifferentContent,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComparedContext {
    pub exact_context: ExactContext,
    pub runtime_configuration: RuntimeConfiguration,
    pub execution_state_sha256: Digest,
    pub composed_catalog_sha256: Digest,
    pub base_fact_catalog_sha256: Digest,
    pub base_mechanics_catalog_sha256: Digest,
    pub refinement_stack_sha256: Digest,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FactComparisonKind {
    Equivalent,
    OutcomeChanged,
    BindingChanged,
    ContractChanged,
    LeftOnly,
    RightOnly,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FactComparison {
    pub id: String,
    pub comparison: FactComparisonKind,
    pub left: Option<InspectedFact>,
    pub right: Option<InspectedFact>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MechanicsComparisonKind {
    Equivalent,
    ContractChanged,
    LeftOnly,
    RightOnly,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MechanicsRecordComparison {
    pub family: String,
    pub id: String,
    pub comparison: MechanicsComparisonKind,
    pub left_sha256: Option<Digest>,
    pub right_sha256: Option<Digest>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComparisonSummary {
    pub fact_counts: BTreeMap<String, usize>,
    pub mechanics_counts: BTreeMap<String, usize>,
    pub left_inapplicable_fact_ids: Vec<String>,
    pub right_inapplicable_fact_ids: Vec<String>,
}

pub fn compare_semantic_contexts(
    left_state: &PlannerExecutionState,
    left_catalog: &ComposedPlannerCatalog,
    left_equivalence_sets: &[EquivalenceSet],
    right_state: &PlannerExecutionState,
    right_catalog: &ComposedPlannerCatalog,
    right_equivalence_sets: &[EquivalenceSet],
    evidence_mode: RuntimeEvidenceMode,
) -> Result<SemanticContextComparison, PlannerContractError> {
    left_catalog.validate()?;
    right_catalog.validate()?;
    let left_inspection = inspect_state(
        left_state,
        &left_catalog.facts,
        left_equivalence_sets,
        evidence_mode,
    )?;
    let right_inspection = inspect_state(
        right_state,
        &right_catalog.facts,
        right_equivalence_sets,
        evidence_mode,
    )?;
    let left_context = compared_context(left_state, left_catalog)?;
    let right_context = compared_context(right_state, right_catalog)?;
    let relation = if left_context.exact_context == right_context.exact_context {
        ContextRelation::ExactSame
    } else if left_context.exact_context.content_sha256
        == right_context.exact_context.content_sha256
    {
        ContextRelation::SameContentDifferentRuntime
    } else {
        ContextRelation::DifferentContent
    };

    let left_facts = left_inspection
        .facts
        .into_iter()
        .map(|fact| (fact.id.clone(), fact))
        .collect::<BTreeMap<_, _>>();
    let right_facts = right_inspection
        .facts
        .into_iter()
        .map(|fact| (fact.id.clone(), fact))
        .collect::<BTreeMap<_, _>>();
    let fact_ids = left_facts
        .keys()
        .chain(right_facts.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    let facts = fact_ids
        .into_iter()
        .map(|id| {
            let left = left_facts.get(&id).cloned();
            let right = right_facts.get(&id).cloned();
            let comparison = compare_fact(left.as_ref(), right.as_ref());
            FactComparison {
                id,
                comparison,
                left,
                right,
            }
        })
        .collect::<Vec<_>>();
    let mechanics = compare_mechanics(left_catalog, right_catalog)?;
    let summary = ComparisonSummary {
        fact_counts: count_fact_comparisons(&facts),
        mechanics_counts: count_mechanics_comparisons(&mechanics),
        left_inapplicable_fact_ids: facts
            .iter()
            .filter_map(|row| {
                row.left
                    .as_ref()
                    .filter(|fact| !fact.scope_applies)
                    .map(|_| row.id.clone())
            })
            .collect(),
        right_inapplicable_fact_ids: facts
            .iter()
            .filter_map(|row| {
                row.right
                    .as_ref()
                    .filter(|fact| !fact.scope_applies)
                    .map(|_| row.id.clone())
            })
            .collect(),
    };
    Ok(SemanticContextComparison {
        schema: CONTEXT_COMPARISON_SCHEMA.into(),
        relation,
        fallback_used: false,
        evidence_mode,
        left: left_context,
        right: right_context,
        facts,
        mechanics,
        summary,
    })
}

fn compared_context(
    state: &PlannerExecutionState,
    catalog: &ComposedPlannerCatalog,
) -> Result<ComparedContext, PlannerContractError> {
    let runtime_configuration = state.snapshot.environment.runtime_configuration.clone();
    let exact_context = runtime_configuration.exact_context()?;
    let refinement_stack_sha256 = catalog.refinement_stack.digest()?;
    Ok(ComparedContext {
        exact_context,
        runtime_configuration,
        execution_state_sha256: state.digest()?,
        composed_catalog_sha256: catalog.digest()?,
        base_fact_catalog_sha256: catalog.base_fact_catalog_sha256,
        base_mechanics_catalog_sha256: catalog.base_mechanics_catalog_sha256,
        refinement_stack_sha256,
    })
}

fn compare_fact(left: Option<&InspectedFact>, right: Option<&InspectedFact>) -> FactComparisonKind {
    match (left, right) {
        (Some(_), None) => FactComparisonKind::LeftOnly,
        (None, Some(_)) => FactComparisonKind::RightOnly,
        (Some(left), Some(right)) if left.evaluated != right.evaluated => {
            FactComparisonKind::OutcomeChanged
        }
        (Some(left), Some(right)) if left.raw_binding != right.raw_binding => {
            FactComparisonKind::BindingChanged
        }
        (Some(left), Some(right))
            if left.label == right.label
                && left.source_kind == right.source_kind
                && left.authored_truth == right.authored_truth
                && left.scope_applies == right.scope_applies
                && left.evidence_permitted == right.evidence_permitted =>
        {
            FactComparisonKind::Equivalent
        }
        (Some(_), Some(_)) => FactComparisonKind::ContractChanged,
        (None, None) => unreachable!("fact ID union cannot contain an absent row"),
    }
}

fn compare_mechanics(
    left: &ComposedPlannerCatalog,
    right: &ComposedPlannerCatalog,
) -> Result<Vec<MechanicsRecordComparison>, PlannerContractError> {
    let left = mechanics_records(left)?;
    let right = mechanics_records(right)?;
    let keys = left
        .keys()
        .chain(right.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    Ok(keys
        .into_iter()
        .map(|(family, id)| {
            let left_sha256 = left.get(&(family.clone(), id.clone())).copied();
            let right_sha256 = right.get(&(family.clone(), id.clone())).copied();
            let comparison = match (left_sha256, right_sha256) {
                (Some(left), Some(right)) if left == right => MechanicsComparisonKind::Equivalent,
                (Some(_), Some(_)) => MechanicsComparisonKind::ContractChanged,
                (Some(_), None) => MechanicsComparisonKind::LeftOnly,
                (None, Some(_)) => MechanicsComparisonKind::RightOnly,
                (None, None) => unreachable!("mechanics key union cannot contain an absent row"),
            };
            MechanicsRecordComparison {
                family,
                id,
                comparison,
                left_sha256,
                right_sha256,
            }
        })
        .collect())
}

fn mechanics_records(
    catalog: &ComposedPlannerCatalog,
) -> Result<BTreeMap<(String, String), Digest>, PlannerContractError> {
    let Value::Object(mechanics) = serde_json::to_value(&catalog.mechanics)? else {
        unreachable!("mechanics catalog always serializes as an object")
    };
    let mut records = BTreeMap::new();
    for (family, value) in mechanics {
        let Value::Array(values) = value else {
            continue;
        };
        for value in values {
            let id = value
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    PlannerContractError::new(
                        "context_comparison.mechanics",
                        format!("{family} contains a record without a stable id"),
                    )
                })?
                .to_owned();
            let digest = Digest(Sha256::digest(serde_json::to_vec(&value)?).into());
            if records
                .insert((family.clone(), id.clone()), digest)
                .is_some()
            {
                return Err(PlannerContractError::new(
                    "context_comparison.mechanics",
                    format!("{family} contains duplicate id {id}"),
                ));
            }
        }
    }
    Ok(records)
}

fn count_fact_comparisons(facts: &[FactComparison]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for row in facts {
        let key = match row.comparison {
            FactComparisonKind::Equivalent => "equivalent",
            FactComparisonKind::OutcomeChanged => "outcome_changed",
            FactComparisonKind::BindingChanged => "binding_changed",
            FactComparisonKind::ContractChanged => "contract_changed",
            FactComparisonKind::LeftOnly => "left_only",
            FactComparisonKind::RightOnly => "right_only",
        };
        *counts.entry(key.into()).or_default() += 1;
    }
    counts
}

fn count_mechanics_comparisons(mechanics: &[MechanicsRecordComparison]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for row in mechanics {
        let key = match row.comparison {
            MechanicsComparisonKind::Equivalent => "equivalent",
            MechanicsComparisonKind::ContractChanged => "contract_changed",
            MechanicsComparisonKind::LeftOnly => "left_only",
            MechanicsComparisonKind::RightOnly => "right_only",
        };
        *counts.entry(key.into()).or_default() += 1;
    }
    counts
}
