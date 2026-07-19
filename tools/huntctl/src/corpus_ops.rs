//! Schema-aware transition corpus lifecycle operations.

use crate::artifact::Digest;
use crate::dataset::{DATASET_SOURCE_SCHEMA_V1, DatasetSourceDescriptor};
use crate::episode::{
    EPISODE_CONTEXT_SCHEMA_V1, EpisodeContext, EpisodeManifest, EpisodeManifestBuild,
};
use crate::observation_view::MOVEMENT_STATE_V2_ID;
use crate::offline_rl::{
    ExploratoryExtractConfig, extract_exploratory_from_bytes, extract_exploratory_v2_from_bytes,
};
use crate::transition_corpus::{MAX_TRANSITIONS, Transition, TransitionCorpus};
use crate::transition_evidence::ExactActionEvidence;
use arrow_array::builder::{
    BooleanBuilder, FixedSizeListBuilder, Float32Builder, Int16Builder, ListBuilder, StringBuilder,
    UInt8Builder, UInt16Builder, UInt32Builder, UInt64Builder,
};
use arrow_array::{ArrayRef, RecordBatch};
use arrow_ipc::writer::FileWriter;
use arrow_schema::{DataType, Field, Schema};
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const CORPUS_OPERATION_REPORT_SCHEMA_V1: &str = "dusklight-corpus-operation/v1";

