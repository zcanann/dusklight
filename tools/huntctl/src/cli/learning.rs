//! CLI adapters for the offline-learning domain.

use huntctl::content_store::{ContentKind, ContentStore};
use huntctl::dataset::{DatasetManifest, DatasetSplit};
use huntctl::double_q::DoubleQConfig;
use huntctl::double_q::prioritized::{PrioritizedDoubleQ, PrioritizedDoubleQConfig};
use huntctl::iql::{ImplicitQ, IqlConfig};
use huntctl::learning::batch::load_fqi_batch;
use huntctl::learning::ensemble_q::{BootstrappedQConfig, BootstrappedQEnsemble};
use huntctl::learning::rainbow::{RainbowAblationConfig, RainbowAblationReport};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

const MAX_LEARN_INPUT_CORPORA: usize = 64;

pub fn command_q_ablation(learn_args: &[String]) -> Result<(), Box<dyn Error>> {
    let component = option(learn_args, "--component")
        .ok_or("learn ablate-q requires --component dueling-heads|n-step|distributional-values|noisy-exploration")?;
    let dataset_path = option(learn_args, "--dataset").map(PathBuf::from);
    let explicit_training = repeated_option(learn_args, "--training");
    let explicit_held_out = repeated_option(learn_args, "--held-out");
    if let Some(path) = &dataset_path {
        if !explicit_training.is_empty() || !explicit_held_out.is_empty() {
            return Err(
                "learn ablate-q accepts either --dataset or explicit --training/--held-out corpora"
                    .into(),
            );
        }
        if !path.exists() {
            return Err(format!("ablation dataset does not exist: {}", path.display()).into());
        }
    } else if explicit_training.is_empty() || explicit_held_out.is_empty() {
        return Err(
            "learn ablate-q requires either --dataset or both --training and --held-out corpora"
                .into(),
        );
    }

    let mut dataset_identity = None;
    let mut held_out_split = None;
    let mut expected_digests = None;
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
        expected_digests = Some((
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
        return Err("training and held-out ablation files overlap".into());
    }
    let training = load_fqi_batch(
        &training_paths,
        "Q component ablation training",
        MAX_LEARN_INPUT_CORPORA,
    )?;
    let held_out = load_fqi_batch(
        &held_out_paths,
        "Q component ablation held-out",
        MAX_LEARN_INPUT_CORPORA,
    )?;
    if expected_digests
        .as_ref()
        .is_some_and(|(expected_training, expected_held_out)| {
            expected_training != &training.corpus_digests
                || expected_held_out != &held_out.corpus_digests
        })
    {
        return Err("ablation corpus content differs from dataset manifest".into());
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
            "ablation requires compatible schemas and content-disjoint held-out corpora".into(),
        );
    }
    let actions = training
        .transitions
        .iter()
        .map(|transition| transition.action)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let defaults = RainbowAblationConfig::default();
    let config = RainbowAblationConfig {
        critic: DoubleQConfig {
            epochs: usize_option(learn_args, "--epochs", defaults.critic.epochs)?,
            hidden_width: usize_option(learn_args, "--hidden-width", defaults.critic.hidden_width)?,
            learning_rate: f64_option(
                learn_args,
                "--learning-rate",
                defaults.critic.learning_rate,
            )?,
            discount: f64_option(learn_args, "--discount", defaults.critic.discount)?,
            target_sync_steps: usize_option(
                learn_args,
                "--target-sync-steps",
                defaults.critic.target_sync_steps,
            )?,
            gradient_clip: f64_option(
                learn_args,
                "--gradient-clip",
                defaults.critic.gradient_clip,
            )?,
            seed: u64_option(learn_args, "--seed", defaults.critic.seed)?,
        },
        n_step: usize_option(learn_args, "--n-step", defaults.n_step)?,
        distribution_atoms: usize_option(
            learn_args,
            "--distribution-atoms",
            defaults.distribution_atoms,
        )?,
        distribution_value_minimum: f64_option(
            learn_args,
            "--distribution-min",
            defaults.distribution_value_minimum,
        )?,
        distribution_value_maximum: f64_option(
            learn_args,
            "--distribution-max",
            defaults.distribution_value_maximum,
        )?,
        noisy_initial_stddev: f64_option(
            learn_args,
            "--noisy-stddev",
            defaults.noisy_initial_stddev,
        )?,
    };
    let ablation = match component.as_str() {
        "dueling-heads" => RainbowAblationReport::evaluate_dueling_heads(
            training.feature_count,
            &actions,
            &training.transitions,
            &training.episode_groups,
            &held_out.transitions,
            &config,
        )?,
        "n-step" => RainbowAblationReport::evaluate_n_step(
            training.feature_count,
            &actions,
            &training.transitions,
            &training.episode_groups,
            &held_out.transitions,
            &config,
        )?,
        "distributional-values" => RainbowAblationReport::evaluate_distributional_values(
            training.feature_count,
            &actions,
            &training.transitions,
            &training.episode_groups,
            &held_out.transitions,
            &config,
        )?,
        "noisy-exploration" => RainbowAblationReport::evaluate_noisy_exploration(
            training.feature_count,
            &actions,
            &training.transitions,
            &training.episode_groups,
            &held_out.transitions,
            &config,
        )?,
        _ => {
            return Err("--component must be dueling-heads, n-step, distributional-values, or noisy-exploration".into());
        }
    };
    let output = option(learn_args, "--output")
        .map(PathBuf::from)
        .ok_or("learn ablate-q requires --output REPORT.json")?;
    if output.exists() {
        return Err(format!("ablation output already exists: {}", output.display()).into());
    }
    let report = json!({
        "schema": "dusklight-q-component-ablation-run/v1",
        "component": component,
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
        "ablation": ablation,
    });
    if let Some(parent) = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(&report)?;
    fs::write(&output, &bytes)?;
    println!("{}", String::from_utf8(bytes)?);
    Ok(())
}

