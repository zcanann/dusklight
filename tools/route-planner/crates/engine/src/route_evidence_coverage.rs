//! Route-suite usage census for identifying heavily reused weak facts.

use crate::PlannerContractError;
use crate::artifact::Digest;
use crate::logic::{FactCatalog, PredicateExpression, TruthStatus};
use crate::refinement::ComposedPlannerCatalog;
use crate::route_book::{RouteActionRef, RouteBook, RouteDirectiveKind};
use crate::transition::{
    FeasibilityObligation, MechanicsCatalog, ObligationDetail, PathConstraint,
};
use crate::{canonical_json, require_canonical_json_bytes};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const ROUTE_EVIDENCE_COVERAGE_SCHEMA: &str =
    "dusklight.route-planner.route-evidence-coverage/v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RouteEvidenceCoverageReport {
    pub schema: String,
    pub composed_catalog_sha256: Digest,
    pub fact_catalog_sha256: Digest,
    pub mechanics_catalog_sha256: Digest,
    pub minimum_route_count: usize,
    pub routes: Vec<RouteCoverageIdentity>,
    pub facts: Vec<FactRouteUsage>,
    pub weak_high_usage_fact_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RouteCoverageIdentity {
    pub id: String,
    pub sha256: Digest,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FactDefinitionKind {
    Alias,
    Derived,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FactRouteUsage {
    pub fact_id: String,
    pub label: String,
    pub definition_kind: FactDefinitionKind,
    pub authored_truth: TruthStatus,
    pub evidence_record_ids: Vec<String>,
    pub route_book_ids: Vec<String>,
}

impl RouteEvidenceCoverageReport {
    pub fn build(
        catalog: &ComposedPlannerCatalog,
        route_books: &[RouteBook],
        minimum_route_count: usize,
    ) -> Result<Self, PlannerContractError> {
        catalog.validate()?;
        if route_books.is_empty() || minimum_route_count == 0 {
            return Err(PlannerContractError::new(
                "route_evidence_coverage",
                "requires at least one route and a nonzero usage threshold",
            ));
        }
        let mut books = route_books.iter().collect::<Vec<_>>();
        books.sort_by(|left, right| left.manifest.id.cmp(&right.manifest.id));
        if books
            .windows(2)
            .any(|pair| pair[0].manifest.id == pair[1].manifest.id)
        {
            return Err(PlannerContractError::new(
                "route_evidence_coverage.routes",
                "route-book IDs must be unique",
            ));
        }
        let mut usage = BTreeMap::<String, BTreeSet<String>>::new();
        let mut routes = Vec::with_capacity(books.len());
        for book in books {
            book.validate_against_composed(catalog)?;
            let mut facts = collect_route_fact_ids(book, &catalog.mechanics);
            expand_derived_fact_dependencies(&catalog.facts, &mut facts);
            for fact_id in facts {
                usage
                    .entry(fact_id)
                    .or_default()
                    .insert(book.manifest.id.clone());
            }
            routes.push(RouteCoverageIdentity {
                id: book.manifest.id.clone(),
                sha256: book.digest()?,
            });
        }
        let mut facts = Vec::with_capacity(usage.len());
        for (fact_id, route_ids) in usage {
            if let Some(alias) = catalog
                .facts
                .aliases
                .iter()
                .find(|record| record.id == fact_id)
            {
                facts.push(FactRouteUsage {
                    fact_id,
                    label: alias.label.clone(),
                    definition_kind: FactDefinitionKind::Alias,
                    authored_truth: alias.evidence.truth,
                    evidence_record_ids: evidence_ids(&alias.evidence.records),
                    route_book_ids: route_ids.into_iter().collect(),
                });
            } else if let Some(fact) = catalog
                .facts
                .derived_facts
                .iter()
                .find(|record| record.id == fact_id)
            {
                facts.push(FactRouteUsage {
                    fact_id,
                    label: fact.label.clone(),
                    definition_kind: FactDefinitionKind::Derived,
                    authored_truth: fact.evidence.truth,
                    evidence_record_ids: evidence_ids(&fact.evidence.records),
                    route_book_ids: route_ids.into_iter().collect(),
                });
            }
        }
        let weak_high_usage_fact_ids = facts
            .iter()
            .filter(|row| {
                row.authored_truth != TruthStatus::Established
                    && row.route_book_ids.len() >= minimum_route_count
            })
            .map(|row| row.fact_id.clone())
            .collect();
        let report = Self {
            schema: ROUTE_EVIDENCE_COVERAGE_SCHEMA.into(),
            composed_catalog_sha256: catalog.digest()?,
            fact_catalog_sha256: catalog.facts.digest()?,
            mechanics_catalog_sha256: catalog.mechanics.digest()?,
            minimum_route_count,
            routes,
            facts,
            weak_high_usage_fact_ids,
        };
        report.validate()?;
        Ok(report)
    }

    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != ROUTE_EVIDENCE_COVERAGE_SCHEMA
            || self.minimum_route_count == 0
            || self.routes.is_empty()
            || self.composed_catalog_sha256 == Digest::ZERO
            || self.fact_catalog_sha256 == Digest::ZERO
            || self.mechanics_catalog_sha256 == Digest::ZERO
        {
            return Err(PlannerContractError::new(
                "route_evidence_coverage",
                "has invalid schema, threshold, route set, or catalog identity",
            ));
        }
        if self.routes.windows(2).any(|pair| pair[0] >= pair[1])
            || self
                .facts
                .windows(2)
                .any(|pair| pair[0].fact_id >= pair[1].fact_id)
            || self
                .weak_high_usage_fact_ids
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
        {
            return Err(PlannerContractError::new(
                "route_evidence_coverage",
                "routes, facts, and weak-fact IDs must be unique and sorted",
            ));
        }
        let expected = self
            .facts
            .iter()
            .filter(|row| {
                row.authored_truth != TruthStatus::Established
                    && row.route_book_ids.len() >= self.minimum_route_count
            })
            .map(|row| row.fact_id.as_str())
            .collect::<Vec<_>>();
        if expected
            != self
                .weak_high_usage_fact_ids
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
        {
            return Err(PlannerContractError::new(
                "route_evidence_coverage.weak_high_usage_fact_ids",
                "does not match fact confidence and route usage counts",
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
        require_canonical_json_bytes("route_evidence_coverage", bytes, &report.canonical_bytes()?)?;
        Ok(report)
    }
}

fn evidence_ids(records: &[crate::logic::EvidenceRecord]) -> Vec<String> {
    records.iter().map(|record| record.id.clone()).collect()
}

fn collect_route_fact_ids(book: &RouteBook, mechanics: &MechanicsCatalog) -> BTreeSet<String> {
    let mut facts = BTreeSet::new();
    for goal_id in &book.goal_ids {
        if let Some(goal) = mechanics.goals.iter().find(|goal| goal.id == *goal_id) {
            collect_predicate(&goal.predicate, &mut facts);
        }
    }
    for constraint in &book.constraints {
        match &constraint.constraint {
            PathConstraint::RequirePredicate { predicate }
            | PathConstraint::ForbidPredicate { predicate }
            | PathConstraint::MaintainPredicate { predicate } => {
                collect_predicate(predicate, &mut facts)
            }
            PathConstraint::RequireTransition { transition_id }
            | PathConstraint::ForbidTransition { transition_id } => collect_action(
                &RouteActionRef::Transition {
                    transition_id: transition_id.clone(),
                },
                mechanics,
                &mut facts,
            ),
            PathConstraint::RequireTechnique { technique_id }
            | PathConstraint::ForbidTechnique { technique_id } => collect_action(
                &RouteActionRef::Technique {
                    technique_id: technique_id.clone(),
                },
                mechanics,
                &mut facts,
            ),
            PathConstraint::EvidenceAtLeast { .. } | PathConstraint::CostAtMost { .. } => {}
        }
    }
    for directive in &book.directives {
        match &directive.directive {
            RouteDirectiveKind::PinAction { action }
            | RouteDirectiveKind::BanAction { action }
            | RouteDirectiveKind::PreferAction { action, .. } => {
                collect_action(action, mechanics, &mut facts)
            }
            RouteDirectiveKind::PinMethod { .. }
            | RouteDirectiveKind::BanMethod { .. }
            | RouteDirectiveKind::PreferMethod { .. } => {}
        }
    }
    for step in &book.steps {
        if let Some(predicate) = &step.precondition {
            collect_predicate(predicate, &mut facts);
        }
        if let Some(predicate) = &step.postcondition {
            collect_predicate(predicate, &mut facts);
        }
        collect_action(&step.action, mechanics, &mut facts);
    }
    for region in &book.regions {
        if let Some(predicate) = &region.entry_predicate {
            collect_predicate(predicate, &mut facts);
        }
        collect_predicate(&region.outcome_predicate, &mut facts);
    }
    facts
}

fn collect_action(
    action: &RouteActionRef,
    mechanics: &MechanicsCatalog,
    facts: &mut BTreeSet<String>,
) {
    match action {
        RouteActionRef::Transition { transition_id } => {
            if let Some(transition) = mechanics
                .transitions
                .iter()
                .find(|record| record.id == *transition_id)
            {
                collect_predicate(&transition.activation.hard_guards, facts);
                collect_obligations(
                    &transition.activation.physical_obligation_ids,
                    mechanics,
                    facts,
                );
                for obstruction in mechanics
                    .obstructions
                    .iter()
                    .filter(|record| record.blocked_action_id == *transition_id)
                {
                    collect_predicate(&obstruction.active_when, facts);
                    collect_obligations(&obstruction.obligation_ids, mechanics, facts);
                }
            }
        }
        RouteActionRef::Technique { technique_id } => {
            if let Some(technique) = mechanics
                .techniques
                .iter()
                .find(|record| record.id == *technique_id)
            {
                collect_predicate(&technique.prerequisites, facts);
                collect_obligations(&technique.discharged_obligation_ids, mechanics, facts);
                collect_obligations(&technique.introduced_obligation_ids, mechanics, facts);
            }
        }
        RouteActionRef::Resolver { resolver_id } => {
            if let Some(resolver) = mechanics
                .resolvers
                .iter()
                .find(|record| record.id == *resolver_id)
            {
                collect_predicate(&resolver.applicable_when, facts);
                if let Some(obstruction) = mechanics
                    .obstructions
                    .iter()
                    .find(|record| record.id == resolver.obstruction_id)
                {
                    collect_predicate(&obstruction.active_when, facts);
                    collect_obligations(&obstruction.obligation_ids, mechanics, facts);
                }
            }
        }
        RouteActionRef::Writer { writer_id } => {
            if let Some(writer) = mechanics
                .writers
                .iter()
                .find(|record| record.id == *writer_id)
            {
                collect_predicate(&writer.activation, facts);
                for gate in mechanics
                    .gates
                    .iter()
                    .filter(|gate| gate.blocked_writer_ids.contains(writer_id))
                {
                    collect_predicate(&gate.active_when, facts);
                }
            }
        }
        RouteActionRef::Microtrace { microtrace_id } => {
            if let Some(microtrace) = mechanics
                .microtraces
                .iter()
                .find(|record| record.id == *microtrace_id)
            {
                collect_predicate(&microtrace.precondition, facts);
                collect_predicate(&microtrace.postcondition, facts);
            }
        }
    }
}

fn collect_obligations(
    obligation_ids: &[String],
    mechanics: &MechanicsCatalog,
    facts: &mut BTreeSet<String>,
) {
    for obligation_id in obligation_ids {
        if let Some(obligation) = mechanics
            .obligations
            .iter()
            .find(|record| record.id == *obligation_id)
        {
            collect_obligation(obligation, facts);
        }
    }
}

fn collect_obligation(obligation: &FeasibilityObligation, facts: &mut BTreeSet<String>) {
    match &obligation.detail {
        ObligationDetail::Predicate { predicate } => collect_predicate(predicate, facts),
        ObligationDetail::Interaction { pose_predicate, .. } => {
            collect_predicate(pose_predicate, facts)
        }
        ObligationDetail::CompoundInteraction { branches, .. } => {
            for branch in branches {
                collect_predicate(&branch.when, facts);
                collect_predicate(&branch.pose_predicate, facts);
            }
        }
        ObligationDetail::Temporal { precondition, .. } => collect_predicate(precondition, facts),
        ObligationDetail::Geometry { .. }
        | ObligationDetail::PlaneSide { .. }
        | ObligationDetail::Facing { .. }
        | ObligationDetail::Unresolved { .. } => {}
    }
}

fn collect_predicate(predicate: &PredicateExpression, facts: &mut BTreeSet<String>) {
    predicate.referenced_facts(facts);
}

fn expand_derived_fact_dependencies(catalog: &FactCatalog, facts: &mut BTreeSet<String>) {
    let mut pending = facts.iter().cloned().collect::<Vec<_>>();
    while let Some(fact_id) = pending.pop() {
        let Some(fact) = catalog
            .derived_facts
            .iter()
            .find(|record| record.id == fact_id)
        else {
            continue;
        };
        let mut dependencies = BTreeSet::new();
        fact.rule.referenced_facts(&mut dependencies);
        for dependency in dependencies {
            if facts.insert(dependency.clone()) {
                pending.push(dependency);
            }
        }
    }
}
