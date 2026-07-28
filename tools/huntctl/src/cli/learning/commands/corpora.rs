//! Dataset, replay, reachability, and artifact inspection commands.

use super::{
    AuxiliarySplitConfig, ContentKind, ContentStore, DATASET_SOURCE_SCHEMA_V1, DatasetBuildConfig,
    DatasetManifest, DatasetSourceDescriptor, Digest, EpisodeContext, EpisodeManifest,
    EpisodeManifestBuild, ExploratoryExtractConfig, ImmutableEpisodeArtifact, InputTape,
    MAX_LEARN_INPUT_CORPORA, MOVEMENT_STATE_V2_ID, NATIVE_AUXILIARY_DATASET_SCHEMA_V2,
    NATIVE_GOAL_TRAJECTORY_DATASET_SCHEMA_V2, NATIVE_REPLAY_CORPUS_SCHEMA_V1,
    NATIVE_REPLAY_SOURCE_SCHEMA_V1, NativeAuxiliaryDataset, NativeEpisodeShard,
    NativeGoalTrajectoryConfig, NativeGoalTrajectoryDataset, NativeReplayCorpus,
    NativeReplaySourceDescriptor, NativeReturnRestartWriteTrace, ReplayEpisodeSource,
    ReplayExperienceRole, Sha256, SiblingTraceDiff, TerminalReasonEvidence,
    TransitionEvidenceBuild, TransitionEvidenceBundle, extract_exploratory_from_bytes,
    extract_exploratory_v2_from_bytes, extract_exploratory_v3_from_bytes,
    inspect_native_episode_corpus, movement_state_v2_spec, option, parse_replay_role,
    repeated_option, required_path, u64_option, usage_error, usize_option,
};
use serde_json::json;
use sha2::Digest as _;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

