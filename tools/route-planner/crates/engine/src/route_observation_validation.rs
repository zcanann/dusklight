//! Snapshot verification for digest-bound planned-edge observations.

use crate::artifact::Digest;
use crate::evaluation::{EvaluatedTruth, EvidencePolicy, PredicateEvaluator};
use crate::execution::PlannerExecutionState;
use crate::identity::EquivalenceSet;
use crate::logic::PredicateExpression;
use crate::refinement::ComposedPlannerCatalog;
use crate::route_book::{ReferenceStep, RouteActionRef, RouteBook};
use crate::route_observation::RouteObservationMatchReport;
use crate::snapshot::StateSnapshot;
use crate::state::{ExecutionEnvironment, StateComponent};
use crate::transition::StateOperation;
use crate::{
    PlannerContractError, canonical_json, require_canonical_json_bytes, validate_stable_id,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const ROUTE_OBSERVATION_VALIDATION_REPORT_SCHEMA: &str =
    "dusklight.route-planner.route-observation-validation-report/v1";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationStatus {
    Verified,
    Refuted,
    Unknown,
    NotAuthored,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentDisposition {
    Absent,
    Preserved,
    Changed,
    Added,
    Removed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentSnapshotCheck {
    pub component_id: String,
    pub modeled_disposition: ComponentDisposition,
    pub observed_disposition: ComponentDisposition,
    pub modeled_state_sha256: Option<Digest>,
    pub observed_state_sha256: Option<Digest>,
    pub matches_model: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservedEdgeValidation {
    pub observation_id: String,
    pub step_id: String,
    pub action: RouteActionRef,
    pub before_snapshot_sha256: Digest,
    pub after_snapshot_sha256: Digest,
    pub action_precondition: EvaluatedTruth,
    pub authored_precondition: Option<EvaluatedTruth>,
    pub intrinsic_postcondition: Option<EvaluatedTruth>,
    pub authored_postcondition: Option<EvaluatedTruth>,
    pub postcondition_status: VerificationStatus,
    pub model_replay_status: VerificationStatus,
    pub model_replay_error: Option<String>,
    pub modeled_snapshot_sha256: Option<Digest>,
    pub snapshot_effects_status: VerificationStatus,
    pub component_preservation_status: VerificationStatus,
    pub preserved_component_ids: Vec<String>,
    pub unexpected_component_change_ids: Vec<String>,
    pub mismatched_component_ids: Vec<String>,
    pub component_checks: Vec<ComponentSnapshotCheck>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RouteObservationValidationReport {
    pub schema: String,
    pub composed_catalog_sha256: Digest,
    pub route_book_id: String,
    pub route_book_sha256: Digest,
    pub observation_match_report_sha256: Digest,
    pub equivalence_set_sha256: Vec<Digest>,
    pub evidence_policy: EvidencePolicy,
    pub validations: Vec<ObservedEdgeValidation>,
}

impl RouteObservationValidationReport {
    pub fn build(
        catalog: &ComposedPlannerCatalog,
        route_book: &RouteBook,
        matches: &RouteObservationMatchReport,
        snapshots: &[StateSnapshot],
        equivalence_sets: &[EquivalenceSet],
        evidence_policy: EvidencePolicy,
    ) -> Result<Self, PlannerContractError> {
        catalog.validate()?;
        route_book.validate_against_composed(catalog)?;
        matches.validate()?;
        if matches.composed_catalog_sha256 != catalog.digest()?
            || matches.route_book_id != route_book.manifest.id
            || matches.route_book_sha256 != route_book.digest()?
        {
            return Err(PlannerContractError::new(
                "route_observation_validation",
                "catalog or route-book identity differs from the match report",
            ));
        }
        let mut equivalence_set_sha256 = equivalence_sets
            .iter()
            .map(|set| {
                set.validate()?;
                set.digest()
            })
            .collect::<Result<Vec<_>, PlannerContractError>>()?;
        equivalence_set_sha256.sort();
        if equivalence_set_sha256
            .windows(2)
            .any(|pair| pair[0] == pair[1])
        {
            return Err(PlannerContractError::new(
                "equivalence_sets",
                "contains duplicate content",
            ));
        }
        let mut by_digest = BTreeMap::new();
        for snapshot in snapshots {
            snapshot.validate()?;
            let digest = snapshot.digest()?;
            if by_digest.insert(digest, snapshot).is_some() {
                return Err(PlannerContractError::new(
                    "snapshots",
                    "contains a duplicate snapshot digest",
                ));
            }
        }
        if by_digest.keys().copied().collect::<Vec<_>>() != matches.snapshots {
            return Err(PlannerContractError::new(
                "snapshots",
                "must exactly reproduce the match report snapshot census",
            ));
        }
        let steps = route_book
            .steps
            .iter()
            .map(|step| (step.id.as_str(), step))
            .collect::<BTreeMap<_, _>>();
        let mut validations = Vec::new();
        for matched_step in &matches.steps {
            let step = steps[matched_step.step_id.as_str()];
            for observation in &matched_step.observations {
                let before = by_digest[&observation.before_snapshot_sha256];
                let after = by_digest[&observation.after_snapshot_sha256];
                validations.push(validate_observation(
                    catalog,
                    step,
                    &observation.id,
                    before,
                    after,
                    equivalence_sets,
                    evidence_policy,
                )?);
            }
        }
        validations.sort_by(|left, right| left.observation_id.cmp(&right.observation_id));
        let report = Self {
            schema: ROUTE_OBSERVATION_VALIDATION_REPORT_SCHEMA.into(),
            composed_catalog_sha256: catalog.digest()?,
            route_book_id: route_book.manifest.id.clone(),
            route_book_sha256: route_book.digest()?,
            observation_match_report_sha256: matches.digest()?,
            equivalence_set_sha256,
            evidence_policy,
            validations,
        };
        report.validate()?;
        Ok(report)
    }

    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != ROUTE_OBSERVATION_VALIDATION_REPORT_SCHEMA
            || self.composed_catalog_sha256 == Digest::ZERO
            || self.route_book_sha256 == Digest::ZERO
            || self.observation_match_report_sha256 == Digest::ZERO
            || self.validations.is_empty()
            || self
                .equivalence_set_sha256
                .iter()
                .any(|digest| *digest == Digest::ZERO)
            || self
                .equivalence_set_sha256
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
        {
            return Err(PlannerContractError::new(
                "route_observation_validation_report",
                "has an invalid schema, identity, or validation census",
            ));
        }
        validate_stable_id("route_book_id", &self.route_book_id)?;
        if self
            .validations
            .windows(2)
            .any(|pair| pair[0].observation_id >= pair[1].observation_id)
        {
            return Err(PlannerContractError::new(
                "validations",
                "must be unique and sorted by observation ID",
            ));
        }
        for validation in &self.validations {
            validate_validation(validation)?;
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
        require_canonical_json_bytes(
            "route_observation_validation_report",
            bytes,
            &report.canonical_bytes()?,
        )?;
        Ok(report)
    }
}

fn validate_observation(
    catalog: &ComposedPlannerCatalog,
    step: &ReferenceStep,
    observation_id: &str,
    before: &StateSnapshot,
    after: &StateSnapshot,
    equivalence_sets: &[EquivalenceSet],
    evidence_policy: EvidencePolicy,
) -> Result<ObservedEdgeValidation, PlannerContractError> {
    let (action_precondition_expression, operations, intrinsic_postcondition_expression) =
        action_contract(&catalog.mechanics, &step.action)?;
    let before_evaluator = PredicateEvaluator::new(
        before,
        &catalog.facts,
        equivalence_sets,
        &BTreeMap::new(),
        evidence_policy,
    )?;
    let after_evaluator = PredicateEvaluator::new(
        after,
        &catalog.facts,
        equivalence_sets,
        &BTreeMap::new(),
        evidence_policy,
    )?;
    let action_precondition = before_evaluator.evaluate(action_precondition_expression);
    let authored_precondition = step
        .precondition
        .as_ref()
        .map(|predicate| before_evaluator.evaluate(predicate));
    let intrinsic_postcondition =
        intrinsic_postcondition_expression.map(|predicate| after_evaluator.evaluate(predicate));
    let authored_postcondition = step
        .postcondition
        .as_ref()
        .map(|predicate| after_evaluator.evaluate(predicate));
    let postcondition_status = predicate_status(
        intrinsic_postcondition
            .into_iter()
            .chain(authored_postcondition),
    );

    let mut result = ObservedEdgeValidation {
        observation_id: observation_id.into(),
        step_id: step.id.clone(),
        action: step.action.clone(),
        before_snapshot_sha256: before.digest()?,
        after_snapshot_sha256: after.digest()?,
        action_precondition,
        authored_precondition,
        intrinsic_postcondition,
        authored_postcondition,
        postcondition_status,
        model_replay_status: VerificationStatus::Unavailable,
        model_replay_error: None,
        modeled_snapshot_sha256: None,
        snapshot_effects_status: VerificationStatus::Unavailable,
        component_preservation_status: VerificationStatus::Unavailable,
        preserved_component_ids: Vec::new(),
        unexpected_component_change_ids: Vec::new(),
        mismatched_component_ids: Vec::new(),
        component_checks: Vec::new(),
    };
    let mut modeled = PlannerExecutionState::new(before.clone())?;
    match modeled.apply_operations("witness.snapshot-validation", &after.id, operations) {
        Ok(_) => {
            result.model_replay_status = VerificationStatus::Verified;
            result.modeled_snapshot_sha256 = Some(modeled.snapshot.digest()?);
            result.component_checks = compare_components(before, &modeled.snapshot, after)?;
            result.preserved_component_ids = result
                .component_checks
                .iter()
                .filter(|check| {
                    check.modeled_disposition == ComponentDisposition::Preserved
                        && check.matches_model
                })
                .map(|check| check.component_id.clone())
                .collect();
            result.unexpected_component_change_ids = result
                .component_checks
                .iter()
                .filter(|check| {
                    matches!(
                        check.modeled_disposition,
                        ComponentDisposition::Preserved | ComponentDisposition::Absent
                    ) && !check.matches_model
                })
                .map(|check| check.component_id.clone())
                .collect();
            result.mismatched_component_ids = result
                .component_checks
                .iter()
                .filter(|check| !check.matches_model)
                .map(|check| check.component_id.clone())
                .collect();
            result.component_preservation_status =
                if result.unexpected_component_change_ids.is_empty() {
                    VerificationStatus::Verified
                } else {
                    VerificationStatus::Refuted
                };
            result.snapshot_effects_status =
                if normalized_environment(&modeled.snapshot.environment)
                    == normalized_environment(&after.environment)
                {
                    VerificationStatus::Verified
                } else {
                    VerificationStatus::Refuted
                };
        }
        Err(error) => {
            result.model_replay_error = Some(error.to_string());
        }
    }
    Ok(result)
}

fn action_contract<'a>(
    mechanics: &'a crate::transition::MechanicsCatalog,
    action: &RouteActionRef,
) -> Result<
    (
        &'a PredicateExpression,
        &'a [StateOperation],
        Option<&'a PredicateExpression>,
    ),
    PlannerContractError,
> {
    match action {
        RouteActionRef::Transition { transition_id } => mechanics
            .transitions
            .iter()
            .find(|record| record.id == *transition_id)
            .map(|record| {
                (
                    &record.activation.hard_guards,
                    record.activation.effects.as_slice(),
                    None,
                )
            })
            .ok_or_else(|| PlannerContractError::new("action.transition_id", "is unknown")),
        RouteActionRef::Technique { technique_id } => mechanics
            .techniques
            .iter()
            .find(|record| record.id == *technique_id)
            .map(|record| (&record.prerequisites, record.operations.as_slice(), None))
            .ok_or_else(|| PlannerContractError::new("action.technique_id", "is unknown")),
        RouteActionRef::Resolver { resolver_id } => mechanics
            .resolvers
            .iter()
            .find(|record| record.id == *resolver_id)
            .map(|record| (&record.applicable_when, record.operations.as_slice(), None))
            .ok_or_else(|| PlannerContractError::new("action.resolver_id", "is unknown")),
        RouteActionRef::Writer { writer_id } => mechanics
            .writers
            .iter()
            .find(|record| record.id == *writer_id)
            .map(|record| {
                (
                    &record.activation,
                    std::slice::from_ref(&record.operation),
                    None,
                )
            })
            .ok_or_else(|| PlannerContractError::new("action.writer_id", "is unknown")),
        RouteActionRef::Microtrace { microtrace_id } => mechanics
            .microtraces
            .iter()
            .find(|record| record.id == *microtrace_id)
            .map(|record| {
                (
                    &record.precondition,
                    record.operations.as_slice(),
                    Some(&record.postcondition),
                )
            })
            .ok_or_else(|| PlannerContractError::new("action.microtrace_id", "is unknown")),
    }
}

fn predicate_status(values: impl Iterator<Item = EvaluatedTruth>) -> VerificationStatus {
    let values = values.collect::<Vec<_>>();
    if values.is_empty() {
        VerificationStatus::NotAuthored
    } else if values.contains(&EvaluatedTruth::False) {
        VerificationStatus::Refuted
    } else if values.contains(&EvaluatedTruth::Unknown) {
        VerificationStatus::Unknown
    } else {
        VerificationStatus::Verified
    }
}

fn compare_components(
    before: &StateSnapshot,
    modeled: &StateSnapshot,
    observed: &StateSnapshot,
) -> Result<Vec<ComponentSnapshotCheck>, PlannerContractError> {
    let before = component_map(before);
    let modeled = component_map(modeled);
    let observed = component_map(observed);
    let ids = before
        .keys()
        .chain(modeled.keys())
        .chain(observed.keys())
        .copied()
        .collect::<BTreeSet<_>>();
    ids.into_iter()
        .map(|id| {
            let before_state = before.get(id).copied();
            let modeled_state = modeled.get(id).copied();
            let observed_state = observed.get(id).copied();
            let modeled_state_sha256 = modeled_state.map(component_state_digest).transpose()?;
            let observed_state_sha256 = observed_state.map(component_state_digest).transpose()?;
            Ok(ComponentSnapshotCheck {
                component_id: id.into(),
                modeled_disposition: disposition(before_state, modeled_state),
                observed_disposition: disposition(before_state, observed_state),
                modeled_state_sha256,
                observed_state_sha256,
                matches_model: modeled_state_sha256 == observed_state_sha256,
            })
        })
        .collect()
}

fn component_map(snapshot: &StateSnapshot) -> BTreeMap<&str, &StateComponent> {
    snapshot
        .environment
        .components
        .iter()
        .map(|component| (component.id.as_str(), component))
        .collect()
}

fn disposition(
    before: Option<&StateComponent>,
    after: Option<&StateComponent>,
) -> ComponentDisposition {
    match (before, after) {
        (None, Some(_)) => ComponentDisposition::Added,
        (Some(_), None) => ComponentDisposition::Removed,
        (Some(before), Some(after)) if same_component_state(before, after) => {
            ComponentDisposition::Preserved
        }
        (Some(_), Some(_)) => ComponentDisposition::Changed,
        (None, None) => ComponentDisposition::Absent,
    }
}

#[derive(Serialize)]
struct ComponentStateIdentity<'a> {
    component_kind: crate::state::ComponentKind,
    payload: &'a crate::state::ComponentPayload,
    binding: &'a crate::state::ComponentBinding,
    lifetime: crate::state::SemanticLifetime,
    serialization_owner: &'a crate::state::SerializationOwner,
}

fn same_component_state(left: &StateComponent, right: &StateComponent) -> bool {
    left.component_kind == right.component_kind
        && left.payload == right.payload
        && left.binding == right.binding
        && left.lifetime == right.lifetime
        && left.serialization_owner == right.serialization_owner
}

fn component_state_digest(component: &StateComponent) -> Result<Digest, PlannerContractError> {
    let identity = ComponentStateIdentity {
        component_kind: component.component_kind.clone(),
        payload: &component.payload,
        binding: &component.binding,
        lifetime: component.lifetime,
        serialization_owner: &component.serialization_owner,
    };
    Ok(Digest(Sha256::digest(canonical_json(&identity)?).into()))
}

fn normalized_environment(environment: &ExecutionEnvironment) -> ExecutionEnvironment {
    let mut normalized = environment.clone();
    for component in &mut normalized.components {
        component.provenance.clear();
    }
    normalized
}

fn validate_validation(validation: &ObservedEdgeValidation) -> Result<(), PlannerContractError> {
    validate_stable_id("validations.observation_id", &validation.observation_id)?;
    validate_stable_id("validations.step_id", &validation.step_id)?;
    validate_action_ref(&validation.action)?;
    if validation.postcondition_status
        != predicate_status(
            validation
                .intrinsic_postcondition
                .into_iter()
                .chain(validation.authored_postcondition),
        )
    {
        return Err(PlannerContractError::new(
            "validations.postcondition_status",
            "does not agree with the evaluated postconditions",
        ));
    }
    if validation.before_snapshot_sha256 == Digest::ZERO
        || validation.after_snapshot_sha256 == Digest::ZERO
        || validation.before_snapshot_sha256 == validation.after_snapshot_sha256
        || validation
            .component_checks
            .windows(2)
            .any(|pair| pair[0].component_id >= pair[1].component_id)
        || !sorted_unique(&validation.preserved_component_ids)
        || !sorted_unique(&validation.unexpected_component_change_ids)
        || !sorted_unique(&validation.mismatched_component_ids)
    {
        return Err(PlannerContractError::new(
            "validations",
            "has invalid snapshot identities or component censuses",
        ));
    }
    for check in &validation.component_checks {
        validate_stable_id("component_checks.component_id", &check.component_id)?;
        let modeled_shape_valid =
            disposition_digest_valid(check.modeled_disposition, check.modeled_state_sha256);
        let observed_shape_valid =
            disposition_digest_valid(check.observed_disposition, check.observed_state_sha256);
        if !modeled_shape_valid
            || !observed_shape_valid
            || check.matches_model != (check.modeled_state_sha256 == check.observed_state_sha256)
        {
            return Err(PlannerContractError::new(
                "component_checks",
                "disposition or match state does not agree with the semantic component digests",
            ));
        }
    }
    match validation.model_replay_status {
        VerificationStatus::Verified
            if validation.model_replay_error.is_none()
                && validation
                    .modeled_snapshot_sha256
                    .is_some_and(|digest| digest != Digest::ZERO) =>
        {
            let preserved = validation
                .component_checks
                .iter()
                .filter(|check| {
                    check.modeled_disposition == ComponentDisposition::Preserved
                        && check.matches_model
                })
                .map(|check| check.component_id.clone())
                .collect::<Vec<_>>();
            let unexpected = validation
                .component_checks
                .iter()
                .filter(|check| {
                    matches!(
                        check.modeled_disposition,
                        ComponentDisposition::Preserved | ComponentDisposition::Absent
                    ) && !check.matches_model
                })
                .map(|check| check.component_id.clone())
                .collect::<Vec<_>>();
            let mismatched = validation
                .component_checks
                .iter()
                .filter(|check| !check.matches_model)
                .map(|check| check.component_id.clone())
                .collect::<Vec<_>>();
            let preservation_status = if unexpected.is_empty() {
                VerificationStatus::Verified
            } else {
                VerificationStatus::Refuted
            };
            if validation.preserved_component_ids != preserved
                || validation.unexpected_component_change_ids != unexpected
                || validation.mismatched_component_ids != mismatched
                || validation.component_preservation_status != preservation_status
                || !matches!(
                    validation.snapshot_effects_status,
                    VerificationStatus::Verified | VerificationStatus::Refuted
                )
            {
                return Err(PlannerContractError::new(
                    "validations.component_checks",
                    "derived component census or snapshot status drifted",
                ));
            }
        }
        VerificationStatus::Unavailable
            if validation.model_replay_error.is_some()
                && validation.modeled_snapshot_sha256.is_none()
                && validation.component_checks.is_empty()
                && validation.snapshot_effects_status == VerificationStatus::Unavailable
                && validation.component_preservation_status == VerificationStatus::Unavailable => {}
        _ => {
            return Err(PlannerContractError::new(
                "validations.model_replay_status",
                "does not agree with the replay result",
            ));
        }
    }
    Ok(())
}

fn disposition_digest_valid(disposition: ComponentDisposition, digest: Option<Digest>) -> bool {
    match (disposition, digest) {
        (ComponentDisposition::Absent | ComponentDisposition::Removed, None) => true,
        (
            ComponentDisposition::Preserved
            | ComponentDisposition::Changed
            | ComponentDisposition::Added,
            Some(digest),
        ) => digest != Digest::ZERO,
        _ => false,
    }
}

fn validate_action_ref(action: &RouteActionRef) -> Result<(), PlannerContractError> {
    let (field, id) = match action {
        RouteActionRef::Transition { transition_id } => ("action.transition_id", transition_id),
        RouteActionRef::Technique { technique_id } => ("action.technique_id", technique_id),
        RouteActionRef::Resolver { resolver_id } => ("action.resolver_id", resolver_id),
        RouteActionRef::Writer { writer_id } => ("action.writer_id", writer_id),
        RouteActionRef::Microtrace { microtrace_id } => ("action.microtrace_id", microtrace_id),
    };
    validate_stable_id(field, id)
}

fn sorted_unique(values: &[String]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
}
