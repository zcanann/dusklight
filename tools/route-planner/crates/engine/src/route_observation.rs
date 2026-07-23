//! Digest-bound matches between planned route edges and observed trace windows.

use crate::artifact::Digest;
use crate::refinement::ComposedPlannerCatalog;
use crate::route_book::{RouteActionRef, RouteBook};
use crate::snapshot::StateSnapshot;
use crate::{
    PlannerContractError, canonical_json, require_canonical_json_bytes, validate_stable_id,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const PLANNED_EDGE_OBSERVATION_MANIFEST_SCHEMA: &str =
    "dusklight.route-planner.planned-edge-observation-manifest/v1";
pub const ROUTE_OBSERVATION_MATCH_REPORT_SCHEMA: &str =
    "dusklight.route-planner.route-observation-match-report/v1";

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationArtifactKind {
    Trace,
    Tape,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationArtifact {
    pub id: String,
    pub kind: ObservationArtifactKind,
    pub sha256: Digest,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlannedEdgeObservation {
    pub id: String,
    pub step_id: String,
    pub trace_artifact_id: String,
    pub tape_artifact_id: Option<String>,
    pub before_snapshot_sha256: Digest,
    pub after_snapshot_sha256: Digest,
    pub start_tick: u64,
    pub end_tick: u64,
    pub start_tape_frame: Option<u64>,
    pub end_tape_frame: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlannedEdgeObservationManifest {
    pub schema: String,
    pub artifacts: Vec<ObservationArtifact>,
    pub observations: Vec<PlannedEdgeObservation>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MatchedRouteStep {
    pub step_id: String,
    pub action: RouteActionRef,
    pub observed: bool,
    pub observations: Vec<PlannedEdgeObservation>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RouteObservationMatchReport {
    pub schema: String,
    pub composed_catalog_sha256: Digest,
    pub route_book_id: String,
    pub route_book_sha256: Digest,
    pub artifacts: Vec<ObservationArtifact>,
    pub snapshots: Vec<Digest>,
    pub steps: Vec<MatchedRouteStep>,
}

impl PlannedEdgeObservationManifest {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != PLANNED_EDGE_OBSERVATION_MANIFEST_SCHEMA || self.observations.is_empty() {
            return Err(PlannerContractError::new(
                "planned_edge_observation_manifest",
                "has an invalid schema or no observations",
            ));
        }
        let mut artifact_ids = BTreeMap::new();
        for artifact in &self.artifacts {
            validate_stable_id("artifacts.id", &artifact.id)?;
            if artifact.sha256 == Digest::ZERO
                || artifact_ids
                    .insert(artifact.id.as_str(), artifact.kind)
                    .is_some()
            {
                return Err(PlannerContractError::new(
                    "artifacts",
                    "contains a zero digest or duplicate ID",
                ));
            }
        }
        if self.artifacts.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(PlannerContractError::new(
                "artifacts",
                "must be unique and sorted",
            ));
        }
        for observation in &self.observations {
            validate_stable_id("observations.id", &observation.id)?;
            validate_stable_id("observations.step_id", &observation.step_id)?;
            validate_stable_id(
                "observations.trace_artifact_id",
                &observation.trace_artifact_id,
            )?;
            if artifact_ids.get(observation.trace_artifact_id.as_str())
                != Some(&ObservationArtifactKind::Trace)
            {
                return Err(PlannerContractError::new(
                    "observations.trace_artifact_id",
                    "must reference a declared trace artifact",
                ));
            }
            if let Some(tape_id) = &observation.tape_artifact_id {
                validate_stable_id("observations.tape_artifact_id", tape_id)?;
                if artifact_ids.get(tape_id.as_str()) != Some(&ObservationArtifactKind::Tape) {
                    return Err(PlannerContractError::new(
                        "observations.tape_artifact_id",
                        "must reference a declared tape artifact",
                    ));
                }
            }
            if observation.before_snapshot_sha256 == Digest::ZERO
                || observation.after_snapshot_sha256 == Digest::ZERO
                || observation.before_snapshot_sha256 == observation.after_snapshot_sha256
                || observation.start_tick >= observation.end_tick
                || matches!(
                    (observation.start_tape_frame, observation.end_tape_frame),
                    (Some(start), Some(end)) if start >= end
                )
                || observation.start_tape_frame.is_some() != observation.end_tape_frame.is_some()
                || observation.tape_artifact_id.is_some() != observation.start_tape_frame.is_some()
            {
                return Err(PlannerContractError::new(
                    "observations",
                    "has invalid snapshot identities or trace/tape boundaries",
                ));
            }
        }
        if self.observations.windows(2).any(|pair| {
            (pair[0].step_id.as_str(), pair[0].id.as_str())
                >= (pair[1].step_id.as_str(), pair[1].id.as_str())
        }) {
            return Err(PlannerContractError::new(
                "observations",
                "must be unique and sorted by step ID and observation ID",
            ));
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PlannerContractError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, PlannerContractError> {
        let manifest: Self = serde_json::from_slice(bytes)?;
        manifest.validate()?;
        require_canonical_json_bytes(
            "planned_edge_observation_manifest",
            bytes,
            &manifest.canonical_bytes()?,
        )?;
        Ok(manifest)
    }
}

impl RouteObservationMatchReport {
    pub fn build(
        catalog: &ComposedPlannerCatalog,
        route_book: &RouteBook,
        manifest: &PlannedEdgeObservationManifest,
        snapshots: &[StateSnapshot],
    ) -> Result<Self, PlannerContractError> {
        catalog.validate()?;
        route_book.validate_against_composed(catalog)?;
        manifest.validate()?;
        if snapshots.is_empty() {
            return Err(PlannerContractError::new("snapshots", "must not be empty"));
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
        let known_steps = route_book
            .steps
            .iter()
            .map(|step| step.id.as_str())
            .collect::<BTreeSet<_>>();
        for observation in &manifest.observations {
            if !known_steps.contains(observation.step_id.as_str()) {
                return Err(PlannerContractError::new(
                    "observations.step_id",
                    format!("references unknown route step {}", observation.step_id),
                ));
            }
            let before = by_digest
                .get(&observation.before_snapshot_sha256)
                .ok_or_else(|| {
                    PlannerContractError::new(
                        "observations.before_snapshot_sha256",
                        "is not supplied",
                    )
                })?;
            let after = by_digest
                .get(&observation.after_snapshot_sha256)
                .ok_or_else(|| {
                    PlannerContractError::new(
                        "observations.after_snapshot_sha256",
                        "is not supplied",
                    )
                })?;
            if before.sequence >= after.sequence
                || before.environment.runtime_configuration.content_sha256
                    != after.environment.runtime_configuration.content_sha256
            {
                return Err(PlannerContractError::new(
                    "observations.snapshots",
                    "must be ordered observations of one exact content identity",
                ));
            }
        }
        let steps = route_book
            .steps
            .iter()
            .map(|step| {
                let observations = manifest
                    .observations
                    .iter()
                    .filter(|observation| observation.step_id == step.id)
                    .cloned()
                    .collect::<Vec<_>>();
                MatchedRouteStep {
                    step_id: step.id.clone(),
                    action: step.action.clone(),
                    observed: !observations.is_empty(),
                    observations,
                }
            })
            .collect();
        let report = Self {
            schema: ROUTE_OBSERVATION_MATCH_REPORT_SCHEMA.into(),
            composed_catalog_sha256: catalog.digest()?,
            route_book_id: route_book.manifest.id.clone(),
            route_book_sha256: route_book.digest()?,
            artifacts: manifest.artifacts.clone(),
            snapshots: by_digest.into_keys().collect(),
            steps,
        };
        report.validate()?;
        Ok(report)
    }

    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != ROUTE_OBSERVATION_MATCH_REPORT_SCHEMA
            || self.composed_catalog_sha256 == Digest::ZERO
            || self.route_book_sha256 == Digest::ZERO
            || self.steps.is_empty()
            || self.snapshots.is_empty()
        {
            return Err(PlannerContractError::new(
                "route_observation_match_report",
                "has an invalid schema or empty identity census",
            ));
        }
        validate_stable_id("route_book_id", &self.route_book_id)?;
        let manifest = PlannedEdgeObservationManifest {
            schema: PLANNED_EDGE_OBSERVATION_MANIFEST_SCHEMA.into(),
            artifacts: self.artifacts.clone(),
            observations: self
                .steps
                .iter()
                .flat_map(|step| step.observations.clone())
                .collect(),
        };
        manifest.validate()?;
        if self.snapshots.windows(2).any(|pair| pair[0] >= pair[1])
            || self.snapshots.iter().any(|digest| *digest == Digest::ZERO)
            || self
                .steps
                .windows(2)
                .any(|pair| pair[0].step_id >= pair[1].step_id)
        {
            return Err(PlannerContractError::new(
                "route_observation_match_report",
                "snapshot or step rows are not unique and sorted",
            ));
        }
        for step in &self.steps {
            validate_stable_id("steps.step_id", &step.step_id)?;
            validate_action_ref(&step.action)?;
            if step.observed != !step.observations.is_empty()
                || step
                    .observations
                    .iter()
                    .any(|observation| observation.step_id != step.step_id)
            {
                return Err(PlannerContractError::new(
                    "steps",
                    "observed state or observation ownership drifted",
                ));
            }
            for observation in &step.observations {
                if self
                    .snapshots
                    .binary_search(&observation.before_snapshot_sha256)
                    .is_err()
                    || self
                        .snapshots
                        .binary_search(&observation.after_snapshot_sha256)
                        .is_err()
                {
                    return Err(PlannerContractError::new(
                        "steps.observations",
                        "references a snapshot outside the retained census",
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

    pub fn digest(&self) -> Result<Digest, PlannerContractError> {
        Ok(Digest(Sha256::digest(self.canonical_bytes()?).into()))
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, PlannerContractError> {
        let report: Self = serde_json::from_slice(bytes)?;
        report.validate()?;
        require_canonical_json_bytes(
            "route_observation_match_report",
            bytes,
            &report.canonical_bytes()?,
        )?;
        Ok(report)
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