pub fn command_iql(learn_args: &[String]) -> Result<(), Box<dyn Error>> {
    let direct_inputs = repeated_option(learn_args, "--input");
    let dataset_path = option(learn_args, "--dataset").map(PathBuf::from);
    if dataset_path.is_some() && !direct_inputs.is_empty() {
        return Err("learn iql accepts either --dataset or --input, not both".into());
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
    let training = load_fqi_batch(&inputs, "IQL training", MAX_LEARN_INPUT_CORPORA)?;
    if dataset_manifest.as_ref().is_some_and(|manifest| {
        manifest
            .entries
            .iter()
            .filter(|entry| entry.split == DatasetSplit::Train)
            .map(|entry| entry.corpus_sha256)
            .collect::<Vec<_>>()
            != training.corpus_digests
    }) {
        return Err("IQL corpus content differs from dataset manifest".into());
    }
    let defaults = IqlConfig::default();
    let config = IqlConfig {
        epochs: usize_option(learn_args, "--epochs", defaults.epochs)?,
        hidden_width: usize_option(learn_args, "--hidden-width", defaults.hidden_width)?,
        learning_rate: option(learn_args, "--learning-rate")
            .map(|value| value.parse::<f64>())
            .transpose()?
            .unwrap_or(defaults.learning_rate),
        discount: option(learn_args, "--discount")
            .map(|value| value.parse::<f64>())
            .transpose()?
            .unwrap_or(defaults.discount),
        expectile: option(learn_args, "--expectile")
            .map(|value| value.parse::<f64>())
            .transpose()?
            .unwrap_or(defaults.expectile),
        advantage_inverse_temperature: option(learn_args, "--advantage-beta")
            .map(|value| value.parse::<f64>())
            .transpose()?
            .unwrap_or(defaults.advantage_inverse_temperature),
        max_advantage_weight: option(learn_args, "--max-advantage-weight")
            .map(|value| value.parse::<f64>())
            .transpose()?
            .unwrap_or(defaults.max_advantage_weight),
        target_sync_steps: usize_option(
            learn_args,
            "--target-sync-steps",
            defaults.target_sync_steps,
        )?,
        gradient_clip: option(learn_args, "--gradient-clip")
            .map(|value| value.parse::<f64>())
            .transpose()?
            .unwrap_or(defaults.gradient_clip),
        seed: u64_option(learn_args, "--seed", defaults.seed)?,
    };
    let action_support = training.transitions.iter().fold(
        BTreeMap::<u32, usize>::new(),
        |mut counts, transition| {
            *counts.entry(transition.action).or_default() += 1;
            counts
        },
    );
    let actions = action_support.keys().copied().collect::<Vec<_>>();
    let model = ImplicitQ::fit(
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
                "policy_probability": estimate.policy_probability,
                "mean_q": estimate.mean_q,
                "value": estimate.value,
                "advantage": estimate.advantage,
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
            return Err(format!("IQL model output already exists: {}", path.display()).into());
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
        model_content_blob =
            Some(ContentStore::initialize(&store_path)?.put_bytes(&bytes, ContentKind::Model)?);
        model_artifact_store = Some(store_path);
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": "dusklight-discrete-iql-ranking/v1",
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
            "mean_advantage_weight": model.mean_advantage_weight(),
            "clipped_advantage_weights": model.clipped_advantage_weights(),
            "policy_objective": "clipped_advantage_weighted_behavior_cloning",
            "model_output": model_output,
            "model_artifact_store": model_artifact_store,
            "model_content_blob": model_content_blob,
            "ranking": ranking,
            "promotion_authority": false,
            "limitations": [
                "the policy is trained only from logged actions but function approximation can generalize across states",
                "critic disagreement and policy probabilities are not calibrated safety estimates",
                "rankings are proposals and require native predicate and cold replay proof"
            ]
        }))?
    );
    Ok(())
}