#[derive(Clone, Debug, Serialize)]
pub struct CorpusQueryRow {
    pub corpus: PathBuf,
    pub transition: usize,
    pub source_sha256: Digest,
    pub action: u32,
    pub duration_ticks: u32,
    pub reward: f32,
    pub terminal: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct CorpusComparison {
    pub schema: &'static str,
    pub compatible: bool,
    pub left_sha256: Digest,
    pub right_sha256: Digest,
    pub left_transitions: usize,
    pub right_transitions: usize,
    pub shared_exact_transitions: usize,
    pub left_only_transitions: usize,
    pub right_only_transitions: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct CorpusWriteReport {
    pub schema: &'static str,
    pub inputs: Vec<PathBuf>,
    pub outputs: Vec<PathBuf>,
    pub input_transitions: usize,
    pub output_transitions: usize,
    pub duplicates_removed: usize,
    pub content_sha256: Vec<Digest>,
}

#[derive(Clone, Debug, Serialize)]
pub struct QuarantineReport {
    pub schema: &'static str,
    pub dry_run: bool,
    pub valid: Vec<PathBuf>,
    pub quarantined: Vec<QuarantinedCorpus>,
}

#[derive(Clone, Debug, Serialize)]
pub struct QuarantinedCorpus {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct AnalysisExportReport {
    pub schema: &'static str,
    pub format: &'static str,
    pub replay_authority: bool,
    pub inputs: Vec<PathBuf>,
    pub input_sha256: Vec<Digest>,
    pub output: PathBuf,
    pub output_sha256: Digest,
    pub rows: usize,
    pub feature_count: u32,
    pub feature_schema: Digest,
    pub action_schema: Digest,
    pub manifest: PathBuf,
}

pub fn query(
    paths: &[PathBuf],
    action: Option<u32>,
    terminal: Option<bool>,
    minimum_reward: Option<f32>,
    limit: usize,
) -> Result<Vec<CorpusQueryRow>, CorpusOpsError> {
    if paths.is_empty() || limit == 0 || limit > 100_000 {
        return Err(CorpusOpsError::new("invalid corpus query inputs or limit"));
    }
    let mut rows = Vec::new();
    for path in paths {
        let corpus = TransitionCorpus::read_zstd_file(path)?;
        for (index, transition) in corpus.transitions.iter().enumerate() {
            if action.is_some_and(|value| transition.action.action_id != value)
                || terminal.is_some_and(|value| transition.terminal != value)
                || minimum_reward.is_some_and(|value| transition.reward < value)
            {
                continue;
            }
            rows.push(CorpusQueryRow {
                corpus: path.clone(),
                transition: index,
                source_sha256: transition.source.digest,
                action: transition.action.action_id,
                duration_ticks: transition.duration_ticks,
                reward: transition.reward,
                terminal: transition.terminal,
            });
            if rows.len() == limit {
                return Ok(rows);
            }
        }
    }
    Ok(rows)
}

pub fn compare(left: &Path, right: &Path) -> Result<CorpusComparison, CorpusOpsError> {
    let left = TransitionCorpus::read_zstd_file(left)?;
    let right = TransitionCorpus::read_zstd_file(right)?;
    let compatible = left.feature_schema == right.feature_schema
        && left.action_schema == right.action_schema
        && left.feature_count == right.feature_count;
    let left_ids: BTreeSet<_> = left.transitions.iter().map(transition_digest).collect();
    let right_ids: BTreeSet<_> = right.transitions.iter().map(transition_digest).collect();
    let shared = left_ids.intersection(&right_ids).count();
    Ok(CorpusComparison {
        schema: CORPUS_OPERATION_REPORT_SCHEMA_V1,
        compatible,
        left_sha256: left.content_digest()?,
        right_sha256: right.content_digest()?,
        left_transitions: left.transitions.len(),
        right_transitions: right.transitions.len(),
        shared_exact_transitions: shared,
        left_only_transitions: left_ids.len() - shared,
        right_only_transitions: right_ids.len() - shared,
    })
}

pub fn merge(
    inputs: &[PathBuf],
    output: &Path,
    compression_level: i32,
) -> Result<CorpusWriteReport, CorpusOpsError> {
    prepare_new_output(output)?;
    let corpora = load_compatible(inputs)?;
    let input_transitions = corpora.iter().map(|corpus| corpus.transitions.len()).sum();
    let feature_schema = corpora[0].feature_schema;
    let action_schema = corpora[0].action_schema;
    let feature_count = corpora[0].feature_count;
    let mut seen = BTreeSet::new();
    let mut transitions = Vec::new();
    for transition in corpora.into_iter().flat_map(|corpus| corpus.transitions) {
        if seen.insert(transition_digest(&transition)) {
            transitions.push(transition);
        }
    }
    let corpus = TransitionCorpus::new(feature_schema, action_schema, feature_count, transitions)?;
    let digest = corpus.write_zstd_file(output, compression_level)?;
    Ok(CorpusWriteReport {
        schema: CORPUS_OPERATION_REPORT_SCHEMA_V1,
        inputs: inputs.to_vec(),
        outputs: vec![output.into()],
        input_transitions,
        output_transitions: corpus.transitions.len(),
        duplicates_removed: input_transitions - corpus.transitions.len(),
        content_sha256: vec![digest],
    })
}

pub fn shard(
    input: &Path,
    output_directory: &Path,
    maximum_transitions: usize,
    compression_level: i32,
) -> Result<CorpusWriteReport, CorpusOpsError> {
    if maximum_transitions == 0 {
        return Err(CorpusOpsError::new("shard size must be nonzero"));
    }
    let corpus = TransitionCorpus::read_zstd_file(input)?;
    fs::create_dir_all(output_directory)?;
    let mut outputs = Vec::new();
    let mut digests = Vec::new();
    for (index, transitions) in corpus.transitions.chunks(maximum_transitions).enumerate() {
        let shard = TransitionCorpus::new(
            corpus.feature_schema,
            corpus.action_schema,
            corpus.feature_count,
            transitions.to_vec(),
        )?;
        let path = output_directory.join(format!("shard-{index:05}.dtcz"));
        prepare_new_output(&path)?;
        digests.push(shard.write_zstd_file(&path, compression_level)?);
        outputs.push(path);
    }
    Ok(CorpusWriteReport {
        schema: CORPUS_OPERATION_REPORT_SCHEMA_V1,
        inputs: vec![input.into()],
        outputs,
        input_transitions: corpus.transitions.len(),
        output_transitions: corpus.transitions.len(),
        duplicates_removed: 0,
        content_sha256: digests,
    })
}

pub fn refeature(
    descriptor_path: &Path,
    output: &Path,
    view: &str,
) -> Result<DatasetSourceDescriptor, CorpusOpsError> {
    prepare_new_output(output)?;
    let descriptor: DatasetSourceDescriptor = serde_json::from_slice(&fs::read(descriptor_path)?)?;
    let source = descriptor.load(descriptor_path.parent().unwrap_or(Path::new(".")))?;
    let trace_bytes = fs::read(&source.gameplay_trace_path)?;
    let tape_bytes = fs::read(&source.absolute_tape_path)?;
    let frames: Vec<_> = source
        .evidence
        .transitions
        .iter()
        .map(|transition| match &transition.action {
            ExactActionEvidence::PadFrame { tape_frame, .. } => Ok(*tape_frame),
            ExactActionEvidence::Option { .. } => Err(CorpusOpsError::new(
                "re-featurizing typed option episodes is not implemented",
            )),
        })
        .collect::<Result<_, _>>()?;
    let start = *frames
        .first()
        .ok_or_else(|| CorpusOpsError::new("episode is empty"))?;
    let end = *frames.last().unwrap();
    let first = source.corpus.transitions.first().unwrap();
    let last = source.corpus.transitions.last().unwrap();
    let config = ExploratoryExtractConfig {
        episode_digest: source.episode.episode_sha256,
        start_tape_frame: start,
        end_tape_frame: end,
        start_reference: Some(first.source),
        terminal_reference: last.terminal.then_some(last.next),
        end_is_terminal: last.terminal,
    };
    let corpus = match view {
        "movement-state/v1" => extract_exploratory_from_bytes(&trace_bytes, &tape_bytes, config)?,
        MOVEMENT_STATE_V2_ID => {
            extract_exploratory_v2_from_bytes(&trace_bytes, &tape_bytes, config)?
        }
        _ => return Err(CorpusOpsError::new("unknown re-feature observation view")),
    };
    corpus.write_zstd_file(output, 3)?;
    let mut evidence = source.evidence.clone();
    evidence.corpus_sha256 = corpus.content_digest()?;
    evidence
        .validate(&corpus)
        .map_err(|error| CorpusOpsError::new(error.to_string()))?;
    let evidence_path = PathBuf::from(format!("{}.evidence.json", output.display()));
    let evidence_bytes = serde_json::to_vec_pretty(&evidence)?;
    fs::write(&evidence_path, &evidence_bytes)?;
    let context = EpisodeContext {
        schema: EPISODE_CONTEXT_SCHEMA_V1.into(),
        run_identity: source.episode.run_identity.clone(),
        run_build: source.episode.run_build.clone(),
        objective: source.episode.objective.clone(),
        producer: source.episode.producer.clone(),
        seed: source.episode.seed.clone(),
        worker_id: source.episode.worker_id.clone(),
        lineage: source.episode.lineage.clone(),
        outcome: source.episode.outcome.clone(),
    };
    let episode = EpisodeManifest::build(EpisodeManifestBuild {
        context: &context,
        boot: &source.tape.boot,
        corpus: &corpus,
        query_view_id: view,
        tape_sha256: source.episode.artifacts.absolute_tape_sha256,
        trace_sha256: source.episode.artifacts.gameplay_trace_sha256,
        transition_evidence_sha256: Digest(Sha256::digest(&evidence_bytes).into()),
    })?;
    let episode_path = PathBuf::from(format!("{}.episode.json", output.display()));
    fs::write(&episode_path, serde_json::to_vec_pretty(&episode)?)?;
    let result = DatasetSourceDescriptor {
        schema: DATASET_SOURCE_SCHEMA_V1.into(),
        source_id: episode.episode_sha256.to_string(),
        episode_manifest: fs::canonicalize(episode_path)?,
        transition_corpus: fs::canonicalize(output)?,
        absolute_tape: fs::canonicalize(source.absolute_tape_path)?,
        transition_evidence: fs::canonicalize(evidence_path)?,
        gameplay_trace: fs::canonicalize(source.gameplay_trace_path)?,
        route_family: descriptor.route_family,
        screenshot_sha256: descriptor.screenshot_sha256,
        checkpoint_sha256: descriptor.checkpoint_sha256,
    };
    fs::write(
        format!("{}.dataset-source.json", output.display()),
        serde_json::to_vec_pretty(&result)?,
    )?;
    Ok(result)
}

pub fn quarantine_invalid(
    inputs: &[PathBuf],
    quarantine_root: &Path,
    dry_run: bool,
) -> Result<QuarantineReport, CorpusOpsError> {
    if inputs.is_empty() || quarantine_root.as_os_str().is_empty() {
        return Err(CorpusOpsError::new(
            "quarantine inputs and root are required",
        ));
    }
    let mut report = QuarantineReport {
        schema: CORPUS_OPERATION_REPORT_SCHEMA_V1,
        dry_run,
        valid: Vec::new(),
        quarantined: Vec::new(),
    };
    for input in inputs {
        match TransitionCorpus::read_zstd_file(input) {
            Ok(_) => report.valid.push(input.clone()),
            Err(error) => {
                let bytes = fs::read(input)?;
                let digest = Digest(Sha256::digest(&bytes).into());
                let destination = quarantine_root.join(format!("{digest}.dtcz"));
                if !dry_run {
                    fs::create_dir_all(quarantine_root)?;
                    if destination.exists() {
                        return Err(CorpusOpsError::new("quarantine destination already exists"));
                    }
                    fs::rename(input, &destination)?;
                }
                report.quarantined.push(QuarantinedCorpus {
                    source: input.clone(),
                    destination,
                    reason: error.to_string(),
                });
            }
        }
    }
    Ok(report)
}

pub fn export_arrow(
    inputs: &[PathBuf],
    output: &Path,
) -> Result<AnalysisExportReport, CorpusOpsError> {
    prepare_new_output(output)?;
    let manifest_path = PathBuf::from(format!("{}.manifest.json", output.display()));
    prepare_new_output(&manifest_path)?;
    let corpora = load_compatible(inputs)?;
    let feature_schema = corpora[0].feature_schema;
    let action_schema = corpora[0].action_schema;
    let feature_count = corpora[0].feature_count;
    let input_sha256 = corpora
        .iter()
        .map(TransitionCorpus::content_digest)
        .collect::<Result<Vec<_>, _>>()?;
    let rows = corpora.iter().map(|corpus| corpus.transitions.len()).sum();
    if inputs.len() > 64 || rows > MAX_TRANSITIONS {
        return Err(CorpusOpsError::new(
            "Arrow export exceeds 64 corpora or the one-million-row bound",
        ));
    }
    let width = i32::try_from(feature_count)
        .map_err(|_| CorpusOpsError::new("feature count does not fit Arrow width"))?;

    let mut corpus_sha256 = StringBuilder::with_capacity(rows, rows * 64);
    let mut transition_index = UInt64Builder::with_capacity(rows);
    let mut source_kind = UInt8Builder::with_capacity(rows);
    let mut source_sha256 = StringBuilder::with_capacity(rows, rows * 64);
    let mut state = FixedSizeListBuilder::new(
        Float32Builder::with_capacity(rows.saturating_mul(feature_count as usize)),
        width,
    );
    let mut action_id = UInt32Builder::with_capacity(rows);
    let mut macro_kind = UInt16Builder::with_capacity(rows);
    let mut action_parameters = ListBuilder::new(Int16Builder::new());
    let mut duration_ticks = UInt32Builder::with_capacity(rows);
    let mut reward = Float32Builder::with_capacity(rows);
    let mut next_kind = UInt8Builder::with_capacity(rows);
    let mut next_sha256 = StringBuilder::with_capacity(rows, rows * 64);
    let mut next_state = FixedSizeListBuilder::new(
        Float32Builder::with_capacity(rows.saturating_mul(feature_count as usize)),
        width,
    );
    let mut terminal = BooleanBuilder::with_capacity(rows);
    for (corpus, corpus_digest) in corpora.iter().zip(&input_sha256) {
        for (index, transition) in corpus.transitions.iter().enumerate() {
            corpus_sha256.append_value(corpus_digest.to_string());
            transition_index.append_value(index as u64);
            source_kind.append_value(transition.source.kind as u8);
            source_sha256.append_value(transition.source.digest.to_string());
            for value in &transition.state {
                state.values().append_value(*value);
            }
            state.append(true);
            action_id.append_value(transition.action.action_id);
            macro_kind.append_value(transition.action.macro_kind);
            for value in &transition.action.parameters {
                action_parameters.values().append_value(*value);
            }
            action_parameters.append(true);
            duration_ticks.append_value(transition.duration_ticks);
            reward.append_value(transition.reward);
            next_kind.append_value(transition.next.kind as u8);
            next_sha256.append_value(transition.next.digest.to_string());
            for value in &transition.next_state {
                next_state.values().append_value(*value);
            }
            next_state.append(true);
            terminal.append_value(transition.terminal);
        }
    }
    // Arrow builders expose child validity even though authoritative corpora
    // never emit null feature/parameter values.
    let item = Arc::new(Field::new("item", DataType::Float32, true));
    let parameter = Arc::new(Field::new("item", DataType::Int16, true));
    let mut metadata = std::collections::HashMap::new();
    metadata.insert(
        "dusklight.schema".into(),
        "dusklight-analysis-transition-arrow/v1".into(),
    );
    metadata.insert("dusklight.replay_authority".into(), "false".into());
    metadata.insert(
        "dusklight.feature_schema_sha256".into(),
        feature_schema.to_string(),
    );
    metadata.insert(
        "dusklight.action_schema_sha256".into(),
        action_schema.to_string(),
    );
    let arrow_schema = Arc::new(Schema::new_with_metadata(
        vec![
            Field::new("corpus_sha256", DataType::Utf8, false),
            Field::new("transition_index", DataType::UInt64, false),
            Field::new("source_kind", DataType::UInt8, false),
            Field::new("source_sha256", DataType::Utf8, false),
            Field::new("state", DataType::FixedSizeList(item.clone(), width), false),
            Field::new("action_id", DataType::UInt32, false),
            Field::new("macro_kind", DataType::UInt16, false),
            Field::new("action_parameters", DataType::List(parameter), false),
            Field::new("duration_ticks", DataType::UInt32, false),
            Field::new("reward", DataType::Float32, false),
            Field::new("next_kind", DataType::UInt8, false),
            Field::new("next_sha256", DataType::Utf8, false),
            Field::new("next_state", DataType::FixedSizeList(item, width), false),
            Field::new("terminal", DataType::Boolean, false),
        ],
        metadata,
    ));
    let columns: Vec<ArrayRef> = vec![
        Arc::new(corpus_sha256.finish()),
        Arc::new(transition_index.finish()),
        Arc::new(source_kind.finish()),
        Arc::new(source_sha256.finish()),
        Arc::new(state.finish()),
        Arc::new(action_id.finish()),
        Arc::new(macro_kind.finish()),
        Arc::new(action_parameters.finish()),
        Arc::new(duration_ticks.finish()),
        Arc::new(reward.finish()),
        Arc::new(next_kind.finish()),
        Arc::new(next_sha256.finish()),
        Arc::new(next_state.finish()),
        Arc::new(terminal.finish()),
    ];
    let batch = RecordBatch::try_new(arrow_schema.clone(), columns)
        .map_err(|error| CorpusOpsError::new(error.to_string()))?;
    let output_file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)?;
    let mut writer = FileWriter::try_new(output_file, &arrow_schema)
        .map_err(|error| CorpusOpsError::new(error.to_string()))?;
    writer
        .write(&batch)
        .map_err(|error| CorpusOpsError::new(error.to_string()))?;
    writer
        .finish()
        .map_err(|error| CorpusOpsError::new(error.to_string()))?;
    drop(writer);
    let output_sha256 = sha256_file(output)?;
    let report = AnalysisExportReport {
        schema: "dusklight-analysis-export/v1",
        format: "apache-arrow-ipc-file",
        replay_authority: false,
        inputs: inputs.to_vec(),
        input_sha256,
        output: output.into(),
        output_sha256,
        rows,
        feature_count,
        feature_schema,
        action_schema,
        manifest: manifest_path.clone(),
    };
    let mut manifest_file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&manifest_path)?;
    std::io::Write::write_all(&mut manifest_file, &serde_json::to_vec_pretty(&report)?)?;
    Ok(report)
}

fn sha256_file(path: &Path) -> Result<Digest, CorpusOpsError> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(Digest(hasher.finalize().into()))
}

