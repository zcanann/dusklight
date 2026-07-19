//! Canonical per-boundary hashes over the supported typed-fact aperture.

use crate::artifact::Digest;
use crate::tape::TapeBoot;
use crate::trace::DecodedTrace;
use dusklight_automation_contracts::typed_facts::{
    TypedFactId, TypedFactPhase, TypedFactResponse, TypedFactValueType,
};
use dusklight_trace::trace_typed_facts::typed_facts_from_trace_record;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

pub const SEMANTIC_STATE_HASH_SERIES_SCHEMA_V1: &str = "dusklight-semantic-state-hash-series/v1";
pub const SEMANTIC_STATE_HASH_COMPARISON_SCHEMA_V1: &str =
    "dusklight-semantic-state-hash-comparison/v1";
pub const SEMANTIC_STATE_HASH_PROFILE_ID_V1: &str = "core-typed-facts/v1";
pub const MAX_SEMANTIC_STATE_HASHES: usize = 10_000_000;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticStateField {
    pub id: TypedFactId,
    pub value_type: TypedFactValueType,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticStateHashProfile {
    pub id: String,
    pub fields: Vec<SemanticStateField>,
    pub identity_sha256: Digest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticStateHashEntry {
    pub boundary_index: u64,
    pub simulation_tick: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tape_frame: Option<u64>,
    pub phase: TypedFactPhase,
    pub facts_sha256: Digest,
    pub state_sha256: Digest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticStateHashSeries {
    pub schema: String,
    pub source_trace_sha256: Digest,
    pub trace_version: u16,
    pub boot: TapeBoot,
    pub tick_rate_numerator: u32,
    pub tick_rate_denominator: u32,
    pub profile: SemanticStateHashProfile,
    pub entries: Vec<SemanticStateHashEntry>,
    pub series_sha256: Digest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticStateHashComparison {
    pub schema: String,
    pub profile_sha256: Digest,
    pub left_series_sha256: Digest,
    pub right_series_sha256: Digest,
    pub compared_boundaries: u64,
    pub identical: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_divergence: Option<SemanticStateHashDivergence>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticStateHashDivergence {
    pub boundary_index: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left_state_sha256: Option<Digest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub right_state_sha256: Option<Digest>,
}

impl SemanticStateHashSeries {
    pub fn build(trace: &DecodedTrace, trace_bytes: &[u8]) -> Result<Self, SemanticStateHashError> {
        if trace.records.is_empty() {
            return Err(SemanticStateHashError::new(
                "trace contains no retained boundaries",
            ));
        }
        if trace.records.len() > MAX_SEMANTIC_STATE_HASHES {
            return Err(SemanticStateHashError::new(format!(
                "trace has {} records; limit is {MAX_SEMANTIC_STATE_HASHES}",
                trace.records.len()
            )));
        }

        let mut profile: Option<SemanticStateHashProfile> = None;
        let mut entries = Vec::with_capacity(trace.records.len());
        for record in &trace.records {
            let facts = typed_facts_from_trace_record(record);
            facts
                .validate()
                .map_err(|error| SemanticStateHashError::new(error.to_string()))?;
            let observed_profile = SemanticStateHashProfile::build(&facts)?;
            if profile
                .as_ref()
                .is_some_and(|expected| expected != &observed_profile)
            {
                return Err(SemanticStateHashError::new(
                    "typed-fact field profile changed within one trace",
                ));
            }
            profile.get_or_insert(observed_profile);
            entries.push(SemanticStateHashEntry::build(
                record.boundary_index,
                &facts,
                profile
                    .as_ref()
                    .expect("nonempty trace assigned a profile")
                    .identity_sha256,
            )?);
        }

        let mut series = Self {
            schema: SEMANTIC_STATE_HASH_SERIES_SCHEMA_V1.into(),
            source_trace_sha256: Digest(Sha256::digest(trace_bytes).into()),
            trace_version: trace.version,
            boot: trace.boot.clone(),
            tick_rate_numerator: trace.tick_rate_numerator,
            tick_rate_denominator: trace.tick_rate_denominator,
            profile: profile.expect("nonempty trace assigned a profile"),
            entries,
            series_sha256: Digest::ZERO,
        };
        series.series_sha256 = series.compute_identity()?;
        series.validate()?;
        Ok(series)
    }

    pub fn validate(&self) -> Result<(), SemanticStateHashError> {
        self.profile.validate()?;
        if self.schema != SEMANTIC_STATE_HASH_SERIES_SCHEMA_V1
            || self.source_trace_sha256 == Digest::ZERO
            || self.trace_version == 0
            || self.tick_rate_numerator == 0
            || self.tick_rate_denominator == 0
            || self.entries.is_empty()
            || self.entries.len() > MAX_SEMANTIC_STATE_HASHES
            || self
                .entries
                .windows(2)
                .any(|pair| pair[0].boundary_index >= pair[1].boundary_index)
        {
            return Err(SemanticStateHashError::new(
                "semantic state-hash series envelope is invalid",
            ));
        }
        for entry in &self.entries {
            entry.validate(self.profile.identity_sha256)?;
        }
        if self.series_sha256 != self.compute_identity()? {
            return Err(SemanticStateHashError::new(
                "semantic state-hash series seal mismatch",
            ));
        }
        Ok(())
    }

    fn compute_identity(&self) -> Result<Digest, SemanticStateHashError> {
        let mut canonical = self.clone();
        canonical.series_sha256 = Digest::ZERO;
        canonical_digest(b"dusklight.semantic-state-hash-series/v1\0", &canonical)
    }
}

impl SemanticStateHashProfile {
    fn build(facts: &TypedFactResponse) -> Result<Self, SemanticStateHashError> {
        let mut profile = Self {
            id: SEMANTIC_STATE_HASH_PROFILE_ID_V1.into(),
            fields: facts
                .entries
                .iter()
                .map(|entry| SemanticStateField {
                    id: entry.id,
                    value_type: entry.value_type,
                })
                .collect(),
            identity_sha256: Digest::ZERO,
        };
        profile.identity_sha256 = profile.compute_identity()?;
        profile.validate()?;
        Ok(profile)
    }

    fn validate(&self) -> Result<(), SemanticStateHashError> {
        if self.id != SEMANTIC_STATE_HASH_PROFILE_ID_V1
            || self.fields.is_empty()
            || self.fields.windows(2).any(|pair| pair[0].id >= pair[1].id)
            || self.identity_sha256 != self.compute_identity()?
        {
            return Err(SemanticStateHashError::new(
                "semantic state-hash field profile is invalid",
            ));
        }
        Ok(())
    }

    fn compute_identity(&self) -> Result<Digest, SemanticStateHashError> {
        let mut canonical = self.clone();
        canonical.identity_sha256 = Digest::ZERO;
        canonical_digest(b"dusklight.semantic-state-hash-profile/v1\0", &canonical)
    }
}

impl SemanticStateHashEntry {
    fn build(
        boundary_index: u64,
        facts: &TypedFactResponse,
        profile_sha256: Digest,
    ) -> Result<Self, SemanticStateHashError> {
        let facts_sha256 = canonical_digest(b"dusklight.semantic-state-typed-facts/v1\0", facts)?;
        let mut entry = Self {
            boundary_index,
            simulation_tick: facts.simulation_tick,
            tape_frame: facts.tape_frame,
            phase: facts.phase,
            facts_sha256,
            state_sha256: Digest::ZERO,
        };
        entry.state_sha256 = entry.compute_state_hash(profile_sha256)?;
        Ok(entry)
    }

    fn validate(&self, profile_sha256: Digest) -> Result<(), SemanticStateHashError> {
        if self.facts_sha256 == Digest::ZERO
            || self.state_sha256 != self.compute_state_hash(profile_sha256)?
        {
            return Err(SemanticStateHashError::new(
                "semantic state-hash entry is invalid",
            ));
        }
        Ok(())
    }

    fn compute_state_hash(&self, profile_sha256: Digest) -> Result<Digest, SemanticStateHashError> {
        canonical_digest(
            b"dusklight.semantic-state/v1\0",
            &(
                profile_sha256,
                self.boundary_index,
                self.simulation_tick,
                self.tape_frame,
                self.phase,
                self.facts_sha256,
            ),
        )
    }
}

impl SemanticStateHashComparison {
    pub fn compare(
        left: &SemanticStateHashSeries,
        right: &SemanticStateHashSeries,
    ) -> Result<Self, SemanticStateHashError> {
        left.validate()?;
        right.validate()?;
        if left.profile != right.profile
            || left.boot != right.boot
            || left.tick_rate_numerator != right.tick_rate_numerator
            || left.tick_rate_denominator != right.tick_rate_denominator
        {
            return Err(SemanticStateHashError::new(
                "state-hash series use incompatible profiles, boot origins, or tick rates",
            ));
        }

        let left_entries = left
            .entries
            .iter()
            .map(|entry| (entry.boundary_index, entry))
            .collect::<BTreeMap<_, _>>();
        let right_entries = right
            .entries
            .iter()
            .map(|entry| (entry.boundary_index, entry))
            .collect::<BTreeMap<_, _>>();
        let boundaries = left_entries
            .keys()
            .chain(right_entries.keys())
            .copied()
            .collect::<BTreeSet<_>>();
        let first_divergence = boundaries.iter().find_map(|boundary_index| {
            let left_hash = left_entries
                .get(boundary_index)
                .map(|entry| entry.state_sha256);
            let right_hash = right_entries
                .get(boundary_index)
                .map(|entry| entry.state_sha256);
            (left_hash != right_hash).then_some(SemanticStateHashDivergence {
                boundary_index: *boundary_index,
                left_state_sha256: left_hash,
                right_state_sha256: right_hash,
            })
        });
        Ok(Self {
            schema: SEMANTIC_STATE_HASH_COMPARISON_SCHEMA_V1.into(),
            profile_sha256: left.profile.identity_sha256,
            left_series_sha256: left.series_sha256,
            right_series_sha256: right.series_sha256,
            compared_boundaries: boundaries.len() as u64,
            identical: first_divergence.is_none(),
            first_divergence,
        })
    }
}

fn canonical_digest(
    domain: &[u8],
    value: &impl Serialize,
) -> Result<Digest, SemanticStateHashError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| SemanticStateHashError::new(error.to_string()))?;
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
    Ok(Digest(hasher.finalize().into()))
}

#[derive(Debug)]
pub struct SemanticStateHashError(String);

impl SemanticStateHashError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for SemanticStateHashError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for SemanticStateHashError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace::{TraceChannel, TraceChannelStatus, TracePhase, TraceRecord};
    use std::collections::BTreeMap;

    fn trace(position: [f32; 3]) -> DecodedTrace {
        let mut channel_status = BTreeMap::new();
        for channel in [
            TraceChannel::Stage,
            TraceChannel::PlayerMotion,
            TraceChannel::Event,
        ] {
            channel_status.insert(channel, TraceChannelStatus::Present);
        }
        let record = |boundary_index, x| TraceRecord {
            boundary_index,
            simulation_tick: boundary_index,
            tape_frame: boundary_index.checked_sub(1),
            observation_phase: TracePhase::PostSimulation,
            channel_status: channel_status.clone(),
            stage_name: "F_SP103".into(),
            flags: 3,
            player_session_process_id: Some(1),
            position: [x, position[1], position[2]],
            ..TraceRecord::default()
        };
        DecodedTrace {
            version: 5,
            boot: TapeBoot::Process,
            tick_rate_numerator: 30,
            tick_rate_denominator: 1,
            requested_channels: 0,
            capacity_exhausted: false,
            retention: None,
            channel_formats: BTreeMap::new(),
            records: vec![record(1, 1.0), record(2, position[0])],
        }
    }

    #[test]
    fn deterministic_series_find_exact_changed_or_missing_boundary() {
        let reference =
            SemanticStateHashSeries::build(&trace([2.0, 3.0, 4.0]), b"trace-a").unwrap();
        let repeated = SemanticStateHashSeries::build(&trace([2.0, 3.0, 4.0]), b"trace-a").unwrap();
        assert_eq!(reference, repeated);
        assert!(
            SemanticStateHashComparison::compare(&reference, &repeated)
                .unwrap()
                .identical
        );

        let changed = SemanticStateHashSeries::build(&trace([9.0, 3.0, 4.0]), b"trace-b").unwrap();
        let comparison = SemanticStateHashComparison::compare(&reference, &changed).unwrap();
        assert!(!comparison.identical);
        assert_eq!(comparison.first_divergence.unwrap().boundary_index, 2);

        let mut missing = repeated.clone();
        missing.entries.remove(0);
        missing.series_sha256 = missing.compute_identity().unwrap();
        let comparison = SemanticStateHashComparison::compare(&reference, &missing).unwrap();
        assert_eq!(comparison.first_divergence.unwrap().boundary_index, 1);
    }

    #[test]
    fn seals_reject_profile_context_order_and_source_tampering() {
        let reference =
            SemanticStateHashSeries::build(&trace([2.0, 3.0, 4.0]), b"trace-a").unwrap();
        for mutate in [
            |series: &mut SemanticStateHashSeries| series.source_trace_sha256 = Digest([7; 32]),
            |series: &mut SemanticStateHashSeries| series.entries[0].simulation_tick += 1,
            |series: &mut SemanticStateHashSeries| series.entries.swap(0, 1),
            |series: &mut SemanticStateHashSeries| {
                series.profile.fields[0].value_type = TypedFactValueType::U32
            },
        ] {
            let mut tampered = reference.clone();
            mutate(&mut tampered);
            assert!(tampered.validate().is_err());
        }

        let mut incompatible = reference.clone();
        incompatible.boot = TapeBoot::Stage {
            stage: "F_SP103".into(),
            room: 0,
            point: 0,
            layer: -1,
            save_slot: None,
            fixture: None,
        };
        incompatible.series_sha256 = incompatible.compute_identity().unwrap();
        assert!(SemanticStateHashComparison::compare(&reference, &incompatible).is_err());
    }
}
