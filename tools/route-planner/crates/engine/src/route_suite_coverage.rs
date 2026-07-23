//! Explicit fact and obligation coverage for named route-suite classes.

use crate::PlannerContractError;
use crate::artifact::Digest;
use crate::refinement::ComposedPlannerCatalog;
use crate::route_book::{RouteActionRef, RouteBook, RouteDirectiveKind};
use crate::route_evidence_coverage::{RouteCoverageIdentity, RouteEvidenceCoverageReport};
use crate::transition::{MechanicsCatalog, PathConstraint};
use crate::{canonical_json, require_canonical_json_bytes};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const ROUTE_SUITE_COVERAGE_SCHEMA: &str = "dusklight.route-planner.route-suite-coverage/v1";

const ALL_SUITES: [RouteSuiteKind; 4] = [
    RouteSuiteKind::GlitchlessStory,
    RouteSuiteKind::HundredPercent,
    RouteSuiteKind::AnyPercent,
    RouteSuiteKind::Hypothetical,
];

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteSuiteKind {
    GlitchlessStory,
    HundredPercent,
    AnyPercent,
    Hypothetical,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RouteSuiteCoverageReport {
    pub schema: String,
    pub composed_catalog_sha256: Digest,
    pub fact_catalog_sha256: Digest,
    pub mechanics_catalog_sha256: Digest,
    pub suites: Vec<RouteSuiteCoverage>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RouteSuiteCoverage {
    pub suite: RouteSuiteKind,
    pub reported: bool,
    pub routes: Vec<RouteCoverageIdentity>,
    pub exercised_fact_ids: Vec<String>,
    pub exercised_obligation_ids: Vec<String>,
}

impl RouteSuiteCoverageReport {
    pub fn build(
        catalog: &ComposedPlannerCatalog,
        categorized_routes: &[(RouteSuiteKind, RouteBook)],
    ) -> Result<Self, PlannerContractError> {
        catalog.validate()?;
        if categorized_routes.is_empty() {
            return Err(PlannerContractError::new(
                "route_suite_coverage.routes",
                "requires at least one categorized route book",
            ));
        }
        let mut grouped = BTreeMap::<RouteSuiteKind, Vec<RouteBook>>::new();
        for (suite, route_book) in categorized_routes {
            grouped.entry(*suite).or_default().push(route_book.clone());
        }
        let mut suites = Vec::with_capacity(ALL_SUITES.len());
        for suite in ALL_SUITES {
            let books = grouped.remove(&suite).unwrap_or_default();
            if books.is_empty() {
                suites.push(RouteSuiteCoverage {
                    suite,
                    reported: false,
                    routes: Vec::new(),
                    exercised_fact_ids: Vec::new(),
                    exercised_obligation_ids: Vec::new(),
                });
                continue;
            }
            let evidence = RouteEvidenceCoverageReport::build(catalog, &books, 1)?;
            let mut obligations = BTreeSet::new();
            for book in &books {
                collect_route_obligations(book, &catalog.mechanics, &mut obligations);
            }
            suites.push(RouteSuiteCoverage {
                suite,
                reported: true,
                routes: evidence.routes,
                exercised_fact_ids: evidence
                    .facts
                    .into_iter()
                    .map(|fact| fact.fact_id)
                    .collect(),
                exercised_obligation_ids: obligations.into_iter().collect(),
            });
        }
        let report = Self {
            schema: ROUTE_SUITE_COVERAGE_SCHEMA.into(),
            composed_catalog_sha256: catalog.digest()?,
            fact_catalog_sha256: catalog.facts.digest()?,
            mechanics_catalog_sha256: catalog.mechanics.digest()?,
            suites,
        };
        report.validate()?;
        Ok(report)
    }

    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != ROUTE_SUITE_COVERAGE_SCHEMA
            || self.composed_catalog_sha256 == Digest::ZERO
            || self.fact_catalog_sha256 == Digest::ZERO
            || self.mechanics_catalog_sha256 == Digest::ZERO
            || self.suites.len() != ALL_SUITES.len()
        {
            return Err(PlannerContractError::new(
                "route_suite_coverage",
                "has invalid schema, catalog identity, or suite census",
            ));
        }
        for (expected, suite) in ALL_SUITES.iter().zip(&self.suites) {
            if expected != &suite.suite
                || suite.reported != !suite.routes.is_empty()
                || suite.routes.windows(2).any(|pair| pair[0] >= pair[1])
                || !sorted_unique(&suite.exercised_fact_ids)
                || !sorted_unique(&suite.exercised_obligation_ids)
                || (!suite.reported
                    && (!suite.exercised_fact_ids.is_empty()
                        || !suite.exercised_obligation_ids.is_empty()))
            {
                return Err(PlannerContractError::new(
                    "route_suite_coverage.suites",
                    "suite order, reported state, route identity, facts, or obligations drifted",
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
        let report: Self = serde_json::from_slice(bytes)?;
        report.validate()?;
        require_canonical_json_bytes("route_suite_coverage", bytes, &report.canonical_bytes()?)?;
        Ok(report)
    }
}

fn sorted_unique(values: &[String]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
}

fn collect_route_obligations(
    book: &RouteBook,
    mechanics: &MechanicsCatalog,
    obligations: &mut BTreeSet<String>,
) {
    for constraint in &book.constraints {
        match &constraint.constraint {
            PathConstraint::RequireTechnique { technique_id }
            | PathConstraint::ForbidTechnique { technique_id } => collect_action_obligations(
                &RouteActionRef::Technique {
                    technique_id: technique_id.clone(),
                },
                mechanics,
                obligations,
            ),
            PathConstraint::RequireTransition { transition_id }
            | PathConstraint::ForbidTransition { transition_id } => collect_action_obligations(
                &RouteActionRef::Transition {
                    transition_id: transition_id.clone(),
                },
                mechanics,
                obligations,
            ),
            PathConstraint::RequirePredicate { .. }
            | PathConstraint::ForbidPredicate { .. }
            | PathConstraint::MaintainPredicate { .. }
            | PathConstraint::EvidenceAtLeast { .. }
            | PathConstraint::CostAtMost { .. } => {}
        }
    }
    for directive in &book.directives {
        match &directive.directive {
            RouteDirectiveKind::PinAction { action }
            | RouteDirectiveKind::BanAction { action }
            | RouteDirectiveKind::PreferAction { action, .. } => {
                collect_action_obligations(action, mechanics, obligations)
            }
            RouteDirectiveKind::PinMethod { .. }
            | RouteDirectiveKind::BanMethod { .. }
            | RouteDirectiveKind::PreferMethod { .. } => {}
        }
    }
    for step in &book.steps {
        collect_action_obligations(&step.action, mechanics, obligations);
    }
}

fn collect_action_obligations(
    action: &RouteActionRef,
    mechanics: &MechanicsCatalog,
    obligations: &mut BTreeSet<String>,
) {
    match action {
        RouteActionRef::Transition { transition_id } => {
            if let Some(transition) = mechanics
                .transitions
                .iter()
                .find(|record| record.id == *transition_id)
            {
                obligations.extend(
                    transition
                        .activation
                        .physical_obligation_ids
                        .iter()
                        .cloned(),
                );
                for obstruction in mechanics
                    .obstructions
                    .iter()
                    .filter(|record| record.blocked_action_id == *transition_id)
                {
                    obligations.extend(obstruction.obligation_ids.iter().cloned());
                }
            }
        }
        RouteActionRef::Technique { technique_id } => {
            if let Some(technique) = mechanics
                .techniques
                .iter()
                .find(|record| record.id == *technique_id)
            {
                obligations.extend(technique.discharged_obligation_ids.iter().cloned());
                obligations.extend(technique.introduced_obligation_ids.iter().cloned());
            }
        }
        RouteActionRef::Resolver { resolver_id } => {
            if let Some(resolver) = mechanics
                .resolvers
                .iter()
                .find(|record| record.id == *resolver_id)
                && let Some(obstruction) = mechanics
                    .obstructions
                    .iter()
                    .find(|record| record.id == resolver.obstruction_id)
            {
                obligations.extend(obstruction.obligation_ids.iter().cloned());
            }
        }
        RouteActionRef::Writer { .. } | RouteActionRef::Microtrace { .. } => {}
    }
}