pub fn command_ensemble_q(learn_args: &[String]) -> Result<(), Box<dyn Error>> {
    let direct_inputs = repeated_option(learn_args, "--input");
    let dataset_path = option(learn_args, "--dataset").map(PathBuf::from);
    if dataset_path.is_some() && !direct_inputs.is_empty() {
        return Err("learn ensemble-q accepts either --dataset or --input, not both".into());
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
    let training = load_fqi_batch(&inputs, "ensemble-Q training", MAX_LEARN_INPUT_CORPORA)?;
    if dataset_manifest.as_ref().is_some_and(|manifest| {
        manifest
            .entries
            .iter()
            .filter(|entry| entry.split == DatasetSplit::Train)
            .map(|entry| entry.corpus_sha256)
            .collect::<Vec<_>>()
            != training.corpus_digests
    }) {
        return Err("ensemble-Q corpus content differs from dataset manifest".into());
    }
    let defaults = BootstrappedQConfig::default();
    let config = BootstrappedQConfig {
        members: usize_option(learn_args, "--members", defaults.members)?,
        seed: u64_option(learn_args, "--seed", defaults.seed)?,
        critic: DoubleQConfig {
            epochs: usize_option(learn_args, "--epochs", defaults.critic.epochs)?,
            hidden_width: usize_option(learn_args, "--hidden-width", defaults.critic.hidden_width)?,
            learning_rate: option(learn_args, "--learning-rate")
                .map(|value| value.parse::<f64>())
                .transpose()?
                .unwrap_or(defaults.critic.learning_rate),
            discount: option(learn_args, "--discount")
                .map(|value| value.parse::<f64>())
                .transpose()?
                .unwrap_or(defaults.critic.discount),
            target_sync_steps: usize_option(
                learn_args,
                "--target-sync-steps",
                defaults.critic.target_sync_steps,
            )?,
            gradient_clip: option(learn_args, "--gradient-clip")
                .map(|value| value.parse::<f64>())
                .transpose()?
                .unwrap_or(defaults.critic.gradient_clip),
            seed: u64_option(learn_args, "--critic-seed", defaults.critic.seed)?,
        },
    };
    let action_support = training.transitions.iter().fold(
        BTreeMap::<u32, usize>::new(),
        |mut counts, transition| {
            *counts.entry(transition.action).or_default() += 1;
            counts
        },
    );
    let actions = action_support.keys().copied().collect::<Vec<_>>();
    let model = BootstrappedQEnsemble::fit(
        training.feature_count,
        &actions,
        &training.transitions,
        &training.episode_groups,
        &config,
    )?;
    let query_index = usize_option(learn_args, "--query-transition", 0)?;
    let query = training
        .transitions
        .get(query_index)
        .ok_or("--query-transition is outside the merged transition batch")?;
    let query_side = option(learn_args, "--query-side").unwrap_or_else(|| "state".into());
    let query_state = match query_side.as_str() {
        "state" => &query.state,
        "next-state" => &query.next_state,
        _ => return Err("--query-side must be state or next-state".into()),
    };
    let ranking = model
        .rank_actions(query_state)?
        .into_iter()
        .map(|estimate| {
            json!({
                "action": estimate.action,
                "mean_q": estimate.mean_q,
                "member_variance": estimate.member_variance,
                "mean_twin_disagreement": estimate.mean_twin_disagreement,
                "support": action_support[&estimate.action],
            })
        })
        .collect::<Vec<_>>();
    let model_output = option(learn_args, "--model-output").map(PathBuf::from);
    let mut model_content_blob = None;
    if let Some(path) = &model_output {
        if path.exists() {
            return Err(
                format!("ensemble-Q model output already exists: {}", path.display()).into(),
            );
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
        model_content_blob =
            Some(ContentStore::initialize(&store_path)?.put_bytes(&bytes, ContentKind::Model)?);
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": "dusklight-bootstrapped-q-ranking/v1",
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
            "members": model.members(),
            "member_bootstrap_episode_groups": model.member_bootstrap_episode_groups(),
            "support_repair_episode_draws": model.support_repair_episode_draws(),
            "model_output": model_output,
            "model_content_blob": model_content_blob,
            "ranking": ranking,
            "promotion_authority": false,
            "limitations": [
                "member variance and twin disagreement are uncalibrated sampling diagnostics",
                "support repair appends whole episodes and does not synthesize transition rows",
                "rankings are proposals and require native predicate and cold replay proof"
            ]
        }))?
    );
    Ok(())
}

