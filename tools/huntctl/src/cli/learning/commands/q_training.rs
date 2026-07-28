//! Double-Q, fitted-Q, and throughput benchmark commands.

use super::{
    ContentKind, ContentStore, DatasetManifest, DatasetSplit, DoubleQ, DoubleQConfig, FittedQ,
    FqiConfig, FqiTransition, MAX_FQI_ACTIONS, MAX_FQI_BACKUP_STEPS, MAX_FQI_ITERATIONS,
    MAX_FQI_TRANSITIONS, MAX_FQI_TREE_DEPTH, MAX_FQI_TREES_PER_ACTION, MAX_LEARN_INPUT_CORPORA,
    MOVEMENT_CATEGORICAL_FEATURES_V1, PotentialShapingSpec, REWARD_REPORT_SCHEMA_V1,
    TransitionCorpus, load_fqi_batch, movement_feature_schema_digest_v1, movement_state_v2_spec,
    option, repeated_option, u64_option, usage_error, usize_option,
};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

pub(super) fn command(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("double-q") => {
            let learn_args = &args[1..];
            let direct_inputs = repeated_option(learn_args, "--input");
            let dataset_path = option(learn_args, "--dataset").map(PathBuf::from);
            if dataset_path.is_some() && !direct_inputs.is_empty() {
                return Err("learn double-q accepts either --dataset or --input, not both".into());
            }
            let dataset_manifest: Option<DatasetManifest> = dataset_path
                .as_ref()
                .map(|path| -> Result<_, Box<dyn Error>> {
                    let manifest: DatasetManifest = serde_json::from_slice(&fs::read(path)?)?;
                    manifest.validate()?;
                    Ok(manifest)
                })
                .transpose()?;
            let inputs = if let Some(manifest) = &dataset_manifest {
                manifest
                    .entries
                    .iter()
                    .filter(|entry| entry.split == DatasetSplit::Train)
                    .map(|entry| entry.transition_corpus.to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
            } else {
                direct_inputs
            };
            let training = load_fqi_batch(&inputs, "Double-Q training", MAX_LEARN_INPUT_CORPORA)?;
            let expected_corpus_digests = dataset_manifest.as_ref().map(|manifest| {
                manifest
                    .entries
                    .iter()
                    .filter(|entry| entry.split == DatasetSplit::Train)
                    .map(|entry| entry.corpus_sha256)
                    .collect::<Vec<_>>()
            });
            if expected_corpus_digests
                .as_ref()
                .is_some_and(|expected| expected != &training.corpus_digests)
            {
                return Err("Double-Q corpus content differs from dataset manifest".into());
            }
            let config = DoubleQConfig {
                epochs: usize_option(learn_args, "--epochs", DoubleQConfig::default().epochs)?,
                hidden_width: usize_option(
                    learn_args,
                    "--hidden-width",
                    DoubleQConfig::default().hidden_width,
                )?,
                learning_rate: option(learn_args, "--learning-rate")
                    .map(|value| value.parse::<f64>())
                    .transpose()?
                    .unwrap_or(DoubleQConfig::default().learning_rate),
                discount: option(learn_args, "--discount")
                    .map(|value| value.parse::<f64>())
                    .transpose()?
                    .unwrap_or(DoubleQConfig::default().discount),
                target_sync_steps: usize_option(
                    learn_args,
                    "--target-sync-steps",
                    DoubleQConfig::default().target_sync_steps,
                )?,
                gradient_clip: option(learn_args, "--gradient-clip")
                    .map(|value| value.parse::<f64>())
                    .transpose()?
                    .unwrap_or(DoubleQConfig::default().gradient_clip),
                seed: u64_option(learn_args, "--seed", DoubleQConfig::default().seed)?,
            };
            let action_support = training.transitions.iter().fold(
                BTreeMap::<u32, usize>::new(),
                |mut counts, transition| {
                    *counts.entry(transition.action).or_default() += 1;
                    counts
                },
            );
            if action_support.len() > MAX_FQI_ACTIONS {
                return Err(format!(
                    "Double-Q supports at most {MAX_FQI_ACTIONS} distinct actions; received {}",
                    action_support.len()
                )
                .into());
            }
            let actions = action_support.keys().copied().collect::<Vec<_>>();
            let model = DoubleQ::fit(
                training.feature_count,
                &actions,
                &training.transitions,
                &config,
            )?;
            let query_index = usize_option(learn_args, "--query-transition", 0)?;
            let query_transition = training
                .transitions
                .get(query_index)
                .ok_or("--query-transition is outside the merged transition batch")?;
            let query_side = option(learn_args, "--query-side").unwrap_or_else(|| "state".into());
            let query_state = match query_side.as_str() {
                "state" => &query_transition.state,
                "next-state" => &query_transition.next_state,
                _ => return Err("--query-side must be state or next-state".into()),
            };
            let ranking = model
                .rank_actions(query_state)?
                .into_iter()
                .map(|estimate| {
                    json!({
                        "action": estimate.action,
                        "mean_q": estimate.mean,
                        "critic_a": estimate.critic_a,
                        "critic_b": estimate.critic_b,
                        "critic_disagreement": estimate.critic_disagreement,
                        "support": action_support[&estimate.action],
                    })
                })
                .collect::<Vec<_>>();
            let model_output = option(learn_args, "--model-output").map(PathBuf::from);
            let mut model_content_blob = None;
            let mut model_artifact_store = None;
            if let Some(path) = &model_output {
                if path.exists() {
                    return Err(format!(
                        "Double-Q model output already exists: {}",
                        path.display()
                    )
                    .into());
                }
                if let Some(parent) = path
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                {
                    fs::create_dir_all(parent)?;
                }
                let bytes = model.artifact_bytes(
                    training.feature_schema,
                    training.action_schema,
                    dataset_manifest
                        .as_ref()
                        .map(|manifest| manifest.dataset_sha256),
                    &training.corpus_digests,
                    &config,
                )?;
                fs::write(path, &bytes)?;
                let store_path = option(learn_args, "--artifact-store")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| path.parent().unwrap_or(Path::new(".")).join("content"));
                model_content_blob = Some(
                    ContentStore::initialize(&store_path)?.put_bytes(&bytes, ContentKind::Model)?,
                );
                model_artifact_store = Some(store_path);
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": "dusklight-double-q-ranking/v1",
                    "feature_schema": training.feature_schema,
                    "action_schema": training.action_schema,
                    "input_corpora": inputs,
                    "training_corpus_sha256": training.corpus_digests,
                    "training_dataset": dataset_path,
                    "training_dataset_sha256": dataset_manifest.as_ref().map(|manifest| manifest.dataset_sha256),
                    "transition_count": training.transitions.len(),
                    "episode_groups": training.episode_groups.iter().copied().collect::<BTreeSet<_>>().len(),
                    "query_transition": query_index,
                    "query_side": query_side,
                    "config": config,
                    "gradient_updates": model.gradient_updates(),
                    "target_synchronizations": model.target_synchronizations(),
                    "target_evaluation": "online_selects_opposite_frozen_target_evaluates",
                    "sample_order": "deterministic_seeded_epoch_shuffle",
                    "model_output": model_output,
                    "model_artifact_store": model_artifact_store,
                    "model_content_blob": model_content_blob,
                    "ranking": ranking,
                    "promotion_authority": false,
                    "limitations": [
                        "offline Double-Q can overvalue actions outside dataset support; use support diagnostics and the conservative learner",
                        "numeric normalization does not provide categorical embeddings or missingness masks",
                        "critic disagreement is not calibrated uncertainty",
                        "rankings are proposals and require native predicate and cold replay proof"
                    ]
                }))?
            );
            Ok(())
        }
        Some("fit") => {
            let learn_args = &args[1..];
            let direct_inputs = repeated_option(learn_args, "--input");
            let dataset_path = option(learn_args, "--dataset").map(PathBuf::from);
            if dataset_path.is_some() && !direct_inputs.is_empty() {
                return Err("learn fit accepts either --dataset or --input, not both".into());
            }
            let dataset_manifest: Option<DatasetManifest> = dataset_path
                .as_ref()
                .map(|path| -> Result<_, Box<dyn Error>> {
                    let manifest: DatasetManifest = serde_json::from_slice(&fs::read(path)?)?;
                    manifest.validate()?;
                    Ok(manifest)
                })
                .transpose()?;
            let inputs = if let Some(manifest) = &dataset_manifest {
                manifest
                    .entries
                    .iter()
                    .filter(|entry| entry.split == huntctl::dataset::DatasetSplit::Train)
                    .map(|entry| entry.transition_corpus.to_string_lossy().into_owned())
                    .collect()
            } else {
                direct_inputs
            };
            if inputs.is_empty() {
                return Err(
                    "learn fit requires training entries in --dataset or at least one --input FILE"
                        .into(),
                );
            }
            if inputs.len() > MAX_LEARN_INPUT_CORPORA {
                return Err(format!(
                    "learn fit accepts at most {MAX_LEARN_INPUT_CORPORA} input corpora; received {}",
                    inputs.len()
                )
                .into());
            }
            let mut config = FqiConfig {
                iterations: usize_option(learn_args, "--iterations", 24)?,
                backup_steps: usize_option(learn_args, "--n-step", 1)?,
                trees_per_action: usize_option(learn_args, "--trees", 31)?,
                max_tree_depth: usize_option(learn_args, "--max-depth", 8)?,
                seed: u64_option(learn_args, "--seed", FqiConfig::default().seed)?,
                discount: option(learn_args, "--discount")
                    .map(|value| value.parse::<f32>())
                    .transpose()?
                    .unwrap_or(FqiConfig::default().discount),
                ..FqiConfig::default()
            };
            if config.iterations > MAX_FQI_ITERATIONS {
                return Err(format!(
                    "--iterations must not exceed {MAX_FQI_ITERATIONS}; received {}",
                    config.iterations
                )
                .into());
            }
            if config.backup_steps == 0 || config.backup_steps > MAX_FQI_BACKUP_STEPS {
                return Err(format!(
                    "--n-step must be within 1..={MAX_FQI_BACKUP_STEPS}; received {}",
                    config.backup_steps
                )
                .into());
            }
            if config.trees_per_action > MAX_FQI_TREES_PER_ACTION {
                return Err(format!(
                    "--trees must not exceed {MAX_FQI_TREES_PER_ACTION}; received {}",
                    config.trees_per_action
                )
                .into());
            }
            if config.max_tree_depth > MAX_FQI_TREE_DEPTH {
                return Err(format!(
                    "--max-depth must not exceed {MAX_FQI_TREE_DEPTH}; received {}",
                    config.max_tree_depth
                )
                .into());
            }
            let mut feature_schema = None;
            let mut action_schema = None;
            let mut feature_count = None;
            let mut transitions = Vec::new();
            let mut episode_groups = Vec::new();
            let mut next_episode_group = 0_u64;
            let mut training_corpus_sha256 = Vec::new();
            let mut action_support = BTreeMap::<u32, usize>::new();
            let shaping_path = option(learn_args, "--shaping").map(PathBuf::from);
            let shaping_report_path = option(learn_args, "--shaping-report").map(PathBuf::from);
            if shaping_path.is_some() != shaping_report_path.is_some() {
                return Err(
                    "--shaping SPEC.json and --shaping-report REPORT.json must be supplied together"
                        .into(),
                );
            }
            let shaping_spec: Option<PotentialShapingSpec> = if let Some(path) = &shaping_path {
                Some(serde_json::from_slice(&fs::read(path)?)?)
            } else {
                None
            };
            let mut shaping_records = Vec::new();
            for input in &inputs {
                let corpus = TransitionCorpus::read_zstd_file(input)?;
                training_corpus_sha256.push(corpus.content_digest()?);
                if feature_schema.is_some_and(|value| value != corpus.feature_schema)
                    || action_schema.is_some_and(|value| value != corpus.action_schema)
                    || feature_count.is_some_and(|value| value != corpus.feature_count)
                {
                    return Err(
                        "transition corpora use incompatible feature or action schemas".into(),
                    );
                }
                feature_schema = Some(corpus.feature_schema);
                action_schema = Some(corpus.action_schema);
                feature_count = Some(corpus.feature_count);
                if let Some(spec) = &shaping_spec {
                    if spec.feature_schema != corpus.feature_schema {
                        return Err(format!(
                            "shaping feature schema {} does not match corpus feature schema {}",
                            spec.feature_schema, corpus.feature_schema
                        )
                        .into());
                    }
                    spec.validate(corpus.feature_count as usize)?;
                }
                let merged_count = transitions
                    .len()
                    .checked_add(corpus.transitions.len())
                    .ok_or("learn fit merged transition count overflow")?;
                if merged_count > MAX_FQI_TRANSITIONS {
                    return Err(format!(
                        "learn fit accepts at most {MAX_FQI_TRANSITIONS} merged transitions; received at least {merged_count}"
                    )
                    .into());
                }
                transitions.reserve(corpus.transitions.len());
                let mut ended_terminal = false;
                for (transition_index, transition) in corpus.transitions.into_iter().enumerate() {
                    let action = transition.action.action_id;
                    let terminal = transition.terminal;
                    if !action_support.contains_key(&action)
                        && action_support.len() >= MAX_FQI_ACTIONS
                    {
                        return Err(format!(
                            "learn fit accepts at most {MAX_FQI_ACTIONS} distinct actions; encountered action {action} after reaching the limit"
                        )
                        .into());
                    }
                    *action_support.entry(action).or_default() += 1;
                    let reward = if let Some(spec) = &shaping_spec {
                        let breakdown = spec.shape_reward(
                            corpus.feature_count as usize,
                            &transition.state,
                            &transition.next_state,
                            transition.reward,
                            transition.duration_ticks,
                            terminal,
                            config.discount,
                        )?;
                        let training_reward = breakdown.training_reward;
                        shaping_records.push(json!({
                            "input_corpus": input,
                            "transition": transition_index,
                            "source_reference": transition.source.digest,
                            "next_reference": transition.next.digest,
                            "reward": breakdown,
                        }));
                        training_reward
                    } else {
                        transition.reward
                    };
                    transitions.push(FqiTransition {
                        state: transition.state,
                        action,
                        duration: transition.duration_ticks,
                        reward,
                        next_state: transition.next_state,
                        terminal,
                    });
                    episode_groups.push(next_episode_group);
                    ended_terminal = terminal;
                    if terminal {
                        next_episode_group = next_episode_group
                            .checked_add(1)
                            .ok_or("learn fit episode-group count overflowed")?;
                    }
                }
                if !ended_terminal {
                    next_episode_group = next_episode_group
                        .checked_add(1)
                        .ok_or("learn fit episode-group count overflowed")?;
                }
            }
            let declared_categorical = repeated_option(learn_args, "--categorical-feature")
                .into_iter()
                .map(|value| value.parse::<usize>())
                .collect::<Result<Vec<_>, _>>()?;
            let declared_all_continuous = learn_args.iter().any(|arg| arg == "--all-continuous");
            if declared_all_continuous && !declared_categorical.is_empty() {
                return Err(
                    "--all-continuous and --categorical-feature cannot be used together".into(),
                );
            }
            if feature_schema == Some(movement_feature_schema_digest_v1()) {
                if declared_all_continuous || !declared_categorical.is_empty() {
                    return Err(
                        "the authenticated movement schema owns its categorical feature map; do not override it"
                            .into(),
                    );
                }
                config.categorical_features = MOVEMENT_CATEGORICAL_FEATURES_V1.to_vec();
            } else if feature_schema == Some(movement_state_v2_spec().digest()?) {
                if declared_all_continuous || !declared_categorical.is_empty() {
                    return Err(
                        "the authenticated movement schema owns its categorical feature map; do not override it"
                            .into(),
                    );
                }
                config.categorical_features = movement_state_v2_spec().categorical_features();
            } else if declared_all_continuous {
                config.categorical_features.clear();
            } else if !declared_categorical.is_empty() {
                config.categorical_features = declared_categorical;
            } else {
                return Err(
                    "unknown feature schema: declare --all-continuous or repeat --categorical-feature N"
                        .into(),
                );
            }
            let actions: Vec<u32> = action_support.keys().copied().collect();
            let query_index = usize_option(learn_args, "--query-transition", 0)?;
            let query_transition = transitions
                .get(query_index)
                .ok_or("--query-transition is outside the merged transition batch")?;
            let query_side = option(learn_args, "--query-side").unwrap_or_else(|| "state".into());
            let query_state = match query_side.as_str() {
                "state" => query_transition.state.clone(),
                "next-state" => query_transition.next_state.clone(),
                _ => return Err("--query-side must be state or next-state".into()),
            };
            let learned_feature_count =
                feature_count.ok_or("transition corpus has no feature width")? as usize;
            let shaping_identity = shaping_spec
                .as_ref()
                .map(|spec| spec.identity(learned_feature_count))
                .transpose()?;
            if let (Some(spec), Some(path)) = (&shaping_spec, &shaping_report_path) {
                if path.exists() {
                    return Err(format!(
                        "shaping reward report already exists: {}",
                        path.display()
                    )
                    .into());
                }
                if let Some(parent) = path
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                {
                    fs::create_dir_all(parent)?;
                }
                fs::write(
                    path,
                    serde_json::to_vec_pretty(&json!({
                        "schema": REWARD_REPORT_SCHEMA_V1,
                        "shaping_identity": shaping_identity,
                        "shaping_spec": spec,
                        "feature_schema": feature_schema,
                        "action_schema": action_schema,
                        "per_tick_discount": config.discount,
                        "proposal_signal_only": true,
                        "terminal_objective": "unchanged_external_predicate",
                        "input_corpora": &inputs,
                        "transitions": shaping_records,
                    }))?,
                )?;
            }
            let model = FittedQ::fit_with_episode_groups(
                learned_feature_count,
                &actions,
                &transitions,
                &episode_groups,
                &config,
            )?;
            let model_output = option(learn_args, "--model-output").map(PathBuf::from);
            let mut model_content_blob = None;
            let mut model_artifact_store = None;
            if let Some(path) = &model_output {
                if path.exists() {
                    return Err(format!("model output already exists: {}", path.display()).into());
                }
                if let Some(parent) = path
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                {
                    fs::create_dir_all(parent)?;
                }
                let bytes = model.artifact_bytes(
                    feature_schema.ok_or("transition corpus has no feature schema")?,
                    action_schema.ok_or("transition corpus has no action schema")?,
                    dataset_manifest
                        .as_ref()
                        .map(|manifest| manifest.dataset_sha256),
                    &training_corpus_sha256,
                    &config,
                )?;
                fs::write(path, &bytes)?;
                let store_path = option(learn_args, "--artifact-store")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| path.parent().unwrap_or(Path::new(".")).join("content"));
                model_content_blob = Some(
                    ContentStore::initialize(&store_path)?.put_bytes(&bytes, ContentKind::Model)?,
                );
                model_artifact_store = Some(store_path);
            }
            let ranking: Vec<_> = model
                .rank_actions(&query_state)?
                .into_iter()
                .map(|estimate| {
                    json!({
                        "action": estimate.action,
                        "mean_q": estimate.mean,
                        "ensemble_variance": estimate.variance,
                        "support": action_support[&estimate.action],
                    })
                })
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": "dusklight-fitted-q-ranking/v1",
                    "feature_schema": feature_schema,
                    "action_schema": action_schema,
                    "input_corpora": inputs,
                    "training_dataset": dataset_path,
                    "training_dataset_sha256": dataset_manifest.as_ref().map(|manifest| manifest.dataset_sha256),
                    "transition_count": transitions.len(),
                    "episode_groups": episode_groups.iter().copied().collect::<BTreeSet<_>>().len(),
                    "bootstrap_unit": model.bootstrap_unit(),
                    "query_transition": query_index,
                    "query_side": query_side,
                    "per_tick_discount": config.discount,
                    "potential_shaping": shaping_identity,
                    "reward_report": shaping_report_path,
                    "model_output": model_output,
                    "model_artifact_store": model_artifact_store,
                    "model_content_blob": model_content_blob,
                    "iterations": config.iterations,
                    "backup_steps": config.backup_steps,
                    "trees_per_action": config.trees_per_action,
                    "categorical_features": config.categorical_features,
                    "seed": config.seed,
                    "ranking": ranking,
                }))?
            );
            Ok(())
        }
        Some("benchmark") => {
            const ADVANCE: u32 = 3;
            const WAIT: u32 = 9;
            let mut transitions = Vec::new();
            for nuisance in [-1.0, 1.0] {
                transitions.extend([
                    FqiTransition {
                        state: vec![0.0, nuisance],
                        action: ADVANCE,
                        duration: 1,
                        reward: 0.0,
                        next_state: vec![1.0, nuisance],
                        terminal: false,
                    },
                    FqiTransition {
                        state: vec![0.0, nuisance],
                        action: WAIT,
                        duration: 1,
                        reward: -1.0,
                        next_state: vec![0.0, nuisance],
                        terminal: false,
                    },
                    FqiTransition {
                        state: vec![1.0, nuisance],
                        action: ADVANCE,
                        duration: 1,
                        reward: 10.0,
                        next_state: vec![2.0, nuisance],
                        terminal: true,
                    },
                    FqiTransition {
                        state: vec![1.0, nuisance],
                        action: WAIT,
                        duration: 1,
                        reward: -1.0,
                        next_state: vec![1.0, nuisance],
                        terminal: false,
                    },
                ]);
            }
            let config = FqiConfig {
                iterations: 16,
                trees_per_action: 7,
                max_tree_depth: 3,
                features_per_split: 2,
                discount: 0.9,
                bootstrap: false,
                ..FqiConfig::default()
            };
            let model = FittedQ::fit(2, &[WAIT, ADVANCE], &transitions, &config)?;
            let held_out = [[0.0, 0.0], [1.0, 0.0]];
            let selected: Vec<u32> = held_out
                .iter()
                .map(|state| model.best_action(state).map(|estimate| estimate.action))
                .collect::<Result<_, _>>()?;
            let passed = selected == [ADVANCE, ADVANCE];
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": "dusklight-fqi-benchmark/v1",
                    "training_transitions": transitions.len(),
                    "held_out_states": held_out,
                    "selected_actions": selected,
                    "expected_actions": [ADVANCE, ADVANCE],
                    "passed": passed,
                }))?
            );
            if !passed {
                return Err("fitted-Q benchmark failed its fixed acceptance threshold".into());
            }
            Ok(())
        }
        _ => usage_error(),
    }
}