fn load_compatible(inputs: &[PathBuf]) -> Result<Vec<TransitionCorpus>, CorpusOpsError> {
    if inputs.is_empty() {
        return Err(CorpusOpsError::new("at least one corpus is required"));
    }
    let corpora = inputs
        .iter()
        .map(TransitionCorpus::read_zstd_file)
        .collect::<Result<Vec<_>, _>>()?;
    let first = &corpora[0];
    if corpora.iter().any(|corpus| {
        corpus.feature_schema != first.feature_schema
            || corpus.action_schema != first.action_schema
            || corpus.feature_count != first.feature_count
    }) {
        return Err(CorpusOpsError::new(
            "transition corpus schemas are incompatible",
        ));
    }
    Ok(corpora)
}

fn prepare_new_output(output: &Path) -> Result<(), CorpusOpsError> {
    if output.exists() {
        return Err(CorpusOpsError::new(format!(
            "output already exists: {}",
            output.display()
        )));
    }
    if let Some(parent) = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    Ok(())
}

fn transition_digest(transition: &Transition) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight.transition/v1\0");
    hasher.update([transition.source.kind as u8]);
    hasher.update(transition.source.digest.as_bytes());
    for value in &transition.state {
        hasher.update(value.to_bits().to_le_bytes());
    }
    hasher.update(transition.action.action_id.to_le_bytes());
    hasher.update(transition.action.macro_kind.to_le_bytes());
    for value in &transition.action.parameters {
        hasher.update(value.to_le_bytes());
    }
    hasher.update(transition.duration_ticks.to_le_bytes());
    hasher.update(transition.reward.to_bits().to_le_bytes());
    hasher.update([transition.next.kind as u8]);
    hasher.update(transition.next.digest.as_bytes());
    for value in &transition.next_state {
        hasher.update(value.to_bits().to_le_bytes());
    }
    hasher.update([u8::from(transition.terminal)]);
    Digest(hasher.finalize().into())
}

