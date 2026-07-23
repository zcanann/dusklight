//! Evidence-only promotion of observed actions through composable refinement packs.

use crate::artifact::Digest;
use crate::evaluation::EvaluatedTruth;
use crate::identity::ContextSelector;
use crate::logic::{ContextScope, EvidenceKind, EvidenceRecord, RuleEvidence, TruthStatus};
use crate::refinement::{
    ComposedPlannerCatalog, PackDependency, REFINEMENT_PACK_SCHEMA, RefinementOperation,
    RefinementPack, RefinementPackManifest, RefinementRule, ReplacementKind,
};
use crate::route_book::RouteActionRef;
use crate::route_observation_validation::{RouteObservationValidationReport, VerificationStatus};
use crate::{
    PlannerContractError, canonical_json, require_canonical_json_bytes, validate_label,
    validate_stable_id,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const WITNESS_PROMOTION_REQUEST_SCHEMA: &str =
    "dusklight.route-planner.witness-promotion-request/v1";
pub const WITNESS_PROMOTION_RECEIPT_SCHEMA: &str =
    "dusklight.route-planner.witness-promotion-receipt/v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WitnessPromotionPackMetadata {
    pub id: String,
    pub version: String,
    pub author: String,
    pub source: String,
    pub precedence: i32,
    pub conflicts: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RequestedWitness {
    pub observation_id: String,
    pub evidence_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RequestedActionPromotion {
    pub action: RouteActionRef,
    pub promotion_rule_id: String,
    pub replacement_rule_id: String,
    pub witnesses: Vec<RequestedWitness>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WitnessPromotionRequest {
    pub schema: String,
    pub pack: WitnessPromotionPackMetadata,
    pub promotions: Vec<RequestedActionPromotion>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WitnessPromotionReceiptRow {
    pub action: RouteActionRef,
    pub promotion_rule_id: String,
    pub replacement_rule_id: String,
    pub preserved_evidence_ids: Vec<String>,
    pub route_witness_evidence_ids: Vec<String>,
    pub observation_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WitnessPromotionReceipt {
    pub schema: String,
    pub composed_catalog_sha256: Digest,
    pub validation_report_sha256: Digest,
    pub pack_id: String,
    pub pack_sha256: Digest,
    pub action_ids_before: Vec<String>,
    pub action_ids_after: Vec<String>,
    pub promotions: Vec<WitnessPromotionReceiptRow>,
}

impl WitnessPromotionRequest {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != WITNESS_PROMOTION_REQUEST_SCHEMA || self.promotions.is_empty() {
            return Err(PlannerContractError::new(
                "witness_promotion_request",
                "has an invalid schema or no promotions",
            ));
        }
        validate_stable_id("pack.id", &self.pack.id)?;
        validate_version(&self.pack.version)?;
        validate_label("pack.author", &self.pack.author)?;
        validate_label("pack.source", &self.pack.source)?;
        validate_sorted_ids("pack.conflicts", &self.pack.conflicts, true)?;
        if self.pack.conflicts.iter().any(|id| id == &self.pack.id) {
            return Err(PlannerContractError::new(
                "pack.conflicts",
                "cannot include the generated pack itself",
            ));
        }
        let mut rule_ids = BTreeSet::new();
        let mut evidence_ids = BTreeSet::new();
        let mut observation_ids = BTreeSet::new();
        let mut previous_action = None;
        for promotion in &self.promotions {
            validate_action(&promotion.action)?;
            if previous_action
                .as_ref()
                .is_some_and(|prior| prior >= &promotion.action)
            {
                return Err(PlannerContractError::new(
                    "promotions",
                    "must be unique and sorted by action",
                ));
            }
            previous_action = Some(promotion.action.clone());
            for (field, id) in [
                ("promotions.promotion_rule_id", &promotion.promotion_rule_id),
                (
                    "promotions.replacement_rule_id",
                    &promotion.replacement_rule_id,
                ),
            ] {
                validate_stable_id(field, id)?;
                if !rule_ids.insert(id.as_str()) {
                    return Err(PlannerContractError::new(field, "must be globally unique"));
                }
            }
            if promotion.witnesses.is_empty()
                || promotion
                    .witnesses
                    .windows(2)
                    .any(|pair| pair[0] >= pair[1])
            {
                return Err(PlannerContractError::new(
                    "promotions.witnesses",
                    "must be nonempty, unique, and sorted",
                ));
            }
            for witness in &promotion.witnesses {
                validate_stable_id(
                    "promotions.witnesses.observation_id",
                    &witness.observation_id,
                )?;
                validate_stable_id("promotions.witnesses.evidence_id", &witness.evidence_id)?;
                if !observation_ids.insert(witness.observation_id.as_str())
                    || !evidence_ids.insert(witness.evidence_id.as_str())
                {
                    return Err(PlannerContractError::new(
                        "promotions.witnesses",
                        "observation and evidence IDs must be globally unique",
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PlannerContractError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, PlannerContractError> {
        let request: Self = serde_json::from_slice(bytes)?;
        request.validate()?;
        require_canonical_json_bytes(
            "witness_promotion_request",
            bytes,
            &request.canonical_bytes()?,
        )?;
        Ok(request)
    }
}

pub fn promote_witnessed_actions(
    catalog: &ComposedPlannerCatalog,
    validation: &RouteObservationValidationReport,
    request: &WitnessPromotionRequest,
) -> Result<(RefinementPack, WitnessPromotionReceipt), PlannerContractError> {
    catalog.validate()?;
    validation.validate()?;
    request.validate()?;
    if validation.composed_catalog_sha256 != catalog.digest()? {
        return Err(PlannerContractError::new(
            "witness_promotion.validation_report",
            "was built against another composed catalog",
        ));
    }
    if catalog
        .refinement_stack
        .entries
        .iter()
        .any(|entry| entry.pack_id == request.pack.id)
    {
        return Err(PlannerContractError::new(
            "witness_promotion.pack.id",
            "already exists in the composed catalog stack",
        ));
    }
    if catalog
        .refinement_stack
        .entries
        .iter()
        .any(|entry| request.pack.conflicts.contains(&entry.pack_id))
    {
        return Err(PlannerContractError::new(
            "witness_promotion.pack.conflicts",
            "conflicts with a dependency in the source catalog stack",
        ));
    }
    let validation_by_id = validation
        .validations
        .iter()
        .map(|row| (row.observation_id.as_str(), row))
        .collect::<BTreeMap<_, _>>();
    let validation_sha256 = validation.digest()?;
    let mut selectors = BTreeMap::<String, ContextSelector>::new();
    let mut rules = Vec::new();
    let mut receipt_rows = Vec::new();
    for requested in &request.promotions {
        let (scope, existing_evidence, mut promoted_operation) =
            promoted_operation(catalog, &requested.action)?;
        for selector in &scope.selectors {
            selectors.insert(serde_json::to_string(selector)?, selector.clone());
        }
        let mut witness_records = Vec::new();
        for witness in &requested.witnesses {
            let row = validation_by_id
                .get(witness.observation_id.as_str())
                .ok_or_else(|| {
                    PlannerContractError::new(
                        "promotions.witnesses.observation_id",
                        format!("references absent validation {}", witness.observation_id),
                    )
                })?;
            require_promotable(row, &requested.action)?;
            witness_records.push(EvidenceRecord {
                id: witness.evidence_id.clone(),
                kind: EvidenceKind::RouteWitnessed,
                source_sha256: Some(validation_sha256),
                note: format!(
                    "Observed action {} in validated window {}.",
                    action_id(&requested.action),
                    witness.observation_id
                ),
            });
        }
        let existing_ids = existing_evidence
            .records
            .iter()
            .map(|record| record.id.clone())
            .collect::<BTreeSet<_>>();
        if witness_records
            .iter()
            .any(|record| existing_ids.contains(&record.id))
        {
            return Err(PlannerContractError::new(
                "promotions.witnesses.evidence_id",
                "collides with evidence already attached to the action",
            ));
        }
        let mut records = existing_evidence.records.clone();
        records.extend(witness_records.clone());
        records.sort_by(|left, right| left.id.cmp(&right.id));
        let promoted_evidence = RuleEvidence {
            truth: TruthStatus::Established,
            records,
        };
        set_operation_evidence(&mut promoted_operation, promoted_evidence.clone());
        let label = format!(
            "Promote {} from route observations",
            action_id(&requested.action)
        );
        rules.push(RefinementRule {
            id: requested.promotion_rule_id.clone(),
            label: label.clone(),
            operation: promoted_operation,
            evidence: RuleEvidence {
                truth: TruthStatus::Established,
                records: witness_records.clone(),
            },
        });
        rules.push(RefinementRule {
            id: requested.replacement_rule_id.clone(),
            label,
            operation: RefinementOperation::ReplaceRecord {
                target_id: action_id(&requested.action).into(),
                replacement_kind: ReplacementKind::Replace,
                replacement_rule_id: Some(requested.promotion_rule_id.clone()),
            },
            evidence: RuleEvidence {
                truth: TruthStatus::Established,
                records: witness_records.clone(),
            },
        });
        let mut route_witness_evidence_ids = witness_records
            .iter()
            .map(|record| record.id.clone())
            .collect::<Vec<_>>();
        route_witness_evidence_ids.sort();
        let mut observation_ids = requested
            .witnesses
            .iter()
            .map(|witness| witness.observation_id.clone())
            .collect::<Vec<_>>();
        observation_ids.sort();
        receipt_rows.push(WitnessPromotionReceiptRow {
            action: requested.action.clone(),
            promotion_rule_id: requested.promotion_rule_id.clone(),
            replacement_rule_id: requested.replacement_rule_id.clone(),
            preserved_evidence_ids: existing_ids.into_iter().collect(),
            route_witness_evidence_ids,
            observation_ids,
        });
    }
    rules.sort_by(|left, right| left.id.cmp(&right.id));
    let mut dependencies = catalog
        .refinement_stack
        .entries
        .iter()
        .map(|entry| PackDependency {
            pack_id: entry.pack_id.clone(),
            pack_sha256: entry.pack_sha256,
        })
        .collect::<Vec<_>>();
    dependencies.sort_by(|left, right| left.pack_id.cmp(&right.pack_id));
    let pack = RefinementPack {
        schema: REFINEMENT_PACK_SCHEMA.into(),
        manifest: RefinementPackManifest {
            id: request.pack.id.clone(),
            version: request.pack.version.clone(),
            author: request.pack.author.clone(),
            source: request.pack.source.clone(),
            scope: ContextScope {
                selectors: selectors.into_values().collect(),
            },
            precedence: request.pack.precedence,
            dependencies,
            conflicts: request.pack.conflicts.clone(),
        },
        rules,
    };
    pack.validate()?;
    let action_ids_before = action_ids(catalog);
    // Every generated addition retains its target action ID and is paired with
    // one exact replacement. The action census is therefore unchanged even
    // when the pack depends on source packs unavailable to this isolated call.
    let action_ids_after = action_ids_before.clone();
    if action_ids_before != action_ids_after {
        return Err(PlannerContractError::new(
            "witness_promotion",
            "promotion changed the action census instead of only its evidence",
        ));
    }
    let receipt = WitnessPromotionReceipt {
        schema: WITNESS_PROMOTION_RECEIPT_SCHEMA.into(),
        composed_catalog_sha256: catalog.digest()?,
        validation_report_sha256: validation_sha256,
        pack_id: pack.manifest.id.clone(),
        pack_sha256: pack.digest()?,
        action_ids_before,
        action_ids_after,
        promotions: receipt_rows,
    };
    receipt.validate()?;
    Ok((pack, receipt))
}

impl WitnessPromotionReceipt {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != WITNESS_PROMOTION_RECEIPT_SCHEMA
            || self.composed_catalog_sha256 == Digest::ZERO
            || self.validation_report_sha256 == Digest::ZERO
            || self.pack_sha256 == Digest::ZERO
            || self.promotions.is_empty()
            || self.action_ids_before != self.action_ids_after
        {
            return Err(PlannerContractError::new(
                "witness_promotion_receipt",
                "has invalid identities or changed the action census",
            ));
        }
        validate_stable_id("pack_id", &self.pack_id)?;
        validate_sorted_ids("action_ids_before", &self.action_ids_before, false)?;
        if self
            .promotions
            .windows(2)
            .any(|pair| pair[0].action >= pair[1].action)
        {
            return Err(PlannerContractError::new(
                "promotions",
                "must be unique and sorted by action",
            ));
        }
        let mut rule_ids = BTreeSet::new();
        let mut witness_evidence_ids = BTreeSet::new();
        let mut observation_ids = BTreeSet::new();
        for row in &self.promotions {
            validate_action(&row.action)?;
            if self
                .action_ids_before
                .binary_search_by(|candidate| candidate.as_str().cmp(action_id(&row.action)))
                .is_err()
            {
                return Err(PlannerContractError::new(
                    "promotions.action",
                    "is absent from the retained action census",
                ));
            }
            validate_stable_id("promotions.promotion_rule_id", &row.promotion_rule_id)?;
            validate_stable_id("promotions.replacement_rule_id", &row.replacement_rule_id)?;
            if !rule_ids.insert(row.promotion_rule_id.as_str())
                || !rule_ids.insert(row.replacement_rule_id.as_str())
            {
                return Err(PlannerContractError::new(
                    "promotions.rule_id",
                    "must be globally unique",
                ));
            }
            validate_sorted_ids(
                "promotions.preserved_evidence_ids",
                &row.preserved_evidence_ids,
                true,
            )?;
            validate_sorted_ids(
                "promotions.route_witness_evidence_ids",
                &row.route_witness_evidence_ids,
                false,
            )?;
            validate_sorted_ids("promotions.observation_ids", &row.observation_ids, false)?;
            if row.route_witness_evidence_ids.len() != row.observation_ids.len() {
                return Err(PlannerContractError::new(
                    "promotions",
                    "must retain one evidence ID per observation",
                ));
            }
            if row.route_witness_evidence_ids.iter().any(|id| {
                row.preserved_evidence_ids.binary_search(id).is_ok()
                    || !witness_evidence_ids.insert(id.as_str())
            }) || row
                .observation_ids
                .iter()
                .any(|id| !observation_ids.insert(id.as_str()))
            {
                return Err(PlannerContractError::new(
                    "promotions",
                    "preserved/witness evidence must be disjoint and witness IDs globally unique",
                ));
            }
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
        let receipt: Self = serde_json::from_slice(bytes)?;
        receipt.validate()?;
        require_canonical_json_bytes(
            "witness_promotion_receipt",
            bytes,
            &receipt.canonical_bytes()?,
        )?;
        Ok(receipt)
    }
}

fn require_promotable(
    row: &crate::route_observation_validation::ObservedEdgeValidation,
    action: &RouteActionRef,
) -> Result<(), PlannerContractError> {
    let authored_precondition_verified = row
        .authored_precondition
        .is_none_or(|truth| truth == EvaluatedTruth::True);
    if &row.action != action
        || row.action_precondition != EvaluatedTruth::True
        || !authored_precondition_verified
        || !matches!(
            row.postcondition_status,
            VerificationStatus::Verified | VerificationStatus::NotAuthored
        )
        || row.model_replay_status != VerificationStatus::Verified
        || row.snapshot_effects_status != VerificationStatus::Verified
        || row.component_preservation_status != VerificationStatus::Verified
        || !row.mismatched_component_ids.is_empty()
    {
        return Err(PlannerContractError::new(
            "promotions.witnesses.observation_id",
            format!(
                "validation {} does not fully witness the requested action",
                row.observation_id
            ),
        ));
    }
    Ok(())
}

fn promoted_operation(
    catalog: &ComposedPlannerCatalog,
    action: &RouteActionRef,
) -> Result<(ContextScope, RuleEvidence, RefinementOperation), PlannerContractError> {
    match action {
        RouteActionRef::Transition { transition_id } => catalog
            .mechanics
            .transitions
            .iter()
            .find(|record| record.id == *transition_id)
            .map(|record| {
                (
                    record.scope.clone(),
                    record.evidence.clone(),
                    RefinementOperation::AddTransition {
                        transition: record.clone(),
                    },
                )
            }),
        RouteActionRef::Technique { technique_id } => catalog
            .mechanics
            .techniques
            .iter()
            .find(|record| record.id == *technique_id)
            .map(|record| {
                (
                    record.scope.clone(),
                    record.evidence.clone(),
                    RefinementOperation::AddTechnique {
                        technique: record.clone(),
                    },
                )
            }),
        RouteActionRef::Resolver { resolver_id } => catalog
            .mechanics
            .resolvers
            .iter()
            .find(|record| record.id == *resolver_id)
            .map(|record| {
                (
                    record.scope.clone(),
                    record.evidence.clone(),
                    RefinementOperation::AddResolver {
                        resolver: record.clone(),
                    },
                )
            }),
        RouteActionRef::Writer { writer_id } => catalog
            .mechanics
            .writers
            .iter()
            .find(|record| record.id == *writer_id)
            .map(|record| {
                (
                    record.scope.clone(),
                    record.evidence.clone(),
                    RefinementOperation::AddWriter {
                        writer: record.clone(),
                    },
                )
            }),
        RouteActionRef::Microtrace { microtrace_id } => catalog
            .mechanics
            .microtraces
            .iter()
            .find(|record| record.id == *microtrace_id)
            .map(|record| {
                (
                    record.scope.clone(),
                    record.evidence.clone(),
                    RefinementOperation::AddMicrotrace {
                        microtrace: record.clone(),
                    },
                )
            }),
    }
    .ok_or_else(|| PlannerContractError::new("promotions.action", "is absent from the catalog"))
}

fn set_operation_evidence(operation: &mut RefinementOperation, evidence: RuleEvidence) {
    match operation {
        RefinementOperation::AddTransition { transition } => transition.evidence = evidence,
        RefinementOperation::AddTechnique { technique } => technique.evidence = evidence,
        RefinementOperation::AddResolver { resolver } => resolver.evidence = evidence,
        RefinementOperation::AddWriter { writer } => writer.evidence = evidence,
        RefinementOperation::AddMicrotrace { microtrace } => microtrace.evidence = evidence,
        _ => unreachable!("promoted operation is always an action record"),
    }
}

fn action_id(action: &RouteActionRef) -> &str {
    match action {
        RouteActionRef::Transition { transition_id } => transition_id,
        RouteActionRef::Technique { technique_id } => technique_id,
        RouteActionRef::Resolver { resolver_id } => resolver_id,
        RouteActionRef::Writer { writer_id } => writer_id,
        RouteActionRef::Microtrace { microtrace_id } => microtrace_id,
    }
}

fn action_ids(catalog: &ComposedPlannerCatalog) -> Vec<String> {
    let mut ids = catalog
        .mechanics
        .transitions
        .iter()
        .map(|record| record.id.clone())
        .chain(
            catalog
                .mechanics
                .techniques
                .iter()
                .map(|record| record.id.clone()),
        )
        .chain(
            catalog
                .mechanics
                .resolvers
                .iter()
                .map(|record| record.id.clone()),
        )
        .chain(
            catalog
                .mechanics
                .writers
                .iter()
                .map(|record| record.id.clone()),
        )
        .chain(
            catalog
                .mechanics
                .microtraces
                .iter()
                .map(|record| record.id.clone()),
        )
        .collect::<Vec<_>>();
    ids.sort();
    ids
}

fn validate_action(action: &RouteActionRef) -> Result<(), PlannerContractError> {
    validate_stable_id("action", action_id(action))
}

fn validate_version(version: &str) -> Result<(), PlannerContractError> {
    if version.is_empty()
        || version.len() > 32
        || !version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
    {
        return Err(PlannerContractError::new(
            "pack.version",
            "must be a short ASCII version",
        ));
    }
    Ok(())
}

fn validate_sorted_ids(
    field: &str,
    values: &[String],
    allow_empty: bool,
) -> Result<(), PlannerContractError> {
    if (!allow_empty && values.is_empty())
        || values.len() > 65_536
        || values.windows(2).any(|pair| pair[0] >= pair[1])
    {
        return Err(PlannerContractError::new(
            field,
            "must be bounded, unique, and sorted",
        ));
    }
    for value in values {
        validate_stable_id(field, value)?;
    }
    Ok(())
}