pub(super) fn command(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("diff-episodes") => {
            let learn_args = &args[1..];
            let success_trace_path = required_path(learn_args, "--success-trace")?;
            let failure_trace_path = required_path(learn_args, "--failure-trace")?;
            let output = required_path(learn_args, "--output")?;
            if output.exists() {
                return Err(
                    format!("trace diff output already exists: {}", output.display()).into(),
                );
            }
            let success_evidence_path = option(learn_args, "--success-evidence").map(PathBuf::from);
            let failure_evidence_path = option(learn_args, "--failure-evidence").map(PathBuf::from);
            if success_evidence_path.is_some() != failure_evidence_path.is_some() {
                return Err(
                    "--success-evidence and --failure-evidence must be supplied together".into(),
                );
            }
            let success_bytes = fs::read(&success_trace_path)?;
            let failure_bytes = fs::read(&failure_trace_path)?;
            let success_trace = huntctl::trace::decode(&success_bytes)?;
            let failure_trace = huntctl::trace::decode(&failure_bytes)?;
            let success_evidence: Option<TransitionEvidenceBundle> = success_evidence_path
                .as_ref()
                .map(|path| fs::read(path).map_err(Box::<dyn Error>::from))
                .transpose()?
                .map(|bytes| serde_json::from_slice(&bytes))
                .transpose()?;
            let failure_evidence: Option<TransitionEvidenceBundle> = failure_evidence_path
                .as_ref()
                .map(|path| fs::read(path).map_err(Box::<dyn Error>::from))
                .transpose()?
                .map(|bytes| serde_json::from_slice(&bytes))
                .transpose()?;
            let report = SiblingTraceDiff::compare(
                &success_trace,
                &success_bytes,
                &failure_trace,
                &failure_bytes,
                success_evidence.as_ref(),
                failure_evidence.as_ref(),
            )?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            fs::write(&output, serde_json::to_vec_pretty(&report)?)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Some("dataset") => {
            let learn_args = &args[1..];
            let source_paths = repeated_option(learn_args, "--source");
            if source_paths.is_empty() {
                return Err("learn dataset requires at least one --source SOURCE.json".into());
            }
            let output = required_path(learn_args, "--output")?;
            if output.exists() {
                return Err(format!("dataset output already exists: {}", output.display()).into());
            }
            let mut sources = Vec::with_capacity(source_paths.len());
            for source_path in &source_paths {
                let source_path = PathBuf::from(source_path);
                let descriptor: DatasetSourceDescriptor =
                    serde_json::from_slice(&fs::read(&source_path)?)?;
                sources.push(descriptor.load(source_path.parent().unwrap_or(Path::new(".")))?);
            }
            let validation_percent =
                u8::try_from(usize_option(learn_args, "--validation-percent", 10)?)?;
            let test_percent = u8::try_from(usize_option(learn_args, "--test-percent", 10)?)?;
            let manifest = DatasetManifest::build(
                &sources,
                &DatasetBuildConfig {
                    validation_percent,
                    test_percent,
                    withheld_objectives: repeated_option(learn_args, "--withheld-objective")
                        .into_iter()
                        .collect(),
                },
            )?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            let bytes = serde_json::to_vec_pretty(&manifest)?;
            fs::write(&output, &bytes)?;
            let artifact_store = option(learn_args, "--artifact-store")
                .map(PathBuf::from)
                .unwrap_or_else(|| output.parent().unwrap_or(Path::new(".")).join("content"));
            let content_blob = ContentStore::initialize(&artifact_store)?
                .put_bytes(&bytes, ContentKind::DatasetManifest)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": manifest.schema,
                    "dataset_sha256": manifest.dataset_sha256,
                    "frozen_withheld_sha256": manifest.frozen_withheld_sha256,
                    "output": output,
                    "artifact_store": artifact_store,
                    "content_blob": content_blob,
                    "report": manifest.report,
                    "leakage": manifest.leakage,
                    "normalization_schemas": manifest.normalization.len(),
                }))?
            );
            Ok(())
        }
        Some("extract-trace") => {
            let learn_args = &args[1..];
            let trace_path = required_path(learn_args, "--trace")?;
            let tape_path = required_path(learn_args, "--tape")?;
            let episode_context_path = required_path(learn_args, "--episode-context")?;
            let output = required_path(learn_args, "--output")?;
            let start_tape_frame: u64 = option(learn_args, "--start-frame")
                .ok_or("missing required --start-frame N")?
                .parse()?;
            let end_tape_frame: u64 = option(learn_args, "--end-frame")
                .ok_or("missing required --end-frame N")?
                .parse()?;
            let trace_bytes = fs::read(&trace_path)?;
            let tape_bytes = fs::read(&tape_path)?;
            let episode_context: EpisodeContext =
                serde_json::from_slice(&fs::read(&episode_context_path)?)?;
            episode_context.validate()?;
            let episode_digest = if let Some(value) = option(learn_args, "--episode-digest") {
                value.parse::<Digest>()?
            } else {
                let mut hasher = Sha256::new();
                hasher.update(b"dusklight.exploratory-offline-episode/v1\0");
                hasher.update((trace_bytes.len() as u64).to_le_bytes());
                hasher.update(&trace_bytes);
                hasher.update((tape_bytes.len() as u64).to_le_bytes());
                hasher.update(&tape_bytes);
                Digest(hasher.finalize().into())
            };
            let end_is_terminal = learn_args.iter().any(|arg| arg == "--terminal");
            let feature_view =
                option(learn_args, "--view").unwrap_or_else(|| "movement-state/v1".into());
            let action_view =
                option(learn_args, "--action-view").unwrap_or_else(|| "movement-action/v2".into());
            let extract_config = ExploratoryExtractConfig {
                episode_digest,
                start_tape_frame,
                end_tape_frame,
                start_reference: None,
                terminal_reference: None,
                end_is_terminal,
            };
            let corpus = match (feature_view.as_str(), action_view.as_str()) {
                ("movement-state/v1", "movement-action/v2") => {
                    extract_exploratory_from_bytes(&trace_bytes, &tape_bytes, extract_config)?
                }
                (MOVEMENT_STATE_V2_ID, "movement-action/v2") => {
                    extract_exploratory_v2_from_bytes(&trace_bytes, &tape_bytes, extract_config)?
                }
                (MOVEMENT_STATE_V2_ID, "movement-action/v3") => {
                    extract_exploratory_v3_from_bytes(&trace_bytes, &tape_bytes, extract_config)?
                }
                (feature, actions) => {
                    return Err(format!(
                        "unsupported observation/action view pair {feature:?}/{actions:?}; expected movement-state/v1 with movement-action/v2, or {MOVEMENT_STATE_V2_ID} with movement-action/v2 or movement-action/v3"
                    )
                    .into());
                }
            };
            let decoded_trace = huntctl::trace::decode(&trace_bytes)?;
            let decoded_tape = InputTape::decode(&tape_bytes)?.tape;
            let transition_evidence = TransitionEvidenceBundle::build(TransitionEvidenceBuild {
                corpus: &corpus,
                trace: &decoded_trace,
                tape: &decoded_tape,
                trace_sha256: Digest(Sha256::digest(&trace_bytes).into()),
                tape_sha256: Digest(Sha256::digest(&tape_bytes).into()),
                start_tape_frame,
                end_tape_frame,
                terminal_reason: end_is_terminal
                    .then_some(TerminalReasonEvidence::DeclaredExtractionBoundary),
            })?;
            let transition_evidence_bytes = serde_json::to_vec_pretty(&transition_evidence)?;
            let trace_sha256 = Digest(Sha256::digest(&trace_bytes).into());
            let tape_sha256 = Digest(Sha256::digest(&tape_bytes).into());
            let episode_manifest = EpisodeManifest::build(EpisodeManifestBuild {
                context: &episode_context,
                boot: &decoded_tape.boot,
                corpus: &corpus,
                query_view_id: &feature_view,
                tape_sha256,
                trace_sha256,
                transition_evidence_sha256: Digest(
                    Sha256::digest(&transition_evidence_bytes).into(),
                ),
            })?;
            let compression_level: i32 = option(learn_args, "--compression-level")
                .map(|value| value.parse())
                .transpose()?
                .unwrap_or(3);
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            let content_digest = corpus.write_zstd_file(&output, compression_level)?;
            let artifact_store = option(learn_args, "--artifact-store")
                .map(PathBuf::from)
                .unwrap_or_else(|| output.parent().unwrap_or(Path::new(".")).join("content"));
            let trace_content_blob = ContentStore::initialize(&artifact_store)?
                .put_bytes(&trace_bytes, ContentKind::GameplayTrace)?;
            let transition_evidence_path =
                PathBuf::from(format!("{}.evidence.json", output.display()));
            fs::write(&transition_evidence_path, transition_evidence_bytes)?;
            let episode_manifest_path = PathBuf::from(format!("{}.episode.json", output.display()));
            fs::write(
                &episode_manifest_path,
                serde_json::to_vec_pretty(&episode_manifest)?,
            )?;
            let dataset_source_path =
                PathBuf::from(format!("{}.dataset-source.json", output.display()));
            fs::write(
                &dataset_source_path,
                serde_json::to_vec_pretty(&DatasetSourceDescriptor {
                    schema: DATASET_SOURCE_SCHEMA_V1.into(),
                    source_id: episode_manifest.episode_sha256.to_string(),
                    episode_manifest: fs::canonicalize(&episode_manifest_path)?,
                    transition_corpus: fs::canonicalize(&output)?,
                    absolute_tape: fs::canonicalize(&tape_path)?,
                    transition_evidence: fs::canonicalize(&transition_evidence_path)?,
                    gameplay_trace: fs::canonicalize(&trace_path)?,
                    route_family: episode_manifest.objective.id.clone(),
                    screenshot_sha256: Vec::new(),
                    checkpoint_sha256: Vec::new(),
                })?,
            )?;
            let observation_spec = if feature_view == MOVEMENT_STATE_V2_ID {
                let spec = movement_state_v2_spec();
                let path = PathBuf::from(format!("{}.observation.json", output.display()));
                fs::write(&path, spec.canonical_bytes()?)?;
                Some(path)
            } else {
                None
            };
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": "dusklight-exploratory-extraction/v1",
                    "authoritative": false,
                    "limitations": [
                        "the batch contains observed behavior, not counterfactual actions",
                        "explicit frame bounds are not native milestone proof",
                        "--terminal records a declared extraction boundary, not inferred objective proof",
                        "the observation view is objective-specific and not a complete process state"
                    ],
                    "trace": trace_path,
                    "trace_content_blob": trace_content_blob,
                    "artifact_store": artifact_store,
                    "tape": tape_path,
                    "output": output,
                    "transition_evidence": transition_evidence_path,
                    "episode_context": episode_context_path,
                    "episode_manifest": episode_manifest_path,
                    "dataset_source": dataset_source_path,
                    "input_identity": episode_manifest.input_identity_sha256,
                    "episode_identity": episode_manifest.episode_sha256,
                    "feature_view": feature_view,
                    "observation_spec": observation_spec,
                    "episode_digest": episode_digest,
                    "content_digest": content_digest,
                    "feature_schema": corpus.feature_schema,
                    "action_schema": corpus.action_schema,
                    "feature_count": corpus.feature_count,
                    "transitions": corpus.transitions.len(),
                    "start_frame": start_tape_frame,
                    "end_frame": end_tape_frame,
                    "terminal": end_is_terminal,
                }))?
            );
            Ok(())
        }
        Some("inspect-episode") => {
            let input = required_path(&args[1..], "--input")?;
            let artifact: ImmutableEpisodeArtifact = serde_json::from_slice(&fs::read(&input)?)?;
            artifact.validate()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": artifact.schema,
                    "content_sha256": artifact.content_sha256,
                    "episode_sha256": artifact.episode_sha256,
                    "objective": artifact.objective,
                    "terminal": artifact.terminal,
                    "terminal_detail": artifact.terminal_detail,
                    "realized_tape_sha256": artifact.realized_tape_sha256,
                    "gameplay_trace_sha256": artifact.gameplay_trace_sha256,
                    "transition_corpus_sha256": artifact.transition_corpus_sha256,
                    "transition_evidence_sha256": artifact.transition_evidence_sha256,
                    "steps": artifact.steps.len(),
                    "lineage": artifact.lineage,
                }))?
            );
            Ok(())
        }
        Some("inspect-native") => {
            let learn_args = &args[1..];
            let inputs = repeated_option(learn_args, "--input");
            if inputs.is_empty() || inputs.len() > MAX_LEARN_INPUT_CORPORA {
                return Err(format!(
                    "learn inspect-native requires 1..={MAX_LEARN_INPUT_CORPORA} --input SHARD"
                )
                .into());
            }
            let shards = inputs
                .iter()
                .map(NativeEpisodeShard::read)
                .collect::<Result<Vec<_>, _>>()?;
            let report = inspect_native_episode_corpus(&shards);
            let bytes = serde_json::to_vec_pretty(&report)?;
            if let Some(output) = option(learn_args, "--output").map(PathBuf::from) {
                if output.exists() {
                    return Err(format!(
                        "native corpus inspection output already exists: {}",
                        output.display()
                    )
                    .into());
                }
                if let Some(parent) = output
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                {
                    fs::create_dir_all(parent)?;
                }
                fs::write(output, &bytes)?;
            }
            println!("{}", String::from_utf8(bytes)?);
            Ok(())
        }
        Some("trace-return-restart-writes") => {
            let learn_args = &args[1..];
            let inputs = repeated_option(learn_args, "--input");
            if inputs.is_empty() || inputs.len() > MAX_LEARN_INPUT_CORPORA {
                return Err(format!(
                    "learn trace-return-restart-writes requires 1..={MAX_LEARN_INPUT_CORPORA} --input SHARD"
                )
                .into());
            }
            let output = required_path(learn_args, "--output")?;
            if output.exists() {
                return Err(format!(
                    "return/restart write trace output already exists: {}",
                    output.display()
                )
                .into());
            }
            let shards = inputs
                .iter()
                .map(NativeEpisodeShard::read)
                .collect::<Result<Vec<_>, _>>()?;
            let report = NativeReturnRestartWriteTrace::build(&shards)?;
            let bytes = serde_json::to_vec_pretty(&report)?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            fs::write(&output, &bytes)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": report.schema,
                    "content_sha256": report.content_sha256,
                    "source_shards": report.source_shards.len(),
                    "summary": report.summary,
                    "output": output,
                }))?
            );
            Ok(())
        }
        Some("validate-return-restart-write-trace") => {
            let input = required_path(&args[1..], "--input")?;
            let report = NativeReturnRestartWriteTrace::decode(&fs::read(&input)?)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": report.schema,
                    "content_sha256": report.content_sha256,
                    "source_shards": report.source_shards.len(),
                    "summary": report.summary,
                    "input": input,
                }))?
            );
            Ok(())
        }
        Some("native-replay") => {
            let learn_args = &args[1..];
            let source_paths = repeated_option(learn_args, "--source");
            let shard_paths = repeated_option(learn_args, "--input");
            if source_paths.is_empty() == shard_paths.is_empty()
                || source_paths.len().max(shard_paths.len()) > MAX_LEARN_INPUT_CORPORA
            {
                return Err(format!(
                    "learn native-replay requires either 1..={MAX_LEARN_INPUT_CORPORA} --source SOURCE.json or --input EPISODES.dseps with --role ROLE"
                )
                .into());
            }
            let output = required_path(learn_args, "--output")?;
            if output.exists() {
                return Err(format!(
                    "native replay corpus output already exists: {}",
                    output.display()
                )
                .into());
            }
            let previous: Option<NativeReplayCorpus> = option(learn_args, "--previous")
                .map(PathBuf::from)
                .map(|path| -> Result<_, Box<dyn Error>> {
                    let corpus: NativeReplayCorpus = serde_json::from_slice(&fs::read(path)?)?;
                    corpus.validate()?;
                    Ok(corpus)
                })
                .transpose()?;
            let corpus = if !source_paths.is_empty() {
                let mut loaded = Vec::with_capacity(source_paths.len());
                for source_path in source_paths {
                    let descriptor_path = PathBuf::from(source_path);
                    let descriptor: NativeReplaySourceDescriptor =
                        serde_json::from_slice(&fs::read(&descriptor_path)?)?;
                    if descriptor.schema != NATIVE_REPLAY_SOURCE_SCHEMA_V1 {
                        return Err(format!(
                            "native replay source has invalid schema: {}",
                            descriptor_path.display()
                        )
                        .into());
                    }
                    let shard_path = if descriptor.shard.is_absolute() {
                        descriptor.shard.clone()
                    } else {
                        descriptor_path
                            .parent()
                            .unwrap_or(Path::new("."))
                            .join(&descriptor.shard)
                    };
                    loaded.push((descriptor, NativeEpisodeShard::read(shard_path)?));
                }
                let sources = loaded
                    .iter()
                    .map(|(descriptor, shard)| {
                        let episode_index = shard
                            .episodes
                            .iter()
                            .position(|episode| episode.id == descriptor.episode_id)
                            .ok_or_else(|| {
                                format!(
                                    "native replay episode {:?} is absent from shard {}",
                                    descriptor.episode_id, shard.content_sha256
                                )
                            })?;
                        Ok(ReplayEpisodeSource {
                            shard,
                            episode_index,
                            role: descriptor.role,
                            policy_lineage_sha256: descriptor.policy_lineage_sha256,
                            parent_entry_sha256: descriptor.parent_entry_sha256,
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                NativeReplayCorpus::build(previous.as_ref(), &sources)?
            } else {
                let role_value = option(learn_args, "--role")
                    .ok_or("native replay shard ingestion requires --role ROLE")?;
                let role = parse_replay_role(&role_value)?;
                let policy_lineage_sha256 = option(learn_args, "--policy-lineage-sha256")
                    .map(|value| value.parse::<Digest>())
                    .transpose()?;
                if (role == ReplayExperienceRole::PolicyRollout) != policy_lineage_sha256.is_some()
                {
                    return Err(
                        "policy_rollout shard ingestion requires exactly one --policy-lineage-sha256"
                            .into(),
                    );
                }
                let shards = shard_paths
                    .iter()
                    .map(NativeEpisodeShard::read)
                    .collect::<Result<Vec<_>, _>>()?;
                let sources = shards
                    .iter()
                    .flat_map(|shard| {
                        (0..shard.episodes.len()).map(move |episode_index| ReplayEpisodeSource {
                            shard,
                            episode_index,
                            role,
                            policy_lineage_sha256,
                            parent_entry_sha256: None,
                        })
                    })
                    .collect::<Vec<_>>();
                NativeReplayCorpus::build(previous.as_ref(), &sources)?
            };
            let bytes = serde_json::to_vec_pretty(&corpus)?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            fs::write(&output, &bytes)?;
            let artifact_store = option(learn_args, "--artifact-store")
                .map(PathBuf::from)
                .unwrap_or_else(|| output.parent().unwrap_or(Path::new(".")).join("content"));
            let content_blob = ContentStore::initialize(&artifact_store)?
                .put_bytes(&bytes, ContentKind::NativeReplayCorpus)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": NATIVE_REPLAY_CORPUS_SCHEMA_V1,
                    "generation": corpus.generation,
                    "corpus_sha256": corpus.corpus_sha256,
                    "parent_corpus_sha256": corpus.parent_corpus_sha256,
                    "output": output,
                    "artifact_store": artifact_store,
                    "content_blob": content_blob,
                    "report": corpus.report,
                }))?
            );
            Ok(())
        }
        Some("auxiliary-dataset") => {
            let learn_args = &args[1..];
            let corpus_path = required_path(learn_args, "--corpus")?;
            let input_paths = repeated_option(learn_args, "--input");
            if input_paths.is_empty() || input_paths.len() > MAX_LEARN_INPUT_CORPORA {
                return Err(format!(
                    "learn auxiliary-dataset requires 1..={MAX_LEARN_INPUT_CORPORA} --input EPISODES.dseps"
                )
                .into());
            }
            let output = required_path(learn_args, "--output")?;
            if output.exists() {
                return Err(format!(
                    "native auxiliary dataset output already exists: {}",
                    output.display()
                )
                .into());
            }
            let corpus: NativeReplayCorpus = serde_json::from_slice(&fs::read(&corpus_path)?)?;
            corpus.validate()?;
            let shards = input_paths
                .iter()
                .map(NativeEpisodeShard::read)
                .collect::<Result<Vec<_>, _>>()?;
            let defaults = AuxiliarySplitConfig::default();
            let training_basis_points = usize_option(
                learn_args,
                "--training-basis-points",
                usize::from(defaults.training_basis_points),
            )?;
            let validation_basis_points = usize_option(
                learn_args,
                "--validation-basis-points",
                usize::from(defaults.validation_basis_points),
            )?;
            let split_config = AuxiliarySplitConfig {
                training_basis_points: u16::try_from(training_basis_points)
                    .map_err(|_| "training basis points exceed u16")?,
                validation_basis_points: u16::try_from(validation_basis_points)
                    .map_err(|_| "validation basis points exceed u16")?,
                seed: u64_option(learn_args, "--seed", defaults.seed)?,
            };
            let dataset = NativeAuxiliaryDataset::build(&corpus, &shards, split_config)?;
            let bytes = serde_json::to_vec_pretty(&dataset)?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            fs::write(&output, &bytes)?;
            let artifact_store = option(learn_args, "--artifact-store")
                .map(PathBuf::from)
                .unwrap_or_else(|| output.parent().unwrap_or(Path::new(".")).join("content"));
            let content_blob = ContentStore::initialize(&artifact_store)?
                .put_bytes(&bytes, ContentKind::NativeAuxiliaryDataset)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": NATIVE_AUXILIARY_DATASET_SCHEMA_V2,
                    "dataset_sha256": dataset.dataset_sha256,
                    "replay_corpus_sha256": dataset.replay_corpus_sha256,
                    "output": output,
                    "artifact_store": artifact_store,
                    "content_blob": content_blob,
                    "report": dataset.report,
                }))?
            );
            Ok(())
        }
        Some("goal-trajectory-dataset") => {
            let learn_args = &args[1..];
            let corpus_path = required_path(learn_args, "--corpus")?;
            let input_paths = repeated_option(learn_args, "--input");
            if input_paths.is_empty() || input_paths.len() > MAX_LEARN_INPUT_CORPORA {
                return Err(format!(
                    "learn goal-trajectory-dataset requires 1..={MAX_LEARN_INPUT_CORPORA} --input EPISODES.dseps"
                )
                .into());
            }
            let milestones_path = required_path(learn_args, "--milestones")?;
            let milestone_goal = option(learn_args, "--milestone-goal")
                .ok_or("learn goal-trajectory-dataset requires --milestone-goal NAME")?;
            let output = required_path(learn_args, "--output")?;
            if output.exists() {
                return Err(format!(
                    "native goal trajectory dataset output already exists: {}",
                    output.display()
                )
                .into());
            }
            let corpus: NativeReplayCorpus = serde_json::from_slice(&fs::read(&corpus_path)?)?;
            corpus.validate()?;
            let shards = input_paths
                .iter()
                .map(NativeEpisodeShard::read)
                .collect::<Result<Vec<_>, _>>()?;
            let milestone_bytes = fs::read(&milestones_path)?;
            let decoded = huntctl::milestone_dsl::decode(&milestone_bytes)?;
            let definition_index = decoded
                .definitions
                .iter()
                .position(|definition| definition.name == milestone_goal)
                .ok_or_else(|| {
                    format!(
                        "compiled milestone definition {milestone_goal:?} does not exist in {}",
                        milestones_path.display()
                    )
                })?;
            let compiled = huntctl::milestone_dsl::CompiledMilestones {
                bytes: milestone_bytes,
                program_sha256: decoded.program_sha256,
                definitions: decoded.definitions,
            };
            let graph = huntctl::learning::compiled_goal_graph::CompiledGoalGraph::from_compiled(
                &compiled,
                definition_index,
            )?;
            let defaults = NativeGoalTrajectoryConfig::default();
            let n_step = usize_option(learn_args, "--n-step", usize::from(defaults.n_step))?;
            let discount_millionths = usize_option(
                learn_args,
                "--discount-millionths",
                defaults.discount_millionths as usize,
            )?;
            let training_basis_points = usize_option(
                learn_args,
                "--training-basis-points",
                usize::from(defaults.training_basis_points),
            )?;
            let validation_basis_points = usize_option(
                learn_args,
                "--validation-basis-points",
                usize::from(defaults.validation_basis_points),
            )?;
            let config = NativeGoalTrajectoryConfig {
                demonstration_mode: defaults.demonstration_mode,
                n_step: u16::try_from(n_step).map_err(|_| "n-step exceeds u16")?,
                discount_millionths: u32::try_from(discount_millionths)
                    .map_err(|_| "discount millionths exceed u32")?,
                training_basis_points: u16::try_from(training_basis_points)
                    .map_err(|_| "training basis points exceed u16")?,
                validation_basis_points: u16::try_from(validation_basis_points)
                    .map_err(|_| "validation basis points exceed u16")?,
                split_seed: u64_option(learn_args, "--seed", defaults.split_seed)?,
            };
            let dataset = NativeGoalTrajectoryDataset::build(&corpus, &shards, &graph, config)?;
            let bytes = serde_json::to_vec_pretty(&dataset)?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            fs::write(&output, &bytes)?;
            let artifact_store = option(learn_args, "--artifact-store")
                .map(PathBuf::from)
                .unwrap_or_else(|| output.parent().unwrap_or(Path::new(".")).join("content"));
            let content_blob = ContentStore::initialize(&artifact_store)?
                .put_bytes(&bytes, ContentKind::NativeGoalTrajectoryDataset)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": NATIVE_GOAL_TRAJECTORY_DATASET_SCHEMA_V2,
                    "dataset_sha256": dataset.dataset_sha256,
                    "replay_corpus_sha256": dataset.replay_corpus_sha256,
                    "goal": milestone_goal,
                    "goal_graph_sha256": dataset.goal.graph_sha256,
                    "output": output,
                    "artifact_store": artifact_store,
                    "content_blob": content_blob,
                    "report": dataset.report,
                }))?
            );
            Ok(())
        }
        Some("inspect-auxiliary") => {
            let input = required_path(&args[1..], "--input")?;
            let dataset: NativeAuxiliaryDataset = serde_json::from_slice(&fs::read(input)?)?;
            dataset.validate()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": NATIVE_AUXILIARY_DATASET_SCHEMA_V2,
                    "dataset_sha256": dataset.dataset_sha256,
                    "replay_corpus_sha256": dataset.replay_corpus_sha256,
                    "report": dataset.report,
                    "split_diagnostics": dataset.split_diagnostics()?,
                }))?
            );
            Ok(())
        }
        _ => usage_error(),
    }
}
