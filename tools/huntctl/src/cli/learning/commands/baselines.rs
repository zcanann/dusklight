//! Offline baseline fitting, calibration, inspection, and benchmarks.

use super::{
    DatasetManifest, DatasetSplit, FittedQ, FqiConfig, FqiTransition, LocalFeature,
    LocalReturnConfig, MAX_FQI_ACTIONS, MAX_FQI_BACKUP_STEPS, MAX_FQI_ITERATIONS,
    MAX_FQI_TREE_DEPTH, MAX_FQI_TREES_PER_ACTION, MAX_LEARN_INPUT_CORPORA,
    MOVEMENT_CATEGORICAL_FEATURES_V1, NearestNeighborReturn, TabularAxis, TabularReturn,
    TransitionCorpus, calibrate_fitted_q, empirical_return_samples, load_fqi_batch,
    movement_feature_schema_digest_v1, movement_state_v2_spec, option, repeated_option,
    required_path, u64_option, usage_error, usize_option,
};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs;
use std::path::PathBuf;

pub(super) fn command(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("inspect") => {
            let corpus = TransitionCorpus::read_zstd_file(required_path(&args[1..], "--input")?)?;
            let mut action_counts = BTreeMap::<u32, usize>::new();
            let mut terminal_transitions = 0_usize;
            for transition in &corpus.transitions {
                *action_counts
                    .entry(transition.action.action_id)
                    .or_default() += 1;
                terminal_transitions += usize::from(transition.terminal);
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": "dusklight-transition-inspection/v1",
                    "content_digest": corpus.content_digest()?,
                    "feature_schema": corpus.feature_schema,
                    "action_schema": corpus.action_schema,
                    "feature_count": corpus.feature_count,
                    "transitions": corpus.transitions.len(),
                    "terminal_transitions": terminal_transitions,
                    "action_counts": action_counts,
                }))?
            );
            Ok(())
        }
        Some("baseline") => {
            let learn_args = &args[1..];
            let inputs = repeated_option(learn_args, "--input");
            if inputs.is_empty() || inputs.len() > MAX_LEARN_INPUT_CORPORA {
                return Err(format!(
                    "learn baseline requires 1..={MAX_LEARN_INPUT_CORPORA} --input corpora"
                )
                .into());
            }
            let method = option(learn_args, "--method")
                .ok_or("learn baseline requires --method nearest-neighbor|tabular")?;
            let discount = option(learn_args, "--discount")
                .map(|value| value.parse::<f32>())
                .transpose()?
                .unwrap_or(1.0);
            let mut feature_schema = None;
            let mut action_schema = None;
            let mut feature_count = None;
            let mut transitions = Vec::new();
            let mut episode_groups = Vec::new();
            let mut next_episode_group = 0_u64;
            for input in &inputs {
                let corpus = TransitionCorpus::read_zstd_file(input)?;
                if feature_schema.is_some_and(|value| value != corpus.feature_schema)
                    || action_schema.is_some_and(|value| value != corpus.action_schema)
                    || feature_count.is_some_and(|value| value != corpus.feature_count)
                {
                    return Err("baseline corpora use incompatible schemas".into());
                }
                feature_schema = Some(corpus.feature_schema);
                action_schema = Some(corpus.action_schema);
                feature_count = Some(corpus.feature_count);
                let mut ended_terminal = false;
                for transition in corpus.transitions {
                    let terminal = transition.terminal;
                    transitions.push(FqiTransition {
                        state: transition.state,
                        action: transition.action.action_id,
                        duration: transition.duration_ticks,
                        reward: transition.reward,
                        next_state: transition.next_state,
                        terminal,
                    });
                    episode_groups.push(next_episode_group);
                    ended_terminal = terminal;
                    if terminal {
                        next_episode_group = next_episode_group
                            .checked_add(1)
                            .ok_or("baseline episode-group count overflowed")?;
                    }
                }
                if !ended_terminal {
                    next_episode_group = next_episode_group
                        .checked_add(1)
                        .ok_or("baseline episode-group count overflowed")?;
                }
            }
            let query_index = usize_option(learn_args, "--query-transition", 0)?;
            let query = transitions
                .get(query_index)
                .ok_or("--query-transition is outside the merged transition batch")?;
            let query_side = option(learn_args, "--query-side").unwrap_or_else(|| "state".into());
            let query_state = match query_side.as_str() {
                "state" => &query.state,
                "next-state" => &query.next_state,
                _ => return Err("--query-side must be state or next-state".into()),
            };
            let samples = empirical_return_samples(&transitions, &episode_groups, discount)?;
            let (ranking, configuration) = match method.as_str() {
                "nearest-neighbor" => {
                    let declared = repeated_option(learn_args, "--feature");
                    let categorical = if feature_schema == Some(movement_feature_schema_digest_v1())
                    {
                        MOVEMENT_CATEGORICAL_FEATURES_V1.to_vec()
                    } else if feature_schema == Some(movement_state_v2_spec().digest()?) {
                        movement_state_v2_spec().categorical_features()
                    } else {
                        Vec::new()
                    };
                    let features = if declared.is_empty() {
                        if categorical.is_empty() {
                            return Err("unknown schema requires repeated --feature INDEX:SCALE:continuous|categorical".into());
                        }
                        (0..feature_count.unwrap() as usize)
                            .map(|index| LocalFeature {
                                index,
                                scale: 1.0,
                                categorical: categorical.contains(&index),
                            })
                            .collect::<Vec<_>>()
                    } else {
                        declared
                            .iter()
                            .map(|value| -> Result<LocalFeature, Box<dyn Error>> {
                                let parts = value.split(':').collect::<Vec<_>>();
                                if parts.len() != 3
                                    || !matches!(parts[2], "continuous" | "categorical")
                                {
                                    return Err(
                                        "--feature syntax is INDEX:SCALE:continuous|categorical"
                                            .into(),
                                    );
                                }
                                Ok(LocalFeature {
                                    index: parts[0].parse()?,
                                    scale: parts[1].parse()?,
                                    categorical: parts[2] == "categorical",
                                })
                            })
                            .collect::<Result<Vec<_>, _>>()?
                    };
                    let neighbors = usize_option(learn_args, "--neighbors", 8)?;
                    let model = NearestNeighborReturn::fit(
                        samples,
                        LocalReturnConfig {
                            neighbors,
                            features: features.clone(),
                        },
                    )?;
                    (
                        model.rank(query_state)?,
                        json!({
                            "neighbors": neighbors,
                            "features": features.iter().map(|feature| json!({
                                "index": feature.index,
                                "scale": feature.scale,
                                "categorical": feature.categorical,
                            })).collect::<Vec<_>>(),
                        }),
                    )
                }
                "tabular" => {
                    let axes = repeated_option(learn_args, "--axis")
                        .iter()
                        .map(|value| -> Result<TabularAxis, Box<dyn Error>> {
                            let parts = value.split(':').collect::<Vec<_>>();
                            if parts.len() != 3 {
                                return Err("--axis syntax is INDEX:ORIGIN:WIDTH".into());
                            }
                            Ok(TabularAxis {
                                index: parts[0].parse()?,
                                origin: parts[1].parse()?,
                                width: parts[2].parse()?,
                            })
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    let model = TabularReturn::fit(&samples, axes.clone())?;
                    (
                        model.rank(query_state)?,
                        json!({
                            "axes": axes.iter().map(|axis| json!({
                                "index": axis.index,
                                "origin": axis.origin,
                                "width": axis.width,
                            })).collect::<Vec<_>>(),
                        }),
                    )
                }
                _ => return Err("--method must be nearest-neighbor or tabular".into()),
            };
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": "dusklight-low-data-baseline/v1",
                    "method": method,
                    "feature_schema": feature_schema,
                    "action_schema": action_schema,
                    "input_corpora": inputs,
                    "episode_groups": episode_groups.iter().copied().collect::<BTreeSet<_>>().len(),
                    "transitions": transitions.len(),
                    "per_tick_discount": discount,
                    "query_transition": query_index,
                    "query_side": query_side,
                    "configuration": configuration,
                    "ranking": ranking,
                    "limitations": [
                        "empirical observed returns only; no counterfactual inference",
                        "a nonterminal episode end is truncated and receives no cross-episode bootstrap",
                        "rankings are proposal heuristics and require native rollout proof"
                    ]
                }))?
            );
            Ok(())
        }
        Some("calibrate") => {
            let learn_args = &args[1..];
            let dataset_path = option(learn_args, "--dataset").map(PathBuf::from);
            let explicit_training = repeated_option(learn_args, "--training");
            let explicit_held_out = repeated_option(learn_args, "--held-out");
            if dataset_path.is_some()
                == (!explicit_training.is_empty() || !explicit_held_out.is_empty())
            {
                return Err("learn calibrate requires either --dataset or both --training/--held-out corpora".into());
            }
            let mut dataset_identity = None;
            let mut held_out_split = None;
            let mut expected_dataset_corpus_digests = None;
            let (training_paths, held_out_paths) = if let Some(path) = &dataset_path {
                let manifest: DatasetManifest = serde_json::from_slice(&fs::read(path)?)?;
                manifest.validate()?;
                let split = match option(learn_args, "--split")
                    .unwrap_or_else(|| "test".into())
                    .as_str()
                {
                    "validation" => DatasetSplit::Validation,
                    "test" => DatasetSplit::Test,
                    "withheld" => DatasetSplit::Withheld,
                    _ => return Err("--split must be validation, test, or withheld".into()),
                };
                let training = manifest
                    .entries
                    .iter()
                    .filter(|entry| entry.split == DatasetSplit::Train)
                    .map(|entry| entry.transition_corpus.to_string_lossy().into_owned())
                    .collect::<Vec<_>>();
                let held_out = manifest
                    .entries
                    .iter()
                    .filter(|entry| entry.split == split)
                    .map(|entry| entry.transition_corpus.to_string_lossy().into_owned())
                    .collect::<Vec<_>>();
                expected_dataset_corpus_digests = Some((
                    manifest
                        .entries
                        .iter()
                        .filter(|entry| entry.split == DatasetSplit::Train)
                        .map(|entry| entry.corpus_sha256)
                        .collect::<Vec<_>>(),
                    manifest
                        .entries
                        .iter()
                        .filter(|entry| entry.split == split)
                        .map(|entry| entry.corpus_sha256)
                        .collect::<Vec<_>>(),
                ));
                dataset_identity = Some(manifest.dataset_sha256);
                held_out_split = Some(split);
                (training, held_out)
            } else {
                if explicit_training.is_empty() || explicit_held_out.is_empty() {
                    return Err(
                        "explicit calibration requires both --training and --held-out".into(),
                    );
                }
                (explicit_training, explicit_held_out)
            };
            let training_files = training_paths
                .iter()
                .map(fs::canonicalize)
                .collect::<Result<BTreeSet<_>, _>>()?;
            let held_out_files = held_out_paths
                .iter()
                .map(fs::canonicalize)
                .collect::<Result<BTreeSet<_>, _>>()?;
            if !training_files.is_disjoint(&held_out_files) {
                return Err("training and held-out calibration files overlap".into());
            }
            let training = load_fqi_batch(
                &training_paths,
                "calibration training",
                MAX_LEARN_INPUT_CORPORA,
            )?;
            let held_out = load_fqi_batch(
                &held_out_paths,
                "calibration held-out",
                MAX_LEARN_INPUT_CORPORA,
            )?;
            if expected_dataset_corpus_digests.as_ref().is_some_and(
                |(expected_training, expected_held_out)| {
                    expected_training != &training.corpus_digests
                        || expected_held_out != &held_out.corpus_digests
                },
            ) {
                return Err("calibration corpus content differs from dataset manifest".into());
            }
            if training.feature_schema != held_out.feature_schema
                || training.action_schema != held_out.action_schema
                || training.feature_count != held_out.feature_count
                || !training
                    .corpus_digests
                    .iter()
                    .all(|digest| !held_out.corpus_digests.contains(digest))
            {
                return Err(
                    "calibration requires compatible schemas and content-disjoint held-out corpora"
                        .into(),
                );
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
            if config.iterations == 0
                || config.iterations > MAX_FQI_ITERATIONS
                || config.backup_steps == 0
                || config.backup_steps > MAX_FQI_BACKUP_STEPS
                || config.trees_per_action == 0
                || config.trees_per_action > MAX_FQI_TREES_PER_ACTION
                || config.max_tree_depth > MAX_FQI_TREE_DEPTH
            {
                return Err("invalid bounded calibration FQI configuration".into());
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
            if training.feature_schema == movement_feature_schema_digest_v1() {
                if declared_all_continuous || !declared_categorical.is_empty() {
                    return Err(
                        "the authenticated movement schema owns its categorical feature map".into(),
                    );
                }
                config.categorical_features = MOVEMENT_CATEGORICAL_FEATURES_V1.to_vec();
            } else if training.feature_schema == movement_state_v2_spec().digest()? {
                if declared_all_continuous || !declared_categorical.is_empty() {
                    return Err(
                        "the authenticated movement schema owns its categorical feature map".into(),
                    );
                }
                config.categorical_features = movement_state_v2_spec().categorical_features();
            } else if declared_all_continuous {
                config.categorical_features.clear();
            } else if !declared_categorical.is_empty() {
                config.categorical_features = declared_categorical;
            } else {
                return Err("unknown feature schema: declare --all-continuous or repeat --categorical-feature N".into());
            }
            let actions = training
                .transitions
                .iter()
                .map(|transition| transition.action)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            if actions.is_empty() || actions.len() > MAX_FQI_ACTIONS {
                return Err("calibration training action support is outside bounds".into());
            }
            let model = FittedQ::fit_with_episode_groups(
                training.feature_count,
                &actions,
                &training.transitions,
                &training.episode_groups,
                &config,
            )?;
            let held_out_samples = empirical_return_samples(
                &held_out.transitions,
                &held_out.episode_groups,
                config.discount,
            )?;
            let calibration = calibrate_fitted_q(&model, &held_out_samples)?;
            let output_path = required_path(learn_args, "--output")?;
            if output_path.exists() {
                return Err(format!(
                    "calibration output already exists: {}",
                    output_path.display()
                )
                .into());
            }
            let report = json!({
                "schema": "dusklight-held-out-fqi-calibration/v1",
                "dataset": dataset_path,
                "dataset_sha256": dataset_identity,
                "held_out_split": held_out_split,
                "training_corpora": training_paths,
                "training_corpus_sha256": training.corpus_digests,
                "held_out_corpora": held_out_paths,
                "held_out_corpus_sha256": held_out.corpus_digests,
                "feature_schema": training.feature_schema,
                "action_schema": training.action_schema,
                "training_episode_groups": training.episode_groups.iter().copied().collect::<BTreeSet<_>>().len(),
                "held_out_episode_groups": held_out.episode_groups.iter().copied().collect::<BTreeSet<_>>().len(),
                "config": config,
                "calibration": calibration,
                "promotion_authority": false,
                "limitations": [
                    "exact-state proposal win rate is measured only where held-out actions are comparable",
                    "unsupported held-out actions and proposed actions remain explicit OOD diagnostics",
                    "calibration is analysis evidence and cannot replace native predicate or cold replay proof"
                ]
            });
            if let Some(parent) = output_path
                .parent()
                .filter(|path| !path.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            fs::write(&output_path, serde_json::to_vec_pretty(&report)?)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        _ => usage_error(),
    }
}
