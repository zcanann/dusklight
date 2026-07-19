//! Immutable tape, transition-corpus, content-store, and episode-store adapters.

use crate::{flag, option, repeated_option, required_path, usage_error, usize_option};
use huntctl::content_store::ContentStore;
use huntctl::corpus::Corpus;
use huntctl::episode_store::EpisodeStore;
use huntctl::observation_view::MOVEMENT_STATE_V2_ID;
use huntctl::transition_corpus::TransitionCorpus;
use huntctl::{BuildIdentity, Digest};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
use std::error::Error;
use std::fs;
use std::path::PathBuf;

pub(crate) fn command_corpus(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("init") if args.len() == 2 => {
            let corpus = Corpus::initialize(&args[1])?;
            println!("initialized {}", corpus.root().display());
            Ok(())
        }
        Some("ingest") if args.len() >= 2 => {
            let corpus = Corpus::open(&args[1])?;
            let tape_path = required_path(args, "--tape")?;
            let build_path = required_path(args, "--build")?;
            let scenario = option(args, "--scenario").ok_or("missing required --scenario ID")?;
            let build: BuildIdentity = serde_json::from_slice(&fs::read(build_path)?)?;
            let metadata = if let Some(path) = option(args, "--scenario-json") {
                serde_json::from_slice(&fs::read(path)?)?
            } else {
                json!({})
            };
            let result = corpus.ingest(&fs::read(tape_path)?, build, scenario, metadata)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "artifact_id": result.artifact_id,
                    "tape_digest": result.tape_digest,
                    "created": result.created
                }))?
            );
            Ok(())
        }
        Some("list") if args.len() == 2 => {
            let artifacts: Vec<Value> = Corpus::open(&args[1])?
                .list()?
                .into_iter()
                .map(|artifact| {
                    json!({
                        "artifact_id": artifact.artifact_id,
                        "scenario": artifact.manifest.scenario.id,
                        "frame_count": artifact.manifest.frame_count,
                        "tape_digest": artifact.manifest.tape.digest
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&artifacts)?);
            Ok(())
        }
        Some("show") if args.len() == 3 => {
            let artifact_id: Digest = args[2].parse()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&Corpus::open(&args[1])?.show(artifact_id)?)?
            );
            Ok(())
        }
        Some("verify") if args.len() == 2 => {
            println!(
                "{}",
                serde_json::to_string_pretty(&Corpus::open(&args[1])?.verify()?)?
            );
            Ok(())
        }
        Some("query") => {
            let inputs = repeated_option(&args[1..], "--input")
                .into_iter()
                .map(PathBuf::from)
                .collect::<Vec<_>>();
            let action = option(&args[1..], "--action")
                .map(|value| value.parse())
                .transpose()?;
            let terminal = option(&args[1..], "--terminal")
                .map(|value| value.parse::<bool>())
                .transpose()?;
            let minimum_reward = option(&args[1..], "--minimum-reward")
                .map(|value| value.parse::<f32>())
                .transpose()?;
            let rows = huntctl::corpus_ops::query(
                &inputs,
                action,
                terminal,
                minimum_reward,
                usize_option(&args[1..], "--limit", 1000)?,
            )?;
            println!("{}", serde_json::to_string_pretty(&rows)?);
            Ok(())
        }
        Some("compare") => {
            let report = huntctl::corpus_ops::compare(
                &required_path(&args[1..], "--left")?,
                &required_path(&args[1..], "--right")?,
            )?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Some("merge" | "compact") => {
            let inputs = repeated_option(&args[1..], "--input")
                .into_iter()
                .map(PathBuf::from)
                .collect::<Vec<_>>();
            let output = required_path(&args[1..], "--output")?;
            let level = if args[0] == "compact" { 19 } else { 3 };
            let report = huntctl::corpus_ops::merge(&inputs, &output, level)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Some("shard") => {
            let report = huntctl::corpus_ops::shard(
                &required_path(&args[1..], "--input")?,
                &required_path(&args[1..], "--output-directory")?,
                usize_option(&args[1..], "--maximum-transitions", 100_000)?,
                3,
            )?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Some("refeature") => {
            let descriptor = huntctl::corpus_ops::refeature(
                &required_path(&args[1..], "--source")?,
                &required_path(&args[1..], "--output")?,
                &option(&args[1..], "--view").unwrap_or_else(|| MOVEMENT_STATE_V2_ID.into()),
            )?;
            println!("{}", serde_json::to_string_pretty(&descriptor)?);
            Ok(())
        }
        Some("validate-transitions") => {
            let inputs = repeated_option(&args[1..], "--input");
            let mut reports = Vec::new();
            for input in inputs {
                let corpus = TransitionCorpus::read_zstd_file(&input)?;
                reports.push(json!({
                    "input": input,
                    "content_sha256": corpus.content_digest()?,
                    "feature_schema": corpus.feature_schema,
                    "action_schema": corpus.action_schema,
                    "transitions": corpus.transitions.len(),
                }));
            }
            println!("{}", serde_json::to_string_pretty(&reports)?);
            Ok(())
        }
        Some("quarantine") => {
            let inputs = repeated_option(&args[1..], "--input")
                .into_iter()
                .map(PathBuf::from)
                .collect::<Vec<_>>();
            let report = huntctl::corpus_ops::quarantine_invalid(
                &inputs,
                &required_path(&args[1..], "--quarantine-root")?,
                !args[1..].iter().any(|argument| argument == "--apply"),
            )?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Some("gc-content") => {
            let corpus_args = &args[1..];
            let store_root = required_path(corpus_args, "--store")?;
            let trash_root = required_path(corpus_args, "--trash-root")?;
            let mut referenced = BTreeSet::new();
            for value in repeated_option(corpus_args, "--reference") {
                referenced.insert(value.parse::<Digest>()?);
            }
            let manifests = repeated_option(corpus_args, "--manifest");
            if manifests.is_empty() && referenced.is_empty() {
                return Err(
                    "corpus gc-content requires at least one --manifest or --reference".into(),
                );
            }
            for manifest in manifests {
                let bytes = fs::read(&manifest)?;
                referenced.insert(Digest(Sha256::digest(&bytes).into()));
                collect_json_digests(&serde_json::from_slice(&bytes)?, &mut referenced);
            }
            let report = ContentStore::initialize(store_root)?.garbage_collect(
                &referenced,
                &trash_root,
                !flag(corpus_args, "--apply"),
            )?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Some("verify-episodes") => {
            let corpus_args = &args[1..];
            let report =
                EpisodeStore::open(required_path(corpus_args, "--store")?)?.verify_all()?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Some("gc-episodes") => {
            let corpus_args = &args[1..];
            let store = EpisodeStore::open(required_path(corpus_args, "--store")?)?;
            let trash_root = required_path(corpus_args, "--trash-root")?;
            let retained = repeated_option(corpus_args, "--retain-episode")
                .into_iter()
                .map(|value| value.parse::<Digest>())
                .collect::<Result<BTreeSet<_>, _>>()?;
            let additionally_referenced = repeated_option(corpus_args, "--reference")
                .into_iter()
                .map(|value| value.parse::<Digest>())
                .collect::<Result<BTreeSet<_>, _>>()?;
            let report = store.garbage_collect(
                &retained,
                &additionally_referenced,
                &trash_root,
                !flag(corpus_args, "--apply"),
            )?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Some("export-arrow") => {
            let corpus_args = &args[1..];
            let inputs = repeated_option(corpus_args, "--input")
                .into_iter()
                .map(PathBuf::from)
                .collect::<Vec<_>>();
            let report = huntctl::corpus_ops::export_arrow(
                &inputs,
                &required_path(corpus_args, "--output")?,
            )?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        _ => usage_error(),
    }
}

fn collect_json_digests(value: &Value, output: &mut BTreeSet<Digest>) {
    match value {
        Value::String(value) => {
            if let Ok(digest) = value.parse() {
                output.insert(digest);
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_json_digests(value, output);
            }
        }
        Value::Object(values) => {
            for value in values.values() {
                collect_json_digests(value, output);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}
