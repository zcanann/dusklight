//! Field and channel coverage derived from authenticated stage-survey traces.
//!
//! This report never proposes an action or evaluates a gameplay objective. It
//! answers only whether requested observation channels were available and
//! whether their serialized fields varied under generic, identity-bound probes.

use crate::stage_survey::{
    StageSurveyAttemptOutcome, StageSurveyClassification, StageSurveyLedger,
    StageSurveyObservedOrigin, StageSurveyProbeKind, applied_pad_matches_frame,
    stage_survey_case_storage_id, survey_probe_tape,
};
use dusklight_automation_contracts::artifact::Digest;
use dusklight_trace::trace::{self, DecodedTrace, TraceChannel, TraceChannelStatus, TraceRecord};
use dusklight_world::stage_boot_catalog::StageBootCatalog;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

pub const STAGE_OBSERVATION_COVERAGE_SCHEMA_V1: &str = "dusklight-stage-observation-coverage/v1";

pub struct StageObservationCoverageSource<'a> {
    pub ledger: &'a StageSurveyLedger,
    pub state_root: &'a Path,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationCoverageStatus {
    Present,
    Varying,
    Absent,
    Ambiguous,
    Truncated,
    RequiresTypedExtension,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationCoverageCaseStatus {
    VerifiedTrace,
    ArtifactMissing,
    ArtifactRejected,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StageObservationCompatibility {
    pub executable_sha256: Digest,
    pub game_data_sha256: Digest,
    pub card_fixture_sha256: Digest,
    pub observation_schema_sha256: Digest,
    pub fidelity_profile: String,
    pub probe_ticks: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StageObservationCoverageSourceSummary {
    pub ledger_sha256: Digest,
    pub probe: StageSurveyProbeKind,
    pub ready_case_count: u32,
    pub verified_case_count: u32,
    pub rejected_case_count: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StageObservationCoverageCase {
    pub ledger_sha256: Digest,
    pub candidate_id: String,
    pub requested_stage: String,
    pub requested_room: i8,
    pub requested_point: i16,
    pub requested_layer: i8,
    pub observed_origin: StageSurveyObservedOrigin,
    pub observed_final: StageSurveyObservedOrigin,
    pub stage_records: Vec<StageObservationCoverageStageRecords>,
    pub probe: StageSurveyProbeKind,
    pub status: ObservationCoverageCaseStatus,
    pub observation_sha256: Digest,
    pub record_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StageObservationCoverageStageRecords {
    pub stage: String,
    pub record_count: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StageObservationChannelCoverage {
    pub channel: String,
    pub status: ObservationCoverageStatus,
    pub present_samples: u64,
    pub absent_samples: u64,
    pub unavailable_samples: u64,
    pub not_sampled_samples: u64,
    pub truncated_samples: u64,
    pub unreported_samples: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StageObservationFieldCoverage {
    pub path: String,
    pub status: ObservationCoverageStatus,
    pub sampled_records: u64,
    pub missing_records: u64,
    pub value_samples: u64,
    pub null_samples: u64,
    pub true_samples: u64,
    pub distinct_nonnull_values: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StageObservationCoverageCell {
    pub stage: String,
    pub probe: StageSurveyProbeKind,
    pub case_count: u32,
    pub record_count: u64,
    pub channels: Vec<StageObservationChannelCoverage>,
    pub fields: Vec<StageObservationFieldCoverage>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StageObservationCoverageReport {
    pub schema: String,
    pub catalog_sha256: Digest,
    pub compatibility: StageObservationCompatibility,
    pub sources: Vec<StageObservationCoverageSourceSummary>,
    pub cases: Vec<StageObservationCoverageCase>,
    pub cells: Vec<StageObservationCoverageCell>,
    pub report_sha256: Digest,
}

#[derive(Default)]
struct CellAccumulator {
    case_count: u32,
    record_count: u64,
    channels: BTreeMap<TraceChannel, ChannelAccumulator>,
    fields: BTreeMap<String, FieldAccumulator>,
}

#[derive(Clone, Default)]
struct ChannelAccumulator {
    present: u64,
    absent: u64,
    unavailable: u64,
    not_sampled: u64,
    truncated: u64,
    unreported: u64,
}

#[derive(Default)]
struct FieldAccumulator {
    sampled_records: BTreeSet<u64>,
    value_samples: u64,
    null_samples: u64,
    true_samples: u64,
    distinct_nonnull: BTreeSet<[u8; 32]>,
}

impl StageObservationCoverageReport {
    pub fn build(
        catalog: &StageBootCatalog,
        sources: &[StageObservationCoverageSource<'_>],
    ) -> Result<Self, StageObservationCoverageError> {
        if sources.is_empty() {
            return Err(StageObservationCoverageError::new(
                "observation coverage requires at least one survey source",
            ));
        }
        let catalog_sha256 = catalog
            .digest()
            .map_err(|error| StageObservationCoverageError::new(error.to_string()))?;
        let first = sources[0].ledger;
        first
            .validate(catalog)
            .map_err(|error| StageObservationCoverageError::new(error.to_string()))?;
        let common_compatibility = compatibility(first);
        let candidates = catalog
            .candidates
            .iter()
            .map(|candidate| (candidate.id.as_str(), candidate))
            .collect::<BTreeMap<_, _>>();
        let mut source_summaries = Vec::new();
        let mut cases = Vec::new();
        let mut cells = BTreeMap::<(String, StageSurveyProbeKind), CellAccumulator>::new();
        let mut source_identities = BTreeSet::new();

        for source in sources {
            source
                .ledger
                .validate(catalog)
                .map_err(|error| StageObservationCoverageError::new(error.to_string()))?;
            if compatibility(source.ledger) != common_compatibility {
                return Err(StageObservationCoverageError::new(
                    "observation coverage sources have incompatible executable, game, card, schema, fidelity, or duration",
                ));
            }
            let ledger_sha256 = source
                .ledger
                .digest(catalog)
                .map_err(|error| StageObservationCoverageError::new(error.to_string()))?;
            if !source_identities.insert(ledger_sha256) {
                return Err(StageObservationCoverageError::new(
                    "observation coverage contains a duplicate survey ledger",
                ));
            }
            let source_case_start = cases.len();
            for case in &source.ledger.cases {
                if case.classification != Some(StageSurveyClassification::Ready) {
                    continue;
                }
                let candidate = candidates.get(case.candidate_id.as_str()).ok_or_else(|| {
                    StageObservationCoverageError::new(
                        "survey case is detached from the boot catalog",
                    )
                })?;
                let attempt = case.attempts.last().ok_or_else(|| {
                    StageObservationCoverageError::new("ready survey case has no attempt")
                })?;
                if attempt.outcome != StageSurveyAttemptOutcome::Ready {
                    return Err(StageObservationCoverageError::new(
                        "ready survey classification does not end in a ready attempt",
                    ));
                }
                let expected_digest = attempt.observation_sha256.ok_or_else(|| {
                    StageObservationCoverageError::new(
                        "ready survey attempt has no observation identity",
                    )
                })?;
                let artifact = locate_trace_artifact(
                    source.state_root,
                    &candidate.id,
                    attempt.number,
                    expected_digest,
                )?;
                let (status, decoded, diagnostic) = match artifact {
                    None => (
                        ObservationCoverageCaseStatus::ArtifactMissing,
                        None,
                        Some("observation_trace_artifact_missing".into()),
                    ),
                    Some(bytes) => match validate_trace(candidate, source.ledger, &bytes) {
                        Ok(decoded) => (
                            ObservationCoverageCaseStatus::VerifiedTrace,
                            Some(decoded),
                            None,
                        ),
                        Err(error) => (
                            ObservationCoverageCaseStatus::ArtifactRejected,
                            None,
                            Some(error),
                        ),
                    },
                };
                let observed_origin = attempt.observed_origin.clone().ok_or_else(|| {
                    StageObservationCoverageError::new("ready survey case has no observed origin")
                })?;
                let observed_final = attempt.observed_final.clone().ok_or_else(|| {
                    StageObservationCoverageError::new(
                        "ready survey case has no observed final state",
                    )
                })?;
                let record_count = decoded
                    .as_ref()
                    .map_or(0, |trace| trace.records.len() as u32);
                let stage_records = decoded.as_ref().map_or_else(
                    || Ok(Vec::new()),
                    |decoded| {
                        accumulate_trace_cells(&mut cells, source.ledger.policy.probe, decoded)
                    },
                )?;
                cases.push(StageObservationCoverageCase {
                    ledger_sha256,
                    candidate_id: candidate.id.clone(),
                    requested_stage: candidate.stage.clone(),
                    requested_room: candidate.room,
                    requested_point: candidate.point,
                    requested_layer: candidate.layer,
                    observed_origin,
                    observed_final,
                    stage_records,
                    probe: source.ledger.policy.probe,
                    status,
                    observation_sha256: expected_digest,
                    record_count,
                    diagnostic,
                });
            }
            let source_cases = &cases[source_case_start..];
            let verified_case_count = source_cases
                .iter()
                .filter(|case| case.status == ObservationCoverageCaseStatus::VerifiedTrace)
                .count() as u32;
            source_summaries.push(StageObservationCoverageSourceSummary {
                ledger_sha256,
                probe: source.ledger.policy.probe,
                ready_case_count: source_cases.len() as u32,
                verified_case_count,
                rejected_case_count: source_cases.len() as u32 - verified_case_count,
            });
        }

        source_summaries.sort_by_key(|source| (source.probe, source.ledger_sha256));
        cases.sort_by(|left, right| {
            (
                &left.requested_stage,
                left.probe,
                &left.candidate_id,
                left.ledger_sha256,
            )
                .cmp(&(
                    &right.requested_stage,
                    right.probe,
                    &right.candidate_id,
                    right.ledger_sha256,
                ))
        });
        let cells = cells
            .into_iter()
            .map(|((stage, probe), accumulator)| finish_cell(stage, probe, accumulator))
            .collect::<Vec<_>>();
        let mut report = Self {
            schema: STAGE_OBSERVATION_COVERAGE_SCHEMA_V1.into(),
            catalog_sha256,
            compatibility: common_compatibility,
            sources: source_summaries,
            cases,
            cells,
            report_sha256: Digest::ZERO,
        };
        report.report_sha256 = report.compute_digest()?;
        report.validate()?;
        Ok(report)
    }

    pub fn validate(&self) -> Result<(), StageObservationCoverageError> {
        if self.schema != STAGE_OBSERVATION_COVERAGE_SCHEMA_V1
            || self.catalog_sha256 == Digest::ZERO
            || self.sources.is_empty()
            || self.compatibility.executable_sha256 == Digest::ZERO
            || self.compatibility.game_data_sha256 == Digest::ZERO
            || self.compatibility.card_fixture_sha256 == Digest::ZERO
            || self.compatibility.observation_schema_sha256 == Digest::ZERO
            || self.compatibility.fidelity_profile.is_empty()
            || self.compatibility.probe_ticks == 0
        {
            return Err(StageObservationCoverageError::new(
                "observation coverage envelope is invalid",
            ));
        }
        let unique_sources = self
            .sources
            .iter()
            .map(|source| source.ledger_sha256)
            .collect::<BTreeSet<_>>();
        let source_by_digest = self
            .sources
            .iter()
            .map(|source| (source.ledger_sha256, source))
            .collect::<BTreeMap<_, _>>();
        if unique_sources.len() != self.sources.len()
            || self.sources.windows(2).any(|pair| {
                (pair[0].probe, pair[0].ledger_sha256) >= (pair[1].probe, pair[1].ledger_sha256)
            })
            || self
                .cells
                .windows(2)
                .any(|pair| (&pair[0].stage, pair[0].probe) >= (&pair[1].stage, pair[1].probe))
        {
            return Err(StageObservationCoverageError::new(
                "observation coverage ordering is invalid",
            ));
        }
        for source in &self.sources {
            let observed = self
                .cases
                .iter()
                .filter(|case| case.ledger_sha256 == source.ledger_sha256)
                .count() as u32;
            let verified = self
                .cases
                .iter()
                .filter(|case| {
                    case.ledger_sha256 == source.ledger_sha256
                        && case.status == ObservationCoverageCaseStatus::VerifiedTrace
                })
                .count() as u32;
            if source.ledger_sha256 == Digest::ZERO
                || observed != source.ready_case_count
                || verified != source.verified_case_count
                || verified > observed
                || observed - verified != source.rejected_case_count
            {
                return Err(StageObservationCoverageError::new(
                    "observation coverage source counts are stale",
                ));
            }
        }
        let mut expected_cells = BTreeMap::<(String, StageSurveyProbeKind), (u32, u64)>::new();
        let mut prior_case = None;
        for case in &self.cases {
            let order = (
                case.requested_stage.as_str(),
                case.probe,
                case.candidate_id.as_str(),
                case.ledger_sha256,
            );
            if prior_case.is_some_and(|prior| prior >= order)
                || case.candidate_id.is_empty()
                || case.requested_stage.is_empty()
                || case
                    .observed_origin
                    .stage
                    .as_ref()
                    .is_none_or(String::is_empty)
                || case
                    .observed_final
                    .stage
                    .as_ref()
                    .is_none_or(String::is_empty)
                || case.observation_sha256 == Digest::ZERO
                || source_by_digest
                    .get(&case.ledger_sha256)
                    .is_none_or(|source| source.probe != case.probe)
            {
                return Err(StageObservationCoverageError::new(
                    "observation coverage case identity or ordering is invalid",
                ));
            }
            prior_case = Some(order);
            if case.status == ObservationCoverageCaseStatus::VerifiedTrace {
                if case.record_count != self.compatibility.probe_ticks || case.diagnostic.is_some()
                {
                    return Err(StageObservationCoverageError::new(
                        "verified observation coverage case is incomplete",
                    ));
                }
                if case.stage_records.is_empty()
                    || case
                        .stage_records
                        .windows(2)
                        .any(|pair| pair[0].stage >= pair[1].stage)
                    || case
                        .stage_records
                        .iter()
                        .any(|slice| slice.stage.is_empty() || slice.record_count == 0)
                    || case
                        .stage_records
                        .iter()
                        .map(|slice| slice.record_count)
                        .sum::<u32>()
                        != case.record_count
                {
                    return Err(StageObservationCoverageError::new(
                        "verified observation coverage stage slices are invalid",
                    ));
                }
                for slice in &case.stage_records {
                    let cell = expected_cells
                        .entry((slice.stage.clone(), case.probe))
                        .or_default();
                    cell.0 += 1;
                    cell.1 += u64::from(slice.record_count);
                }
            } else if case.record_count != 0
                || !case.stage_records.is_empty()
                || case.diagnostic.is_none()
            {
                return Err(StageObservationCoverageError::new(
                    "rejected observation coverage case has retained values",
                ));
            }
        }
        for cell in &self.cells {
            let expected = expected_cells
                .remove(&(cell.stage.clone(), cell.probe))
                .ok_or_else(|| {
                    StageObservationCoverageError::new(
                        "observation coverage cell has no verified source cases",
                    )
                })?;
            if cell.stage.is_empty()
                || (cell.case_count, cell.record_count) != expected
                || cell.channels.len() != TraceChannel::ALL.len()
                || cell.fields.is_empty()
            {
                return Err(StageObservationCoverageError::new(
                    "observation coverage cell counts are stale",
                ));
            }
            for (channel, expected_channel) in cell.channels.iter().zip(TraceChannel::ALL) {
                let accumulator = ChannelAccumulator {
                    present: channel.present_samples,
                    absent: channel.absent_samples,
                    unavailable: channel.unavailable_samples,
                    not_sampled: channel.not_sampled_samples,
                    truncated: channel.truncated_samples,
                    unreported: channel.unreported_samples,
                };
                let total = accumulator.present
                    + accumulator.absent
                    + accumulator.unavailable
                    + accumulator.not_sampled
                    + accumulator.truncated
                    + accumulator.unreported;
                if channel.channel != expected_channel.name()
                    || total != cell.record_count
                    || channel.status != channel_status(&accumulator)
                {
                    return Err(StageObservationCoverageError::new(
                        "observation coverage channel counts are stale",
                    ));
                }
            }
            if cell
                .fields
                .windows(2)
                .any(|pair| pair[0].path >= pair[1].path)
            {
                return Err(StageObservationCoverageError::new(
                    "observation coverage fields are not canonical",
                ));
            }
            for field in &cell.fields {
                if field.path.is_empty()
                    || field.sampled_records == 0
                    || field.sampled_records > cell.record_count
                    || field.missing_records != cell.record_count - field.sampled_records
                    || field.value_samples < field.sampled_records
                    || field.null_samples > field.value_samples
                    || field.true_samples > field.value_samples - field.null_samples
                    || field.distinct_nonnull_values > field.value_samples - field.null_samples
                    || field.status
                        != field_status(
                            &field.path,
                            field.sampled_records,
                            cell.record_count,
                            field.value_samples,
                            field.null_samples,
                            field.true_samples,
                            field.distinct_nonnull_values,
                        )
                {
                    return Err(StageObservationCoverageError::new(
                        "observation coverage field counts are stale",
                    ));
                }
            }
        }
        if !expected_cells.is_empty() {
            return Err(StageObservationCoverageError::new(
                "observation coverage is missing verified cells",
            ));
        }
        if self.report_sha256 != self.compute_digest()? {
            return Err(StageObservationCoverageError::new(
                "observation coverage seal is invalid",
            ));
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, StageObservationCoverageError> {
        self.validate()?;
        serde_json::to_vec(self)
            .map_err(|error| StageObservationCoverageError::new(error.to_string()))
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, StageObservationCoverageError> {
        let report: Self = serde_json::from_slice(bytes)
            .map_err(|error| StageObservationCoverageError::new(error.to_string()))?;
        report.validate()?;
        if report.canonical_bytes()? != bytes {
            return Err(StageObservationCoverageError::new(
                "observation coverage bytes are not canonical",
            ));
        }
        Ok(report)
    }

    fn compute_digest(&self) -> Result<Digest, StageObservationCoverageError> {
        let mut canonical = self.clone();
        canonical.report_sha256 = Digest::ZERO;
        let bytes = serde_json::to_vec(&canonical)
            .map_err(|error| StageObservationCoverageError::new(error.to_string()))?;
        let mut hasher = Sha256::new();
        hasher.update(b"dusklight.stage-observation-coverage.identity/v1\0");
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
        Ok(Digest(hasher.finalize().into()))
    }
}

fn compatibility(ledger: &StageSurveyLedger) -> StageObservationCompatibility {
    StageObservationCompatibility {
        executable_sha256: ledger.identity.executable_sha256,
        game_data_sha256: ledger.identity.game_data_sha256,
        card_fixture_sha256: ledger.identity.card_fixture_sha256,
        observation_schema_sha256: ledger.identity.observation_schema_sha256,
        fidelity_profile: ledger.policy.fidelity_profile.clone(),
        probe_ticks: ledger.policy.probe_ticks,
    }
}

fn validate_trace(
    candidate: &dusklight_world::stage_boot_catalog::StageBootCandidate,
    ledger: &StageSurveyLedger,
    bytes: &[u8],
) -> Result<DecodedTrace, String> {
    let decoded = trace::decode(bytes).map_err(|_| "trace_decode_failed".to_string())?;
    let tape = survey_probe_tape(candidate, &ledger.policy)
        .map_err(|_| "probe_tape_rebuild_failed".to_string())?;
    let requested_all = TraceChannel::ALL
        .iter()
        .fold(0_u64, |mask, channel| mask | channel.bit());
    if decoded.boot != tape.boot
        || decoded.tick_rate_numerator != tape.tick_rate_numerator
        || decoded.tick_rate_denominator != tape.tick_rate_denominator
        || decoded.requested_channels != requested_all
        || decoded.capacity_exhausted
        || decoded.records.len() != tape.frames.len()
    {
        return Err("trace_envelope_mismatch".into());
    }
    if decoded
        .records
        .iter()
        .zip(&tape.frames)
        .any(|(record, frame)| !applied_pad_matches_frame(record.applied_pads.as_ref(), frame))
    {
        return Err("trace_applied_pad_mismatch".into());
    }
    let origin = decoded.records.first().ok_or("trace_empty")?;
    if origin.stage_name != candidate.stage
        || origin.room != candidate.room
        || origin.point != candidate.point
        || (candidate.layer >= 0 && origin.layer != candidate.layer)
        || origin.player_session_process_id.is_none()
    {
        return Err("trace_origin_mismatch".into());
    }
    Ok(decoded)
}

fn accumulate_records<'a>(
    accumulator: &mut CellAccumulator,
    records: impl IntoIterator<Item = &'a TraceRecord>,
) -> Result<(), StageObservationCoverageError> {
    let records = records.into_iter().collect::<Vec<_>>();
    let record_offset = accumulator.record_count;
    accumulator.record_count += records.len() as u64;
    for (record_index, record) in records.into_iter().enumerate() {
        for channel in TraceChannel::ALL {
            let channel_accumulator = accumulator.channels.entry(channel).or_default();
            match record.channel_status.get(&channel) {
                Some(TraceChannelStatus::Present) => channel_accumulator.present += 1,
                Some(TraceChannelStatus::Absent) => channel_accumulator.absent += 1,
                Some(TraceChannelStatus::Unavailable) => channel_accumulator.unavailable += 1,
                Some(TraceChannelStatus::NotSampled) => channel_accumulator.not_sampled += 1,
                Some(TraceChannelStatus::Truncated) => channel_accumulator.truncated += 1,
                None => channel_accumulator.unreported += 1,
            }
        }
        let value = serde_json::to_value(record)
            .map_err(|error| StageObservationCoverageError::new(error.to_string()))?;
        let object = value.as_object().ok_or_else(|| {
            StageObservationCoverageError::new("trace record did not serialize as an object")
        })?;
        for (key, value) in object {
            if key != "channel_status" {
                flatten_value(
                    key,
                    value,
                    record_offset + record_index as u64,
                    &mut accumulator.fields,
                )?;
            }
        }
    }
    Ok(())
}

fn accumulate_trace_cells(
    cells: &mut BTreeMap<(String, StageSurveyProbeKind), CellAccumulator>,
    probe: StageSurveyProbeKind,
    trace: &DecodedTrace,
) -> Result<Vec<StageObservationCoverageStageRecords>, StageObservationCoverageError> {
    let stages = trace
        .records
        .iter()
        .map(|record| record.stage_name.as_str())
        .collect::<BTreeSet<_>>();
    let mut stage_records = Vec::with_capacity(stages.len());
    for stage in stages {
        let records = trace
            .records
            .iter()
            .filter(|record| record.stage_name == stage)
            .collect::<Vec<_>>();
        stage_records.push(StageObservationCoverageStageRecords {
            stage: stage.to_string(),
            record_count: records.len() as u32,
        });
        let cell = cells.entry((stage.to_string(), probe)).or_default();
        cell.case_count += 1;
        accumulate_records(cell, records)?;
    }
    Ok(stage_records)
}

fn flatten_value(
    path: &str,
    value: &Value,
    record_index: u64,
    fields: &mut BTreeMap<String, FieldAccumulator>,
) -> Result<(), StageObservationCoverageError> {
    let field = fields.entry(path.to_string()).or_default();
    field.sampled_records.insert(record_index);
    field.value_samples += 1;
    if value.is_null() {
        field.null_samples += 1;
    } else {
        field.true_samples += u64::from(value == &Value::Bool(true));
        let bytes = serde_json::to_vec(value)
            .map_err(|error| StageObservationCoverageError::new(error.to_string()))?;
        field.distinct_nonnull.insert(Sha256::digest(bytes).into());
    }
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                flatten_value(&format!("{path}.{key}"), child, record_index, fields)?;
            }
        }
        Value::Array(array) => {
            for child in array {
                flatten_value(&format!("{path}[]"), child, record_index, fields)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn finish_cell(
    stage: String,
    probe: StageSurveyProbeKind,
    accumulator: CellAccumulator,
) -> StageObservationCoverageCell {
    let channels = TraceChannel::ALL
        .iter()
        .map(|channel| {
            let value = accumulator
                .channels
                .get(channel)
                .cloned()
                .unwrap_or_default();
            StageObservationChannelCoverage {
                channel: channel.name().into(),
                status: channel_status(&value),
                present_samples: value.present,
                absent_samples: value.absent,
                unavailable_samples: value.unavailable,
                not_sampled_samples: value.not_sampled,
                truncated_samples: value.truncated,
                unreported_samples: value.unreported,
            }
        })
        .collect();
    let fields = accumulator
        .fields
        .into_iter()
        .map(|(path, value)| {
            let sampled_records = value.sampled_records.len() as u64;
            let missing = accumulator.record_count.saturating_sub(sampled_records);
            let distinct = value.distinct_nonnull.len() as u64;
            let status = field_status(
                &path,
                sampled_records,
                accumulator.record_count,
                value.value_samples,
                value.null_samples,
                value.true_samples,
                distinct,
            );
            StageObservationFieldCoverage {
                path,
                status,
                sampled_records,
                missing_records: missing,
                value_samples: value.value_samples,
                null_samples: value.null_samples,
                true_samples: value.true_samples,
                distinct_nonnull_values: distinct,
            }
        })
        .collect();
    StageObservationCoverageCell {
        stage,
        probe,
        case_count: accumulator.case_count,
        record_count: accumulator.record_count,
        channels,
        fields,
    }
}

fn field_status(
    path: &str,
    sampled_records: u64,
    record_count: u64,
    value_samples: u64,
    null_samples: u64,
    true_samples: u64,
    distinct_nonnull: u64,
) -> ObservationCoverageStatus {
    if path.ends_with(".truncated") && true_samples > 0 {
        ObservationCoverageStatus::Truncated
    } else if sampled_records < record_count || (null_samples > 0 && distinct_nonnull > 0) {
        ObservationCoverageStatus::Ambiguous
    } else if null_samples == value_samples {
        ObservationCoverageStatus::Absent
    } else if distinct_nonnull > 1 {
        ObservationCoverageStatus::Varying
    } else {
        ObservationCoverageStatus::Present
    }
}

fn channel_status(value: &ChannelAccumulator) -> ObservationCoverageStatus {
    if value.truncated > 0 {
        return ObservationCoverageStatus::Truncated;
    }
    let categories = [
        value.present > 0,
        value.absent > 0,
        value.unavailable > 0,
        value.not_sampled > 0,
        value.unreported > 0,
    ]
    .into_iter()
    .filter(|present| *present)
    .count();
    if categories > 1 {
        ObservationCoverageStatus::Ambiguous
    } else if value.present > 0 {
        ObservationCoverageStatus::Present
    } else if value.absent > 0 {
        ObservationCoverageStatus::Absent
    } else {
        ObservationCoverageStatus::RequiresTypedExtension
    }
}

fn locate_trace_artifact(
    state_root: &Path,
    candidate_id: &str,
    attempt_number: u8,
    expected_digest: Digest,
) -> Result<Option<Vec<u8>>, StageObservationCoverageError> {
    let case_root = state_root
        .join("cases")
        .join(stage_survey_case_storage_id(candidate_id).to_string());
    if !case_root.is_dir() {
        return Ok(None);
    }
    let prefix = format!("attempt-{attempt_number:03}-run-");
    let mut paths = fs::read_dir(&case_root)
        .map_err(|error| StageObservationCoverageError::new(error.to_string()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.is_dir()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(&prefix))
        })
        .map(|path| path.join("observation.trace"))
        .filter(|path| path.is_file())
        .collect::<Vec<PathBuf>>();
    paths.sort();
    for path in paths {
        let bytes = fs::read(path)
            .map_err(|error| StageObservationCoverageError::new(error.to_string()))?;
        if Digest(Sha256::digest(&bytes).into()) == expected_digest {
            return Ok(Some(bytes));
        }
    }
    Ok(None)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StageObservationCoverageError(String);

impl StageObservationCoverageError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for StageObservationCoverageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for StageObservationCoverageError {}

#[cfg(test)]
mod tests {
    use super::*;
    use dusklight_trace::trace::{TraceRecord, TraceSelectedActors};

    #[test]
    fn field_and_channel_matrix_distinguishes_variation_absence_and_truncation() {
        let mut first = TraceRecord {
            stage_name: "D_MN01".into(),
            position: [1.0, 2.0, 3.0],
            selected_actors: Some(TraceSelectedActors {
                observed_count: 1,
                truncated: true,
                actors: Vec::new(),
            }),
            ..TraceRecord::default()
        };
        first
            .channel_status
            .insert(TraceChannel::Core, TraceChannelStatus::Present);
        first
            .channel_status
            .insert(TraceChannel::SelectedActors, TraceChannelStatus::Truncated);
        let mut second = first.clone();
        second.position[0] = 4.0;
        let trace = DecodedTrace {
            version: 5,
            boot: Default::default(),
            tick_rate_numerator: 30,
            tick_rate_denominator: 1,
            requested_channels: 0,
            capacity_exhausted: false,
            retention: None,
            channel_formats: BTreeMap::new(),
            records: vec![first, second],
        };
        let mut accumulator = CellAccumulator::default();
        accumulator.case_count = 1;
        accumulate_records(&mut accumulator, trace.records.iter()).unwrap();
        let cell = finish_cell("D_MN01".into(), StageSurveyProbeKind::Movement, accumulator);

        assert_eq!(
            cell.channels
                .iter()
                .find(|channel| channel.channel == "core")
                .unwrap()
                .status,
            ObservationCoverageStatus::Present
        );
        assert_eq!(
            cell.channels
                .iter()
                .find(|channel| channel.channel == "selected_actors")
                .unwrap()
                .status,
            ObservationCoverageStatus::Truncated
        );
        assert_eq!(
            cell.fields
                .iter()
                .find(|field| field.path == "position")
                .unwrap()
                .status,
            ObservationCoverageStatus::Varying
        );
        assert_eq!(
            cell.fields
                .iter()
                .find(|field| field.path == "scene_exit")
                .unwrap()
                .status,
            ObservationCoverageStatus::Absent
        );
        assert_eq!(
            cell.fields
                .iter()
                .find(|field| field.path == "selected_actors.truncated")
                .unwrap()
                .status,
            ObservationCoverageStatus::Truncated
        );
    }

    #[test]
    fn channel_mixture_is_ambiguous_and_unobserved_requires_an_extension() {
        let mixed = ChannelAccumulator {
            present: 1,
            absent: 1,
            ..ChannelAccumulator::default()
        };
        assert_eq!(channel_status(&mixed), ObservationCoverageStatus::Ambiguous);
        assert_eq!(
            channel_status(&ChannelAccumulator {
                not_sampled: 2,
                ..ChannelAccumulator::default()
            }),
            ObservationCoverageStatus::RequiresTypedExtension
        );
    }

    #[test]
    fn nested_arrays_surface_wildcard_fields_without_inflating_record_coverage() {
        let mut accumulator = CellAccumulator {
            case_count: 1,
            record_count: 2,
            ..CellAccumulator::default()
        };
        flatten_value(
            "actors",
            &serde_json::json!([
                {"identity": 7, "health": 3},
                {"identity": 8, "health": 4}
            ]),
            0,
            &mut accumulator.fields,
        )
        .unwrap();
        flatten_value(
            "actors",
            &serde_json::json!([{"identity": 7, "health": 2}]),
            1,
            &mut accumulator.fields,
        )
        .unwrap();

        let cell = finish_cell("D_MN01".into(), StageSurveyProbeKind::Movement, accumulator);
        let health = cell
            .fields
            .iter()
            .find(|field| field.path == "actors[].health")
            .unwrap();
        assert_eq!(health.status, ObservationCoverageStatus::Varying);
        assert_eq!(health.sampled_records, 2);
        assert_eq!(health.missing_records, 0);
        assert_eq!(health.value_samples, 3);
        assert_eq!(health.null_samples, 0);
        assert_eq!(health.distinct_nonnull_values, 3);
    }

    #[test]
    fn cross_stage_trace_is_split_into_actual_stage_cells() {
        let trace = DecodedTrace {
            version: 5,
            boot: Default::default(),
            tick_rate_numerator: 30,
            tick_rate_denominator: 1,
            requested_channels: 0,
            capacity_exhausted: false,
            retention: None,
            channel_formats: BTreeMap::new(),
            records: vec![
                TraceRecord {
                    stage_name: "F_SP103".into(),
                    ..TraceRecord::default()
                },
                TraceRecord {
                    stage_name: "F_SP00".into(),
                    ..TraceRecord::default()
                },
                TraceRecord {
                    stage_name: "F_SP00".into(),
                    ..TraceRecord::default()
                },
            ],
        };
        let mut cells = BTreeMap::new();
        let slices =
            accumulate_trace_cells(&mut cells, StageSurveyProbeKind::ContactSweep, &trace).unwrap();

        assert_eq!(
            slices,
            vec![
                StageObservationCoverageStageRecords {
                    stage: "F_SP00".into(),
                    record_count: 2,
                },
                StageObservationCoverageStageRecords {
                    stage: "F_SP103".into(),
                    record_count: 1,
                },
            ]
        );
        assert_eq!(
            cells[&("F_SP00".into(), StageSurveyProbeKind::ContactSweep)].record_count,
            2
        );
        assert_eq!(
            cells[&("F_SP103".into(), StageSurveyProbeKind::ContactSweep)].record_count,
            1
        );
    }
}
