//! Route timeline, content-addressed store, and workbench command adapters.

use crate::{flag, option, repeated_option, required_path, usage_error};
use huntctl::route_store::{ObjectId, RouteStore};
use huntctl::route_workbench::{WorkbenchConfig, prune_thumbnails, serve as serve_route_workbench};
use huntctl::timeline::Timeline;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::env;
use std::error::Error;
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) fn command_timeline(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("parse") if args.len() == 2 => {
            let path = PathBuf::from(&args[1]);
            let timeline = load_timeline(&path)?;
            timeline.validate_artifacts(path.parent())?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "valid": true,
                    "timeline": timeline.name,
                    "goals": timeline.goals.len(),
                    "proofs": timeline.proofs.len(),
                    "segments": timeline.segments.len(),
                    "continuations": timeline.continuations.len(),
                    "branches": timeline.branches.len(),
                }))?
            );
            Ok(())
        }
        Some("inspect") if args.len() == 2 => {
            let path = PathBuf::from(&args[1]);
            let timeline = load_timeline(&path)?;
            timeline.validate_artifacts(path.parent())?;
            println!("{}", serde_json::to_string_pretty(&timeline.inspect()?)?);
            Ok(())
        }
        Some("status") => {
            let timeline_args = &args[1..];
            let path = required_path(timeline_args, "--timeline")?;
            let timeline = load_timeline(&path)?;
            timeline.validate_artifacts(path.parent())?;
            let selections = timeline_selections(timeline_args)?;
            let status = timeline.status(
                option(timeline_args, "--continuation").as_deref(),
                &selections,
            )?;
            let output = serde_json::to_vec_pretty(&status)?;
            if let Some(path) = option(timeline_args, "--output") {
                fs::write(path, &output)?;
            }
            println!("{}", String::from_utf8(output)?);
            Ok(())
        }
        Some("rebase-compatible") => {
            let timeline_args = &args[1..];
            let path = required_path(timeline_args, "--timeline")?;
            let timeline = load_timeline(&path)?;
            timeline.validate_artifacts(path.parent())?;
            let continuation = option(timeline_args, "--continuation")
                .ok_or("missing required --continuation NAME")?;
            let name = option(timeline_args, "--name")
                .ok_or("missing required --name NEW_CONTINUATION")?;
            let selections = timeline_selections(timeline_args)?;
            if selections.is_empty() {
                return Err(
                    "rebase-compatible requires at least one --select ORIGINAL=REPLACEMENT".into(),
                );
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&timeline.rebase_compatible(
                    &continuation,
                    &selections,
                    &name,
                )?)?
            );
            Ok(())
        }
        Some("workbench") => command_timeline_workbench(&args[1..]),
        Some("store") => command_timeline_store(&args[1..]),
        Some("prune-thumbnails") => {
            let timeline_args = &args[1..];
            let path = required_path(timeline_args, "--timeline")?;
            let timeline = load_timeline(&path)?;
            let repository_root = option(timeline_args, "--repository-root")
                .map(PathBuf::from)
                .unwrap_or(env::current_dir()?);
            let state_root = required_path(timeline_args, "--state-root")?;
            let report = prune_thumbnails(
                &timeline,
                &path,
                &repository_root,
                &state_root,
                flag(timeline_args, "--apply"),
            )?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        _ => usage_error(),
    }
}