#[derive(Debug)]
pub struct CorpusOpsError(String);

impl CorpusOpsError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for CorpusOpsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for CorpusOpsError {}

impl From<std::io::Error> for CorpusOpsError {
    fn from(value: std::io::Error) -> Self {
        Self(value.to_string())
    }
}
impl From<serde_json::Error> for CorpusOpsError {
    fn from(value: serde_json::Error) -> Self {
        Self(value.to_string())
    }
}
impl From<crate::transition_corpus::TransitionCorpusError> for CorpusOpsError {
    fn from(value: crate::transition_corpus::TransitionCorpusError) -> Self {
        Self(value.to_string())
    }
}
impl From<crate::offline_rl::OfflineRlError> for CorpusOpsError {
    fn from(value: crate::offline_rl::OfflineRlError) -> Self {
        Self(value.to_string())
    }
}
impl From<crate::dataset::DatasetError> for CorpusOpsError {
    fn from(value: crate::dataset::DatasetError) -> Self {
        Self(value.to_string())
    }
}
impl From<crate::episode::EpisodeError> for CorpusOpsError {
    fn from(value: crate::episode::EpisodeError) -> Self {
        Self(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transition_corpus::{MacroAction, StateReference, StateReferenceKind};
    use arrow_ipc::reader::FileReader;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn corpus(actions: &[u32]) -> TransitionCorpus {
        TransitionCorpus::new(
            Digest([1; 32]),
            Digest([2; 32]),
            1,
            actions
                .iter()
                .enumerate()
                .map(|(index, action)| Transition {
                    source: StateReference {
                        kind: StateReferenceKind::Boundary,
                        digest: Digest([index as u8 + 3; 32]),
                    },
                    state: vec![index as f32],
                    action: MacroAction {
                        action_id: *action,
                        macro_kind: 1,
                        parameters: Vec::new(),
                    },
                    duration_ticks: 1,
                    reward: *action as f32,
                    next: StateReference {
                        kind: StateReferenceKind::Boundary,
                        digest: Digest([index as u8 + 4; 32]),
                    },
                    next_state: vec![index as f32 + 1.0],
                    terminal: index + 1 == actions.len(),
                })
                .collect(),
        )
        .unwrap()
    }

    #[test]
    fn query_compare_merge_shard_and_quarantine_are_schema_checked() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("huntctl-corpus-ops-{nonce}"));
        fs::create_dir_all(&root).unwrap();
        let first = root.join("first.dtcz");
        let second = root.join("second.dtcz");
        corpus(&[1, 2]).write_zstd_file(&first, 1).unwrap();
        corpus(&[1, 2]).write_zstd_file(&second, 1).unwrap();
        assert_eq!(
            query(&[first.clone()], Some(2), None, None, 10)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            compare(&first, &second).unwrap().shared_exact_transitions,
            2
        );
        let merged = root.join("merged.dtcz");
        let report = merge(&[first.clone(), second], &merged, 3).unwrap();
        assert_eq!(report.duplicates_removed, 2);
        let shards = shard(&merged, &root.join("shards"), 1, 1).unwrap();
        assert_eq!(shards.outputs.len(), 2);
        let arrow = root.join("analysis.arrow");
        let exported = export_arrow(&[merged.clone()], &arrow).unwrap();
        assert!(!exported.replay_authority);
        assert_eq!(exported.rows, 2);
        let reader = FileReader::try_new(fs::File::open(&arrow).unwrap(), None).unwrap();
        assert_eq!(
            reader.schema().metadata()["dusklight.replay_authority"],
            "false"
        );
        assert_eq!(
            reader.map(|batch| batch.unwrap().num_rows()).sum::<usize>(),
            2
        );
        let invalid = root.join("invalid.dtcz");
        fs::write(&invalid, b"invalid").unwrap();
        let quarantine = root.join("quarantine");
        let dry = quarantine_invalid(&[invalid.clone()], &quarantine, true).unwrap();
        assert_eq!(dry.quarantined.len(), 1);
        assert!(invalid.exists());
        let moved = quarantine_invalid(&[invalid.clone()], &quarantine, false).unwrap();
        assert_eq!(moved.quarantined.len(), 1);
        assert!(!invalid.exists());
        assert!(moved.quarantined[0].destination.exists());
        fs::remove_dir_all(root).unwrap();
    }
}
