//! Exact raw/decoded comparison of extracted orig bundles and language variants.

use crate::artifact::Digest;
use crate::orig_discovery::{
    ExtractedOrigBundle, ExtractedOrigIgnoredArchive, ExtractedOrigMessageArchive,
    ExtractedOrigStageArchive,
};
use crate::{PlannerContractError, canonical_json, validate_label};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const ORIG_BUNDLE_DIFF_SCHEMA: &str = "dusklight.route-planner.orig-bundle-diff/v2";

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OrigDiffDomain {
    StageArchives,
    MessageFlowArchives,
    IgnoredArchives,
    ExecutableCode,
    RuntimeLanguageSelection,
    ActorSemantics,
    CutsceneSemantics,
    RuleSemantics,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OrigDomainCoverageStatus {
    Compared,
    EmptyOnBoth,
    UncoveredOnLeft,
    UncoveredOnRight,
    NotRepresented,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OrigDomainCoverage {
    pub domain: OrigDiffDomain,
    pub status: OrigDomainCoverageStatus,
    pub left_records: Option<u32>,
    pub right_records: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LocaleComparison {
    pub left: String,
    pub right: String,
    pub left_group_count: u32,
    pub right_group_count: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OrigRecordDiffStatus {
    Identical,
    RawChangedSemanticEqual,
    SemanticChanged,
    UncoveredOnLeft,
    UncoveredOnRight,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OrigRecordDiff {
    pub key: String,
    pub status: OrigRecordDiffStatus,
    pub left_relative_path: Option<String>,
    pub right_relative_path: Option<String>,
    pub left_raw_sha256: Option<Digest>,
    pub right_raw_sha256: Option<Digest>,
    pub left_semantic_sha256: Option<Digest>,
    pub right_semantic_sha256: Option<Digest>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OrigDiffSummary {
    pub identical: u32,
    pub raw_changed_semantic_equal: u32,
    pub semantic_changed: u32,
    pub uncovered_on_left: u32,
    pub uncovered_on_right: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OrigBundleDiff {
    pub schema: String,
    pub left_bundle_sha256: Digest,
    pub right_bundle_sha256: Digest,
    pub left_content_sha256: Digest,
    pub right_content_sha256: Digest,
    pub locale_comparison: Option<LocaleComparison>,
    pub stages: Vec<OrigRecordDiff>,
    pub message_flows: Vec<OrigRecordDiff>,
    pub ignored_archives: Vec<OrigRecordDiff>,
    pub domain_coverage: Vec<OrigDomainCoverage>,
    pub summary: OrigDiffSummary,
}

pub fn compare_orig_bundles(
    left: &ExtractedOrigBundle,
    right: &ExtractedOrigBundle,
    locale_pair: Option<(&str, &str)>,
) -> Result<OrigBundleDiff, PlannerContractError> {
    left.validate()?;
    right.validate()?;
    let selected_messages = locale_pair
        .map(|(left_locale, right_locale)| {
            Ok::<_, PlannerContractError>((
                selected_message_records(&left.message_flows, left_locale)?,
                selected_message_records(&right.message_flows, right_locale)?,
            ))
        })
        .transpose()?;
    let locale_comparison = locale_pair
        .map(|(left_locale, right_locale)| {
            validate_locale("orig_diff.left_locale", left_locale)?;
            validate_locale("orig_diff.right_locale", right_locale)?;
            let (left_records, right_records) = selected_messages
                .as_ref()
                .expect("locale records exist when a locale pair exists");
            Ok::<LocaleComparison, PlannerContractError>(LocaleComparison {
                left: left_locale.into(),
                right: right_locale.into(),
                left_group_count: checked_count("orig_diff.left_locale", left_records.len())?,
                right_group_count: checked_count("orig_diff.right_locale", right_records.len())?,
            })
        })
        .transpose()?;
    let stages = compare_records(stage_records(&left.stages)?, stage_records(&right.stages)?)?;
    let message_flows = match selected_messages {
        Some((left_records, right_records)) => compare_records(left_records, right_records)?,
        None => compare_records(
            all_message_records(&left.message_flows)?,
            all_message_records(&right.message_flows)?,
        )?,
    };
    let ignored_archives = match locale_pair {
        Some((left_locale, right_locale)) => compare_records(
            selected_ignored_archive_records(&left.ignored_archives, left_locale)?,
            selected_ignored_archive_records(&right.ignored_archives, right_locale)?,
        )?,
        None => compare_records(
            ignored_archive_records(&left.ignored_archives)?,
            ignored_archive_records(&right.ignored_archives)?,
        )?,
    };
    let summary = summarize(stages.iter().chain(&message_flows).chain(&ignored_archives));
    let domain_coverage = domain_coverage(&stages, &message_flows, &ignored_archives)?;
    let diff = OrigBundleDiff {
        schema: ORIG_BUNDLE_DIFF_SCHEMA.into(),
        left_bundle_sha256: left.digest()?,
        right_bundle_sha256: right.digest()?,
        left_content_sha256: left.content.digest()?,
        right_content_sha256: right.content.digest()?,
        locale_comparison,
        stages,
        message_flows,
        ignored_archives,
        domain_coverage,
        summary,
    };
    diff.validate()?;
    Ok(diff)
}

impl OrigBundleDiff {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != ORIG_BUNDLE_DIFF_SCHEMA
            || self.left_bundle_sha256 == Digest::ZERO
            || self.right_bundle_sha256 == Digest::ZERO
            || self.left_content_sha256 == Digest::ZERO
            || self.right_content_sha256 == Digest::ZERO
        {
            return Err(PlannerContractError::new(
                "orig_diff",
                "has an unsupported schema or zero input digest",
            ));
        }
        if let Some(pair) = &self.locale_comparison {
            validate_locale("orig_diff.left_locale", &pair.left)?;
            validate_locale("orig_diff.right_locale", &pair.right)?;
            let left_count = side_count(&self.message_flows, true)?;
            let right_count = side_count(&self.message_flows, false)?;
            if pair.left_group_count != left_count || pair.right_group_count != right_count {
                return Err(PlannerContractError::new(
                    "orig_diff.locale_comparison",
                    "group counts disagree with compared message-flow coverage",
                ));
            }
        }
        validate_diff_records("orig_diff.stages", &self.stages)?;
        validate_diff_records("orig_diff.message_flows", &self.message_flows)?;
        validate_diff_records("orig_diff.ignored_archives", &self.ignored_archives)?;
        if self.domain_coverage
            != domain_coverage(&self.stages, &self.message_flows, &self.ignored_archives)?
        {
            return Err(PlannerContractError::new(
                "orig_diff.domain_coverage",
                "does not match represented and absent comparison domains",
            ));
        }
        if self.summary
            != summarize(
                self.stages
                    .iter()
                    .chain(&self.message_flows)
                    .chain(&self.ignored_archives),
            )
        {
            return Err(PlannerContractError::new(
                "orig_diff.summary",
                "does not match the record statuses",
            ));
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PlannerContractError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, PlannerContractError> {
        let diff: Self = serde_json::from_slice(bytes)?;
        diff.validate()?;
        if diff.canonical_bytes()? != bytes {
            return Err(PlannerContractError::new(
                "orig_diff",
                "is not canonical JSON",
            ));
        }
        Ok(diff)
    }
}

#[derive(Clone)]
struct ComparableRecord {
    relative_path: String,
    raw_sha256: Digest,
    semantic_sha256: Digest,
}

fn stage_records(
    stages: &[ExtractedOrigStageArchive],
) -> Result<BTreeMap<String, ComparableRecord>, PlannerContractError> {
    stages
        .iter()
        .map(|record| {
            Ok((
                record.relative_path.clone(),
                ComparableRecord {
                    relative_path: record.relative_path.clone(),
                    raw_sha256: record.archive_sha256,
                    semantic_sha256: semantic_digest(&record.stage)?,
                },
            ))
        })
        .collect()
}

fn all_message_records(
    messages: &[ExtractedOrigMessageArchive],
) -> Result<BTreeMap<String, ComparableRecord>, PlannerContractError> {
    let mut output = BTreeMap::new();
    for record in messages {
        let key = format!(
            "locale/{}/group/{:05}",
            record.locale_bundle, record.message_group
        );
        if output.insert(key, comparable_message(record)?).is_some() {
            return Err(PlannerContractError::new(
                "orig_diff.message_flows",
                "contains duplicate locale/group identities",
            ));
        }
    }
    Ok(output)
}

fn selected_message_records(
    messages: &[ExtractedOrigMessageArchive],
    locale: &str,
) -> Result<BTreeMap<String, ComparableRecord>, PlannerContractError> {
    let mut output = BTreeMap::new();
    for record in messages
        .iter()
        .filter(|record| record.locale_bundle == locale)
    {
        let key = format!("group/{:05}", record.message_group);
        if output.insert(key, comparable_message(record)?).is_some() {
            return Err(PlannerContractError::new(
                "orig_diff.message_flows",
                "contains duplicate groups in one locale",
            ));
        }
    }
    Ok(output)
}

fn comparable_message(
    record: &ExtractedOrigMessageArchive,
) -> Result<ComparableRecord, PlannerContractError> {
    Ok(ComparableRecord {
        relative_path: record.relative_path.clone(),
        raw_sha256: record.archive_sha256,
        semantic_sha256: semantic_digest(&record.flow)?,
    })
}

fn ignored_archive_records(
    archives: &[ExtractedOrigIgnoredArchive],
) -> Result<BTreeMap<String, ComparableRecord>, PlannerContractError> {
    archives
        .iter()
        .map(|record| {
            Ok((
                record.relative_path.clone(),
                ComparableRecord {
                    relative_path: record.relative_path.clone(),
                    raw_sha256: record.archive_sha256,
                    semantic_sha256: semantic_digest(&(&record.reason, &record.resource_names))?,
                },
            ))
        })
        .collect()
}

fn selected_ignored_archive_records(
    archives: &[ExtractedOrigIgnoredArchive],
    locale: &str,
) -> Result<BTreeMap<String, ComparableRecord>, PlannerContractError> {
    let prefix = format!("files/res/Msg{locale}/");
    let mut output = BTreeMap::new();
    for record in archives {
        let Some(key) = record.relative_path.strip_prefix(&prefix) else {
            continue;
        };
        let comparable = ComparableRecord {
            relative_path: record.relative_path.clone(),
            raw_sha256: record.archive_sha256,
            semantic_sha256: semantic_digest(&(&record.reason, &record.resource_names))?,
        };
        if output.insert(key.to_owned(), comparable).is_some() {
            return Err(PlannerContractError::new(
                "orig_diff.ignored_archives",
                "contains duplicate archive names in one locale",
            ));
        }
    }
    Ok(output)
}

fn compare_records(
    left: BTreeMap<String, ComparableRecord>,
    right: BTreeMap<String, ComparableRecord>,
) -> Result<Vec<OrigRecordDiff>, PlannerContractError> {
    let keys = left
        .keys()
        .chain(right.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    keys.into_iter()
        .map(|key| {
            let left = left.get(&key);
            let right = right.get(&key);
            let status = match (left, right) {
                (Some(left), Some(right))
                    if left.raw_sha256 == right.raw_sha256
                        && left.semantic_sha256 == right.semantic_sha256 =>
                {
                    OrigRecordDiffStatus::Identical
                }
                (Some(left), Some(right)) if left.semantic_sha256 == right.semantic_sha256 => {
                    OrigRecordDiffStatus::RawChangedSemanticEqual
                }
                (Some(_), Some(_)) => OrigRecordDiffStatus::SemanticChanged,
                (None, Some(_)) => OrigRecordDiffStatus::UncoveredOnLeft,
                (Some(_), None) => OrigRecordDiffStatus::UncoveredOnRight,
                (None, None) => unreachable!(),
            };
            Ok(OrigRecordDiff {
                key,
                status,
                left_relative_path: left.map(|record| record.relative_path.clone()),
                right_relative_path: right.map(|record| record.relative_path.clone()),
                left_raw_sha256: left.map(|record| record.raw_sha256),
                right_raw_sha256: right.map(|record| record.raw_sha256),
                left_semantic_sha256: left.map(|record| record.semantic_sha256),
                right_semantic_sha256: right.map(|record| record.semantic_sha256),
            })
        })
        .collect()
}

fn semantic_digest(value: &impl Serialize) -> Result<Digest, PlannerContractError> {
    Ok(Digest(Sha256::digest(canonical_json(value)?).into()))
}

fn domain_coverage(
    stages: &[OrigRecordDiff],
    message_flows: &[OrigRecordDiff],
    ignored_archives: &[OrigRecordDiff],
) -> Result<Vec<OrigDomainCoverage>, PlannerContractError> {
    let compared = |domain, records: &[OrigRecordDiff]| -> Result<_, PlannerContractError> {
        let left_records = side_count(records, true)?;
        let right_records = side_count(records, false)?;
        let status = match (left_records, right_records) {
            (0, 0) => OrigDomainCoverageStatus::EmptyOnBoth,
            (0, _) => OrigDomainCoverageStatus::UncoveredOnLeft,
            (_, 0) => OrigDomainCoverageStatus::UncoveredOnRight,
            _ => OrigDomainCoverageStatus::Compared,
        };
        Ok(OrigDomainCoverage {
            domain,
            status,
            left_records: Some(left_records),
            right_records: Some(right_records),
        })
    };
    let mut output = vec![
        compared(OrigDiffDomain::StageArchives, stages)?,
        compared(OrigDiffDomain::MessageFlowArchives, message_flows)?,
        compared(OrigDiffDomain::IgnoredArchives, ignored_archives)?,
    ];
    output.extend(
        [
            OrigDiffDomain::ExecutableCode,
            OrigDiffDomain::RuntimeLanguageSelection,
            OrigDiffDomain::ActorSemantics,
            OrigDiffDomain::CutsceneSemantics,
            OrigDiffDomain::RuleSemantics,
        ]
        .into_iter()
        .map(|domain| OrigDomainCoverage {
            domain,
            status: OrigDomainCoverageStatus::NotRepresented,
            left_records: None,
            right_records: None,
        }),
    );
    output.sort_by_key(|row| row.domain);
    Ok(output)
}

fn summarize<'a>(records: impl Iterator<Item = &'a OrigRecordDiff>) -> OrigDiffSummary {
    let mut summary = OrigDiffSummary {
        identical: 0,
        raw_changed_semantic_equal: 0,
        semantic_changed: 0,
        uncovered_on_left: 0,
        uncovered_on_right: 0,
    };
    for record in records {
        let target = match record.status {
            OrigRecordDiffStatus::Identical => &mut summary.identical,
            OrigRecordDiffStatus::RawChangedSemanticEqual => {
                &mut summary.raw_changed_semantic_equal
            }
            OrigRecordDiffStatus::SemanticChanged => &mut summary.semantic_changed,
            OrigRecordDiffStatus::UncoveredOnLeft => &mut summary.uncovered_on_left,
            OrigRecordDiffStatus::UncoveredOnRight => &mut summary.uncovered_on_right,
        };
        *target += 1;
    }
    summary
}

fn side_count(records: &[OrigRecordDiff], left: bool) -> Result<u32, PlannerContractError> {
    checked_count(
        "orig_diff.locale_comparison",
        records
            .iter()
            .filter(|record| {
                if left {
                    record.left_relative_path.is_some()
                } else {
                    record.right_relative_path.is_some()
                }
            })
            .count(),
    )
}

fn checked_count(field: &str, count: usize) -> Result<u32, PlannerContractError> {
    u32::try_from(count).map_err(|_| PlannerContractError::new(field, "record count exceeds u32"))
}

fn validate_diff_records(
    field: &str,
    records: &[OrigRecordDiff],
) -> Result<(), PlannerContractError> {
    let mut prior = None;
    for record in records {
        validate_label(field, &record.key)?;
        if prior.is_some_and(|key: &str| key >= record.key.as_str()) {
            return Err(PlannerContractError::new(
                field,
                "must be unique and sorted by comparison key",
            ));
        }
        prior = Some(record.key.as_str());
        let left_present = record.left_relative_path.is_some()
            && record.left_raw_sha256.is_some()
            && record.left_semantic_sha256.is_some();
        let right_present = record.right_relative_path.is_some()
            && record.right_raw_sha256.is_some()
            && record.right_semantic_sha256.is_some();
        if record.left_relative_path.is_some() != record.left_raw_sha256.is_some()
            || record.left_raw_sha256.is_some() != record.left_semantic_sha256.is_some()
            || record.right_relative_path.is_some() != record.right_raw_sha256.is_some()
            || record.right_raw_sha256.is_some() != record.right_semantic_sha256.is_some()
            || record.left_raw_sha256 == Some(Digest::ZERO)
            || record.right_raw_sha256 == Some(Digest::ZERO)
            || record.left_semantic_sha256 == Some(Digest::ZERO)
            || record.right_semantic_sha256 == Some(Digest::ZERO)
        {
            return Err(PlannerContractError::new(
                field,
                "has incomplete or zero side identity",
            ));
        }
        let valid_presence = match record.status {
            OrigRecordDiffStatus::Identical
            | OrigRecordDiffStatus::RawChangedSemanticEqual
            | OrigRecordDiffStatus::SemanticChanged => left_present && right_present,
            OrigRecordDiffStatus::UncoveredOnLeft => !left_present && right_present,
            OrigRecordDiffStatus::UncoveredOnRight => left_present && !right_present,
        };
        if !valid_presence {
            return Err(PlannerContractError::new(
                field,
                "status disagrees with side coverage",
            ));
        }
        if left_present && right_present {
            let raw_equal = record.left_raw_sha256 == record.right_raw_sha256;
            let semantic_equal = record.left_semantic_sha256 == record.right_semantic_sha256;
            let valid_status = match record.status {
                OrigRecordDiffStatus::Identical => raw_equal && semantic_equal,
                OrigRecordDiffStatus::RawChangedSemanticEqual => !raw_equal && semantic_equal,
                OrigRecordDiffStatus::SemanticChanged => !semantic_equal,
                OrigRecordDiffStatus::UncoveredOnLeft | OrigRecordDiffStatus::UncoveredOnRight => {
                    false
                }
            };
            if !valid_status {
                return Err(PlannerContractError::new(
                    field,
                    "status disagrees with raw or semantic digests",
                ));
            }
        }
    }
    Ok(())
}

fn validate_locale(field: &str, locale: &str) -> Result<(), PlannerContractError> {
    if locale.is_empty()
        || locale.len() > 16
        || !locale.bytes().all(|byte| byte.is_ascii_alphanumeric())
    {
        return Err(PlannerContractError::new(
            field,
            "must contain 1-16 ASCII letters or digits",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orig_discovery::IgnoredOrigArchiveReason;
    use crate::orig_extraction::ExtractedMessageFlow;

    fn message(
        path: &str,
        locale: &str,
        group: u16,
        raw: u8,
        node_count: u16,
    ) -> ExtractedOrigMessageArchive {
        ExtractedOrigMessageArchive {
            relative_path: path.into(),
            archive_sha256: Digest([raw; 32]),
            locale_bundle: locale.into(),
            message_group: group,
            resource_name: format!("zel_{group:02}.bmg"),
            resource_sha256: Digest([raw.wrapping_add(1); 32]),
            flow: ExtractedMessageFlow {
                header_declared_size: 64,
                resource_size: 64,
                node_count,
                branch_target_count: 0,
                labels: Vec::new(),
                nodes: Vec::new(),
                branch_targets: Vec::new(),
                temporary_flag_accesses: Vec::new(),
                persistent_flag_accesses: Vec::new(),
                switch_accesses: Vec::new(),
            },
        }
    }

    fn compare_messages(
        left: Vec<ExtractedOrigMessageArchive>,
        right: Vec<ExtractedOrigMessageArchive>,
        pair: Option<(&str, &str)>,
    ) -> Vec<OrigRecordDiff> {
        let compare = |records: &[ExtractedOrigMessageArchive], locale: Option<&str>| match locale {
            Some(locale) => selected_message_records(records, locale).unwrap(),
            None => all_message_records(records).unwrap(),
        };
        compare_records(
            compare(&left, pair.map(|value| value.0)),
            compare(&right, pair.map(|value| value.1)),
        )
        .unwrap()
    }

    #[test]
    fn distinguishes_raw_semantic_and_coverage_differences() {
        let left = vec![
            message("files/res/Msgfr/bmgres1.arc", "fr", 1, 1, 2),
            message("files/res/Msgfr/bmgres2.arc", "fr", 2, 2, 3),
            message("files/res/Msgfr/bmgres3.arc", "fr", 3, 3, 4),
        ];
        let right = vec![
            message("files/res/Msgde/bmgres1.arc", "de", 1, 1, 2),
            message("files/res/Msgde/bmgres2.arc", "de", 2, 9, 3),
            message("files/res/Msgde/bmgres3.arc", "de", 3, 8, 9),
            message("files/res/Msgde/bmgres4.arc", "de", 4, 7, 1),
        ];
        let records = compare_messages(left, right, Some(("fr", "de")));
        assert_eq!(records[0].status, OrigRecordDiffStatus::Identical);
        assert_eq!(
            records[1].status,
            OrigRecordDiffStatus::RawChangedSemanticEqual
        );
        assert_eq!(records[2].status, OrigRecordDiffStatus::SemanticChanged);
        assert_eq!(records[3].status, OrigRecordDiffStatus::UncoveredOnLeft);
        assert_eq!(side_count(&records, true).unwrap(), 3);
        assert_eq!(side_count(&records, false).unwrap(), 4);
    }

    #[test]
    fn ignored_candidates_remain_comparable_instead_of_disappearing() {
        let ignored = |locale: &str, raw| ExtractedOrigIgnoredArchive {
            relative_path: format!("files/res/Msg{locale}/bmgres99.arc"),
            archive_sha256: Digest([raw; 32]),
            reason: IgnoredOrigArchiveReason::NoMessageFlowResource,
            resource_names: vec!["placeholder.bin".into()],
        };
        let records = compare_records(
            selected_ignored_archive_records(&[ignored("fr", 1)], "fr").unwrap(),
            selected_ignored_archive_records(&[ignored("de", 2)], "de").unwrap(),
        )
        .unwrap();
        assert_eq!(
            records[0].status,
            OrigRecordDiffStatus::RawChangedSemanticEqual
        );
    }

    #[test]
    fn coverage_never_implies_equivalence_for_unrepresented_domains() {
        let records = compare_messages(
            vec![message("files/res/Msgfr/bmgres1.arc", "fr", 1, 1, 2)],
            Vec::new(),
            Some(("fr", "de")),
        );
        let coverage = domain_coverage(&[], &records, &[]).unwrap();
        let messages = coverage
            .iter()
            .find(|row| row.domain == OrigDiffDomain::MessageFlowArchives)
            .unwrap();
        assert_eq!(messages.status, OrigDomainCoverageStatus::UncoveredOnRight);
        for domain in [
            OrigDiffDomain::ExecutableCode,
            OrigDiffDomain::RuntimeLanguageSelection,
            OrigDiffDomain::ActorSemantics,
            OrigDiffDomain::CutsceneSemantics,
            OrigDiffDomain::RuleSemantics,
        ] {
            let row = coverage.iter().find(|row| row.domain == domain).unwrap();
            assert_eq!(row.status, OrigDomainCoverageStatus::NotRepresented);
            assert_eq!((row.left_records, row.right_records), (None, None));
        }
    }

    #[test]
    fn bundle_decoder_and_diff_artifact_fail_closed() {
        // Exercise the public decoders without fabricating a large valid bundle;
        // unknown fields and noncanonical bytes must fail before comparison.
        assert!(ExtractedOrigBundle::decode_canonical(b"{}\n").is_err());
        assert!(OrigBundleDiff::decode_canonical(b"{}\n").is_err());
    }
}
