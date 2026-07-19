//! Memory-first admission gate for latent visual/world-model research.

use crate::artifact::Digest;
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

pub const LATENT_MODEL_ADMISSION_SCHEMA_V1: &str = "dusklight-latent-model-admission/v1";
const MAX_SIGNALS: usize = 4096;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ObservationSignalEvidence {
    pub signal_id: String,
    pub evidence_sha256: Digest,
    pub required_for_objective: bool,
    pub memory_backed_available: bool,
    pub alternate_observation_available: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct ConsoleTransferGapEvidence {
    pub report_sha256: Digest,
    pub console_corpus_sha256: Digest,
    pub samples: usize,
    pub memory_baseline_mismatch_rate: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct LatentModelAdmissionConfig {
    pub minimum_console_transfer_samples: usize,
    pub minimum_console_mismatch_rate: f64,
}

impl Default for LatentModelAdmissionConfig {
    fn default() -> Self {
        Self {
            minimum_console_transfer_samples: 512,
            minimum_console_mismatch_rate: 0.05,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LatentModelAdmissionDisposition {
    RetainMemoryBackedState,
    MissingSignalHasNoAlternateObservation,
    InsufficientConsoleTransferEvidence,
    EligibleMissingObservationSupplement,
    EligibleConsoleTransferSupplement,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LatentModelAdmissionReport {
    pub schema: &'static str,
    pub objective_sha256: Digest,
    pub observation_inventory_sha256: Digest,
    pub config: LatentModelAdmissionConfig,
    pub signals: usize,
    pub required_signals: usize,
    pub required_signals_missing_from_memory: usize,
    pub missing_signals_with_alternate_observation: usize,
    pub console_transfer: Option<ConsoleTransferGapEvidence>,
    pub disposition: LatentModelAdmissionDisposition,
    pub latent_research_authorized: bool,
    pub memory_backed_state_is_default: bool,
    pub latent_may_replace_available_memory_state: bool,
    pub promotion_authority: bool,
    pub report_sha256: Digest,
}

impl LatentModelAdmissionReport {
    pub fn assess(
        objective_sha256: Digest,
        signals: &[ObservationSignalEvidence],
        console_transfer: Option<ConsoleTransferGapEvidence>,
        config: LatentModelAdmissionConfig,
    ) -> Result<Self, LatentModelAdmissionError> {
        validate_inputs(objective_sha256, signals, console_transfer, config)?;
        let observation_inventory_sha256 =
            canonical_digest(b"dusklight.observation-signal-inventory/v1\0", &signals)?;
        let required_signals = signals
            .iter()
            .filter(|signal| signal.required_for_objective)
            .count();
        let missing = signals
            .iter()
            .filter(|signal| signal.required_for_objective && !signal.memory_backed_available)
            .collect::<Vec<_>>();
        let missing_with_alternate = missing
            .iter()
            .filter(|signal| signal.alternate_observation_available)
            .count();
        let console_ready = console_transfer.is_some_and(|evidence| {
            evidence.samples >= config.minimum_console_transfer_samples
                && evidence.memory_baseline_mismatch_rate >= config.minimum_console_mismatch_rate
        });
        let disposition = if !missing.is_empty() && missing_with_alternate != missing.len() {
            LatentModelAdmissionDisposition::MissingSignalHasNoAlternateObservation
        } else if !missing.is_empty() {
            LatentModelAdmissionDisposition::EligibleMissingObservationSupplement
        } else if console_ready {
            LatentModelAdmissionDisposition::EligibleConsoleTransferSupplement
        } else if console_transfer.is_some() {
            LatentModelAdmissionDisposition::InsufficientConsoleTransferEvidence
        } else {
            LatentModelAdmissionDisposition::RetainMemoryBackedState
        };
        let latent_research_authorized = matches!(
            disposition,
            LatentModelAdmissionDisposition::EligibleMissingObservationSupplement
                | LatentModelAdmissionDisposition::EligibleConsoleTransferSupplement
        );
        let mut report = Self {
            schema: LATENT_MODEL_ADMISSION_SCHEMA_V1,
            objective_sha256,
            observation_inventory_sha256,
            config,
            signals: signals.len(),
            required_signals,
            required_signals_missing_from_memory: missing.len(),
            missing_signals_with_alternate_observation: missing_with_alternate,
            console_transfer,
            disposition,
            latent_research_authorized,
            memory_backed_state_is_default: true,
            latent_may_replace_available_memory_state: false,
            promotion_authority: false,
            report_sha256: Digest::ZERO,
        };
        report.report_sha256 = report.digest()?;
        Ok(report)
    }

    fn digest(&self) -> Result<Digest, LatentModelAdmissionError> {
        canonical_digest(
            b"dusklight.latent-model-admission/v1\0",
            &(
                self.schema,
                self.objective_sha256,
                self.observation_inventory_sha256,
                self.config,
                self.signals,
                self.required_signals,
                self.required_signals_missing_from_memory,
                self.missing_signals_with_alternate_observation,
                self.console_transfer,
                self.disposition,
                self.latent_research_authorized,
                self.memory_backed_state_is_default,
                self.latent_may_replace_available_memory_state,
                self.promotion_authority,
            ),
        )
    }
}

fn validate_inputs(
    objective_sha256: Digest,
    signals: &[ObservationSignalEvidence],
    console_transfer: Option<ConsoleTransferGapEvidence>,
    config: LatentModelAdmissionConfig,
) -> Result<(), LatentModelAdmissionError> {
    if objective_sha256 == Digest::ZERO
        || signals.is_empty()
        || signals.len() > MAX_SIGNALS
        || config.minimum_console_transfer_samples == 0
        || !config.minimum_console_mismatch_rate.is_finite()
        || !(0.0..=1.0).contains(&config.minimum_console_mismatch_rate)
    {
        return Err(LatentModelAdmissionError::new(
            "latent-model admission configuration is invalid",
        ));
    }
    let mut signal_ids = BTreeSet::new();
    if signals.iter().any(|signal| {
        signal.signal_id.is_empty()
            || signal.signal_id.len() > 128
            || !signal.signal_id.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
            })
            || !signal_ids.insert(signal.signal_id.as_str())
            || signal.evidence_sha256 == Digest::ZERO
    }) {
        return Err(LatentModelAdmissionError::new(
            "observation signal evidence is invalid or duplicated",
        ));
    }
    if console_transfer.is_some_and(|evidence| {
        evidence.report_sha256 == Digest::ZERO
            || evidence.console_corpus_sha256 == Digest::ZERO
            || evidence.samples == 0
            || !evidence.memory_baseline_mismatch_rate.is_finite()
            || !(0.0..=1.0).contains(&evidence.memory_baseline_mismatch_rate)
    }) {
        return Err(LatentModelAdmissionError::new(
            "console-transfer gap evidence is invalid",
        ));
    }
    Ok(())
}

fn canonical_digest<T: Serialize>(
    domain: &[u8],
    value: &T,
) -> Result<Digest, LatentModelAdmissionError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| LatentModelAdmissionError::new(error.to_string()))?;
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(bytes);
    Ok(Digest(hasher.finalize().into()))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LatentModelAdmissionError(String);

impl LatentModelAdmissionError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for LatentModelAdmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for LatentModelAdmissionError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn signal(
        id: &str,
        required: bool,
        memory: bool,
        alternate: bool,
    ) -> ObservationSignalEvidence {
        ObservationSignalEvidence {
            signal_id: id.into(),
            evidence_sha256: Digest([id.as_bytes()[0]; 32]),
            required_for_objective: required,
            memory_backed_available: memory,
            alternate_observation_available: alternate,
        }
    }

    #[test]
    fn available_memory_remains_default_and_missing_signal_only_supplements_it() {
        let memory = LatentModelAdmissionReport::assess(
            Digest([1; 32]),
            &[signal("position", true, true, true)],
            None,
            LatentModelAdmissionConfig::default(),
        )
        .unwrap();
        assert_eq!(
            memory.disposition,
            LatentModelAdmissionDisposition::RetainMemoryBackedState
        );
        assert!(!memory.latent_research_authorized);

        let missing = LatentModelAdmissionReport::assess(
            Digest([1; 32]),
            &[
                signal("position", true, true, true),
                signal("visual_occlusion", true, false, true),
            ],
            None,
            LatentModelAdmissionConfig::default(),
        )
        .unwrap();
        assert_eq!(
            missing.disposition,
            LatentModelAdmissionDisposition::EligibleMissingObservationSupplement
        );
        assert!(missing.latent_research_authorized);
        assert!(missing.memory_backed_state_is_default);
        assert!(!missing.latent_may_replace_available_memory_state);
        assert!(!missing.promotion_authority);
    }

    #[test]
    fn console_transfer_requires_measured_gap_and_missing_signal_needs_source() {
        let signals = [signal("position", true, true, true)];
        let weak = LatentModelAdmissionReport::assess(
            Digest([1; 32]),
            &signals,
            Some(ConsoleTransferGapEvidence {
                report_sha256: Digest([2; 32]),
                console_corpus_sha256: Digest([3; 32]),
                samples: 20,
                memory_baseline_mismatch_rate: 0.2,
            }),
            LatentModelAdmissionConfig::default(),
        )
        .unwrap();
        assert_eq!(
            weak.disposition,
            LatentModelAdmissionDisposition::InsufficientConsoleTransferEvidence
        );

        let transfer = LatentModelAdmissionReport::assess(
            Digest([1; 32]),
            &signals,
            Some(ConsoleTransferGapEvidence {
                report_sha256: Digest([2; 32]),
                console_corpus_sha256: Digest([3; 32]),
                samples: 600,
                memory_baseline_mismatch_rate: 0.2,
            }),
            LatentModelAdmissionConfig::default(),
        )
        .unwrap();
        assert_eq!(
            transfer.disposition,
            LatentModelAdmissionDisposition::EligibleConsoleTransferSupplement
        );

        let unavailable = LatentModelAdmissionReport::assess(
            Digest([1; 32]),
            &[signal("hidden_signal", true, false, false)],
            None,
            LatentModelAdmissionConfig::default(),
        )
        .unwrap();
        assert_eq!(
            unavailable.disposition,
            LatentModelAdmissionDisposition::MissingSignalHasNoAlternateObservation
        );
        assert!(!unavailable.latent_research_authorized);
    }
}