pub fn command_prioritized_q(learn_args: &[String]) -> Result<(), Box<dyn Error>> {
    let direct_inputs = repeated_option(learn_args, "--input");
    let dataset_path = option(learn_args, "--dataset").map(PathBuf::from);
    if dataset_path.is_some() && !direct_inputs.is_empty() {
        return Err("learn prioritized-q accepts either --dataset or --input, not both".into());
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
    let training = load_fqi_batch(
        &inputs,
        "prioritized Double-Q training",
        MAX_LEARN_INPUT_CORPORA,
    )?;
    if dataset_manifest.as_ref().is_some_and(|manifest| {
        manifest
            .entries
            .iter()
            .filter(|entry| entry.split == DatasetSplit::Train)
            .map(|entry| entry.corpus_sha256)
            .collect::<Vec<_>>()
            != training.corpus_digests
    }) {
        return Err("prioritized Double-Q corpus content differs from dataset manifest".into());
    }

    let defaults = PrioritizedDoubleQConfig::default();
    let config = PrioritizedDoubleQConfig {
        critic: DoubleQConfig {
            epochs: usize_option(learn_args, "--epochs", defaults.critic.epochs)?,
            hidden_width: usize_option(learn_args, "--hidden-width", defaults.critic.hidden_width)?,
            learning_rate: f64_option(
                learn_args,
                "--learning-rate",
                defaults.critic.learning_rate,
            )?,
            discount: f64_option(learn_args, "--discount", defaults.critic.discount)?,
            target_sync_steps: usize_option(
                learn_args,
                "--target-sync-steps",
                defaults.critic.target_sync_steps,
            )?,
            gradient_clip: f64_option(
                learn_args,
                "--gradient-clip",
                defaults.critic.gradient_clip,
            )?,
            seed: u64_option(learn_args, "--seed", defaults.critic.seed)?,
        },
        priority_exponent: f64_option(learn_args, "--priority-alpha", defaults.priority_exponent)?,
        importance_exponent_start: f64_option(
            learn_args,
            "--importance-beta-start",
            defaults.importance_exponent_start,
        )?,
        importance_exponent_end: f64_option(
            learn_args,
            "--importance-beta-end",
            defaults.importance_exponent_end,
        )?,
        priority_epsilon: f64_option(learn_args, "--priority-epsilon", defaults.priority_epsilon)?,
        importance_weight_cap: f64_option(
            learn_args,
            "--importance-weight-cap",
            defaults.importance_weight_cap,
        )?,
        replay_seed: u64_option(learn_args, "--replay-seed", defaults.replay_seed)?,
    };
    let action_support = training.transitions.iter().fold(
        BTreeMap::<u32, usize>::new(),
        |mut counts, transition| {
            *counts.entry(transition.action).or_default() += 1;
            counts
        },
    );
    let actions = action_support.keys().copied().collect::<Vec<_>>();
    let model = PrioritizedDoubleQ::fit(
        training.feature_count,
        &actions,
        &training.transitions,
        &config,
    )?;
    let query_index = usize_option(learn_args, "--query-transition", 0)?;
    let query = training
        .transitions
        .get(query_index)
        .ok_or("--query-transition is outside the merged transition batch")?;
    let query_side = option(learn_args, "--query-side").unwrap_or_else(|| "state".into());
    let query_state = match query_side.as_str() {
        "state" => &query.state,
        "next-state" => &query.next_state,
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
                "prioritized Double-Q model output already exists: {}",
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
        model_content_blob =
            Some(ContentStore::initialize(&store_path)?.put_bytes(&bytes, ContentKind::Model)?);
        model_artifact_store = Some(store_path);
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": "dusklight-prioritized-double-q-ranking/v1",
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
            "replay_diagnostics": model.diagnostics(),
            "row_sample_counts": model.row_sample_counts(),
            "model_output": model_output,
            "model_artifact_store": model_artifact_store,
            "model_content_blob": model_content_blob,
            "ranking": ranking,
            "promotion_authority": false,
            "limitations": [
                "priorities are online absolute TD-error diagnostics, not calibrated value uncertainty",
                "importance correction is deliberately capped and therefore remains biased",
                "rankings are proposals and require native predicate and cold replay proof"
            ]
        }))?
    );
    Ok(())
}

fn option(args: &[String], name: &str) -> Option<String> {
    args.windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].clone())
}

fn repeated_option(args: &[String], name: &str) -> Vec<String> {
    args.windows(2)
        .filter(|pair| pair[0] == name)
        .map(|pair| pair[1].clone())
        .collect()
}

fn usize_option(args: &[String], name: &str, default: usize) -> Result<usize, Box<dyn Error>> {
    Ok(option(args, name)
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(default))
}

fn u64_option(args: &[String], name: &str, default: u64) -> Result<u64, Box<dyn Error>> {
    Ok(option(args, name)
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(default))
}

fn f64_option(args: &[String], name: &str, default: f64) -> Result<f64, Box<dyn Error>> {
    Ok(option(args, name)
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(default))
}