fn command_timeline_workbench(args: &[String]) -> Result<(), Box<dyn Error>> {
    let timeline_path = required_path(args, "--timeline")?;
    let game = required_path(args, "--game")?;
    let dvd = option(args, "--dvd")
        .map(PathBuf::from)
        .map(Ok)
        .unwrap_or_else(configured_dvd_path)?;
    let world_context = option(args, "--world-context").map(PathBuf::from);
    let state_root = option(args, "--state-root")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("build/automation-state/route-workbench"));
    let port = option(args, "--port")
        .map(|value| value.parse::<u16>())
        .transpose()?
        .unwrap_or(0);
    let working_directory = env::current_dir()?;
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    let address = listener.local_addr()?;
    let url = format!("http://{address}/");

    println!("Route Workbench: {url}");
    println!("Timeline: {}", timeline_path.display());
    println!("Ctrl+C stops the workbench; launched playback sessions keep running.");
    if !args.iter().any(|arg| arg == "--no-open") {
        open_browser(&url)?;
    }

    serve_route_workbench(
        listener,
        WorkbenchConfig {
            timeline_path,
            repository_root: working_directory.clone(),
            working_directory,
            game,
            dvd,
            world_context,
            state_root,
        },
    )?;
    Ok(())
}

fn open_browser(url: &str) -> Result<(), Box<dyn Error>> {
    #[cfg(target_os = "windows")]
    let mut command = {
        let brave = ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"]
            .into_iter()
            .filter_map(env::var_os)
            .map(PathBuf::from)
            .map(|root| root.join("BraveSoftware/Brave-Browser/Application/brave.exe"))
            .find(|path| path.is_file());
        if let Some(brave) = brave {
            let mut command = Command::new(brave);
            command.args(["--new-tab", url]);
            command
        } else {
            let mut command = Command::new("cmd");
            command.args(["/C", "start", "", url]);
            command
        }
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("open");
        command.arg(url);
        command
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut command = Command::new("xdg-open");
        command.arg(url);
        command
    };
    command.spawn()?;
    Ok(())
}

fn configured_dvd_path() -> Result<PathBuf, Box<dyn Error>> {
    let app_data = env::var_os("APPDATA")
        .map(PathBuf::from)
        .ok_or("--dvd is required when APPDATA is unavailable")?;
    let config_path = app_data.join("TwilitRealm/Dusklight/config.json");
    let config: Value = serde_json::from_slice(&fs::read(&config_path).map_err(|error| {
        format!(
            "--dvd was omitted and the Dusklight config {} could not be read: {error}",
            config_path.display()
        )
    })?)?;
    config
        .get("backend.isoPath")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| {
            format!(
                "--dvd was omitted and {} has no backend.isoPath",
                config_path.display()
            )
            .into()
        })
}

