//! Composition of cheap native anomaly observations into corpus signatures.

use crate::comparison_oracle::{
    COMPARISON_EVIDENCE_SCHEMA_V1, ComparisonEvidence, ComparisonRunEvidence, ComparisonRunRole,
    SemanticEventSignature,
};
use crate::semantic_oracle::{RunAnomalyObservation, RunOutcomeEvidence, RunTermination};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as ShaDigest, Sha256};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

pub const ORACLE_COMPOSITION_SCHEMA_V1: &str = "dusklight-oracle-composition/v1";
pub const SEMANTIC_EVENT_CATALOG_SCHEMA_V1: &str = "dusklight-semantic-event-catalog/v1";

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OracleCompositionManifest {
    pub schema: String,
    pub catalog: SemanticEventCatalog,
    pub runs: Vec<OracleCompositionRun>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticEventCatalog {
    pub schema: String,
    #[serde(default)]
    pub signatures: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OracleCompositionRun {
    pub label: String,
    pub role: ComparisonRunRole,
    pub complete: bool,
    #[serde(default)]
    pub final_boundary_identity: Option<String>,
    pub run_outcome: RunOutcomeEvidence,
}

impl OracleCompositionManifest {
    pub fn compose(&self) -> Result<ComparisonEvidence, OraclePipelineError> {
        if self.schema != ORACLE_COMPOSITION_SCHEMA_V1 {
            return Err(OraclePipelineError::new(
                "unsupported oracle-composition schema",
            ));
        }
        self.catalog.validate()?;
        if self.runs.is_empty() || self.runs.len() > 4 {
            return Err(OraclePipelineError::new(
                "oracle composition requires 1..=4 runs",
            ));
        }
        let mut runs = Vec::with_capacity(self.runs.len());
        for run in &self.runs {
            run.run_outcome
                .validate()
                .map_err(|error| OraclePipelineError::new(error.to_string()))?;
            runs.push(compose_run(run)?);
        }
        let evidence = ComparisonEvidence {
            schema: COMPARISON_EVIDENCE_SCHEMA_V1.into(),
            catalog_identity: self.catalog.identity()?,
            known_event_signatures: self.catalog.canonical_signatures(),
            runs,
        };
        evidence
            .validate()
            .map_err(|error| OraclePipelineError::new(error.to_string()))?;
        Ok(evidence)
    }
}

impl SemanticEventCatalog {
    pub fn validate(&self) -> Result<(), OraclePipelineError> {
        if self.schema != SEMANTIC_EVENT_CATALOG_SCHEMA_V1 {
            return Err(OraclePipelineError::new(
                "unsupported semantic-event catalog schema",
            ));
        }
        if self.signatures.len() > 1_000_000
            || self
                .signatures
                .iter()
                .any(|signature| !valid_digest(signature))
        {
            return Err(OraclePipelineError::new(
                "invalid or oversized semantic-event catalog",
            ));
        }
        Ok(())
    }

    pub fn canonical_signatures(&self) -> Vec<String> {
        self.signatures
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    pub fn identity(&self) -> Result<String, OraclePipelineError> {
        self.validate()?;
        let canonical = SemanticEventCatalog {
            schema: SEMANTIC_EVENT_CATALOG_SCHEMA_V1.into(),
            signatures: self.canonical_signatures(),
        };
        let encoded = serde_json::to_vec(&canonical)
            .map_err(|error| OraclePipelineError::new(error.to_string()))?;
        Ok(hex_digest(&encoded))
    }
}

fn compose_run(run: &OracleCompositionRun) -> Result<ComparisonRunEvidence, OraclePipelineError> {
    let mut events = run
        .run_outcome
        .anomalies
        .iter()
        .map(signature_anomaly)
        .collect::<Result<Vec<_>, _>>()?;
    if let Some(termination) = &run.run_outcome.termination {
        events.push(signature_termination(termination, &events)?);
    }
    Ok(ComparisonRunEvidence {
        label: run.label.clone(),
        role: run.role,
        complete: run.complete,
        final_boundary_identity: run.final_boundary_identity.clone(),
        events,
    })
}

fn signature_anomaly(
    observation: &RunAnomalyObservation,
) -> Result<SemanticEventSignature, OraclePipelineError> {
    let (simulation_tick, tape_frame) = anomaly_provenance(observation);
    let mut payload = serde_json::to_value(observation)
        .map_err(|error| OraclePipelineError::new(error.to_string()))?;
    if let Value::Object(object) = &mut payload {
        object.remove("simulation_tick");
        object.remove("tape_frame");
        let start = object.remove("start_tick").and_then(|value| value.as_u64());
        let end = object.remove("end_tick").and_then(|value| value.as_u64());
        if let (Some(start), Some(end)) = (start, end) {
            object.insert(
                "duration_ticks".into(),
                Value::from(end.saturating_sub(start).saturating_add(1)),
            );
        }
    }
    Ok(SemanticEventSignature {
        simulation_tick,
        tape_frame,
        event_kind: anomaly_kind(observation).into(),
        signature: signature_value(&payload)?,
    })
}

fn signature_termination(
    termination: &RunTermination,
    events: &[SemanticEventSignature],
) -> Result<SemanticEventSignature, OraclePipelineError> {
    let mut payload = serde_json::to_value(termination)
        .map_err(|error| OraclePipelineError::new(error.to_string()))?;
    if let Value::Object(object) = &mut payload {
        object.remove("last_simulation_tick");
    }
    let simulation_tick = match termination {
        RunTermination::TimedOut {
            last_simulation_tick,
            ..
        } => *last_simulation_tick,
        _ => events.last().map_or(0, |event| event.simulation_tick),
    };
    let event_kind = match termination {
        RunTermination::Completed { .. } => "termination_completed",
        RunTermination::Crashed { .. } => "termination_crashed",
        RunTermination::TimedOut { .. } => "termination_timed_out",
    };
    Ok(SemanticEventSignature {
        simulation_tick,
        tape_frame: events.last().and_then(|event| event.tape_frame),
        event_kind: event_kind.into(),
        signature: signature_value(&payload)?,
    })
}

fn anomaly_provenance(observation: &RunAnomalyObservation) -> (u64, Option<u64>) {
    match observation {
        RunAnomalyObservation::ActorCorruption {
            simulation_tick,
            tape_frame,
            ..
        }
        | RunAnomalyObservation::SlotExhaustion {
            simulation_tick,
            tape_frame,
            ..
        }
        | RunAnomalyObservation::WatchedFieldCorruption {
            simulation_tick,
            tape_frame,
            ..
        }
        | RunAnomalyObservation::DuplicateItemReward {
            simulation_tick,
            tape_frame,
            ..
        }
        | RunAnomalyObservation::PreservedStorageState {
            simulation_tick,
            tape_frame,
            ..
        }
        | RunAnomalyObservation::EventQueueing {
            simulation_tick,
            tape_frame,
            ..
        }
        | RunAnomalyObservation::SequenceBreak {
            simulation_tick,
            tape_frame,
            ..
        } => (*simulation_tick, *tape_frame),
        RunAnomalyObservation::HeapFailure {
            simulation_tick,
            tape_frame,
            ..
        }
        | RunAnomalyObservation::SaveStateAnomaly {
            simulation_tick,
            tape_frame,
            ..
        } => (simulation_tick.unwrap_or(0), *tape_frame),
        RunAnomalyObservation::Softlock {
            end_tick,
            tape_frame,
            ..
        }
        | RunAnomalyObservation::ControlLoss {
            end_tick,
            tape_frame,
            ..
        } => (*end_tick, *tape_frame),
    }
}

fn anomaly_kind(observation: &RunAnomalyObservation) -> &'static str {
    match observation {
        RunAnomalyObservation::ActorCorruption { .. } => "actor_corruption",
        RunAnomalyObservation::SlotExhaustion { .. } => "slot_exhaustion",
        RunAnomalyObservation::WatchedFieldCorruption { .. } => "watched_field_corruption",
        RunAnomalyObservation::HeapFailure { .. } => "heap_failure",
        RunAnomalyObservation::Softlock { .. } => "softlock",
        RunAnomalyObservation::ControlLoss { .. } => "control_loss",
        RunAnomalyObservation::DuplicateItemReward { .. } => "duplicate_item_reward",
        RunAnomalyObservation::PreservedStorageState { .. } => "preserved_storage_state",
        RunAnomalyObservation::EventQueueing { .. } => "event_queueing",
        RunAnomalyObservation::SequenceBreak { .. } => "sequence_break",
        RunAnomalyObservation::SaveStateAnomaly { .. } => "save_state_anomaly",
    }
}

fn signature_value(value: &Value) -> Result<String, OraclePipelineError> {
    let encoded =
        serde_json::to_vec(value).map_err(|error| OraclePipelineError::new(error.to_string()))?;
    Ok(hex_digest(&encoded))
}

fn hex_digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[derive(Debug)]
pub struct OraclePipelineError(String);

impl OraclePipelineError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for OraclePipelineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Error for OraclePipelineError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::semantic_oracle::{RUN_OUTCOME_SCHEMA_V1, RunEvidenceKind};

    fn outcome(tick: u64) -> RunOutcomeEvidence {
        RunOutcomeEvidence {
            schema: RUN_OUTCOME_SCHEMA_V1.into(),
            monitored: vec![RunEvidenceKind::WatchedFields],
            termination: Some(RunTermination::Completed { exit_code: 0 }),
            anomalies: vec![RunAnomalyObservation::WatchedFieldCorruption {
                simulation_tick: tick,
                tape_frame: Some(tick - 1),
                field: "player.wallet".into(),
                expected: "0..=999".into(),
                actual: "65535".into(),
            }],
        }
    }

    #[test]
    fn composition_removes_tick_provenance_from_semantic_signature() {
        let first = OracleCompositionRun {
            label: "control".into(),
            role: ComparisonRunRole::Control,
            complete: true,
            final_boundary_identity: None,
            run_outcome: outcome(10),
        };
        let second = OracleCompositionRun {
            label: "treatment".into(),
            role: ComparisonRunRole::Treatment,
            complete: true,
            final_boundary_identity: None,
            run_outcome: outcome(20),
        };
        let manifest = OracleCompositionManifest {
            schema: ORACLE_COMPOSITION_SCHEMA_V1.into(),
            catalog: SemanticEventCatalog {
                schema: SEMANTIC_EVENT_CATALOG_SCHEMA_V1.into(),
                signatures: vec![],
            },
            runs: vec![first, second],
        };
        let evidence = manifest.compose().unwrap();
        assert_eq!(
            evidence.runs[0].events[0].signature,
            evidence.runs[1].events[0].signature
        );
        assert_ne!(
            evidence.runs[0].events[0].simulation_tick,
            evidence.runs[1].events[0].simulation_tick
        );
        assert_ne!(evidence.catalog_identity, "0".repeat(64));
    }

    #[test]
    fn checked_in_composition_fixture_produces_valid_comparison_evidence() {
        let manifest: OracleCompositionManifest = serde_json::from_str(include_str!(
            "../../../tests/fixtures/automation/oracle_composition.json"
        ))
        .unwrap();
        let evidence = manifest.compose().unwrap();
        assert_eq!(evidence.runs.len(), 1);
        assert_eq!(evidence.runs[0].events.len(), 2);
        assert_eq!(
            evidence.runs[0].events[0].event_kind,
            "watched_field_corruption"
        );
        assert_eq!(
            evidence.runs[0].events[1].event_kind,
            "termination_completed"
        );
    }
}