fn command_timeline_store(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("init") if args.len() == 2 => {
            RouteStore::initialize(&args[1])?;
            println!("initialized {}", args[1]);
            Ok(())
        }
        Some("import") => {
            let store_args = &args[1..];
            let root = required_path(store_args, "--store")?;
            let path = required_path(store_args, "--timeline")?;
            let reference = option(store_args, "--ref").ok_or("missing required --ref NAME")?;
            let timeline = load_timeline(&path)?;
            let result = RouteStore::open(root)?.import_timeline(
                &timeline,
                path.parent().unwrap_or_else(|| Path::new(".")),
                &reference,
            )?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            Ok(())
        }
        Some("fork") => {
            let store_args = &args[1..];
            let store = RouteStore::open(required_path(store_args, "--store")?)?;
            let from = option(store_args, "--from").ok_or("missing required --from REF")?;
            let to = option(store_args, "--to").ok_or("missing required --to REF")?;
            let id = if let Some(lineage) = option(store_args, "--lineage") {
                store.fork_lineage(&from, &lineage, &to)?
            } else {
                store.fork(&from, &to)?
            };
            println!("{id}");
            Ok(())
        }
        Some("import-evaluation") => {
            let store_args = &args[1..];
            let store = RouteStore::open(required_path(store_args, "--store")?)?;
            let path = required_path(store_args, "--evaluation")?;
            let segment =
                option(store_args, "--segment").ok_or("missing required --segment NAME")?;
            let fingerprint = option(store_args, "--fingerprint")
                .ok_or("missing required --fingerprint VALUE")?;
            let reference = option(store_args, "--ref");
            let id =
                store.import_evaluation(&path, &segment, &fingerprint, reference.as_deref())?;
            println!("{id}");
            Ok(())
        }
        Some("append") => {
            let store_args = &args[1..];
            let store = RouteStore::open(required_path(store_args, "--store")?)?;
            let reference = option(store_args, "--ref").ok_or("missing required --ref REF")?;
            let path = required_path(store_args, "--timeline")?;
            let continuation = option(store_args, "--continuation")
                .ok_or("missing required --continuation NAME")?;
            let timeline = load_timeline(&path)?;
            let id = store.append_lineage(
                &reference,
                &timeline,
                &continuation,
                path.parent().unwrap_or_else(|| Path::new(".")),
            )?;
            println!("{id}");
            Ok(())
        }
        Some("replay-repair") => {
            let store_args = &args[1..];
            let store = RouteStore::open(required_path(store_args, "--store")?)?;
            let from = option(store_args, "--from").ok_or("missing required --from REF")?;
            let to = option(store_args, "--to").ok_or("missing required --to REF")?;
            let path = required_path(store_args, "--timeline")?;
            let continuation = option(store_args, "--continuation")
                .ok_or("missing required --continuation NAME")?;
            let timeline = load_timeline(&path)?;
            let id = store.replay_repair(
                &from,
                &to,
                &timeline,
                &continuation,
                path.parent().unwrap_or_else(|| Path::new(".")),
            )?;
            println!("{id}");
            Ok(())
        }
        Some("promote") => {
            let store_args = &args[1..];
            let store = RouteStore::open(required_path(store_args, "--store")?)?;
            let reference = option(store_args, "--ref").ok_or("missing required --ref REF")?;
            let object: ObjectId = option(store_args, "--object")
                .ok_or("missing required --object ID")?
                .parse()?;
            store.promote(&reference, &object)?;
            println!("{object}");
            Ok(())
        }
        Some("resolve") => {
            let store_args = &args[1..];
            let store = RouteStore::open(required_path(store_args, "--store")?)?;
            let reference = option(store_args, "--ref").ok_or("missing required --ref REF")?;
            println!("{}", store.resolve_ref(&reference)?);
            Ok(())
        }
        Some("show") => {
            let store_args = &args[1..];
            let store = RouteStore::open(required_path(store_args, "--store")?)?;
            let object: ObjectId = option(store_args, "--object")
                .ok_or("missing required --object ID")?
                .parse()?;
            println!("{}", serde_json::to_string_pretty(&store.read(&object)?)?);
            Ok(())
        }
        Some("verify") => {
            let store_args = &args[1..];
            let store = RouteStore::open(required_path(store_args, "--store")?)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({"valid": true, "objects": store.verify()?}))?
            );
            Ok(())
        }
        Some("gc") => {
            let store_args = &args[1..];
            let store = RouteStore::open(required_path(store_args, "--store")?)?;
            let apply = store_args.iter().any(|arg| arg == "--apply");
            println!("{}", serde_json::to_string_pretty(&store.gc(apply)?)?);
            Ok(())
        }
        _ => usage_error(),
    }
}

fn load_timeline(path: impl AsRef<Path>) -> Result<Timeline, Box<dyn Error>> {
    Ok(Timeline::parse(&fs::read_to_string(path)?)?)
}

fn timeline_selections(args: &[String]) -> Result<BTreeMap<String, String>, Box<dyn Error>> {
    let mut output = BTreeMap::new();
    for selection in repeated_option(args, "--select") {
        let (original, replacement) = selection
            .split_once('=')
            .ok_or("--select must be ORIGINAL_SEGMENT=REPLACEMENT_SEGMENT")?;
        if original.is_empty() || replacement.is_empty() {
            return Err("--select segment IDs must be nonempty".into());
        }
        if output
            .insert(original.to_owned(), replacement.to_owned())
            .is_some()
        {
            return Err(format!("duplicate selection for segment {original}").into());
        }
    }
    Ok(output)
}
