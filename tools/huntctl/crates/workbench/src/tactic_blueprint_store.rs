//! File-backed authored tactic blueprints and immutable built-in catalogue projection.

use super::*;
use dusklight_learning::default_tactic_catalog::default_route_tactic_catalog;
use dusklight_learning::tactic_blueprint::{TacticBlueprint, TacticBlueprintNode};
use std::fs::OpenOptions;

const TACTIC_CATALOG_SCHEMA: &str = "dusklight.route-workbench.tactic-catalog.v1";
const MAX_AUTHORED_TACTICS: usize = 256;
const MAX_TACTIC_FILE_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BrowserTacticCreateRequest {
    pub asset_id: String,
    pub steps: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BrowserTacticUpdateRequest {
    pub asset_id: String,
    pub revision: String,
    pub steps: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BrowserTacticRenameRequest {
    pub asset_id: String,
    pub revision: String,
    pub new_asset_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BrowserTacticDuplicateRequest {
    pub asset_id: String,
    pub revision: String,
    pub new_asset_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BrowserTacticDeleteRequest {
    pub asset_id: String,
    pub revision: String,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct TacticMutationResponse {
    pub schema: &'static str,
    pub asset_id: String,
    pub revision: Option<String>,
    pub status: &'static str,
}

pub(super) fn tactic_catalog_projection(
    repository_root: &Path,
    timeline_path: &Path,
) -> Result<GraphTacticCatalog, WorkbenchError> {
    let catalog = default_route_tactic_catalog().map_err(tactic_store_error)?;
    let built_ins = catalog
        .entries()
        .iter()
        .map(|entry| {
            let description = entry.description();
            GraphTacticAsset {
                asset_id: entry.option_id().into(),
                source: "library".into(),
                read_only: true,
                revision: description.content_sha256.to_string(),
                kind: serde_json::to_value(description.kind)
                    .ok()
                    .and_then(|value| value.as_str().map(str::to_owned))
                    .unwrap_or_else(|| "built_in".into()),
                minimum_ticks: Some(description.duration.minimum_ticks),
                maximum_ticks: Some(description.duration.maximum_ticks),
                steps: Vec::new(),
                error: None,
            }
        })
        .collect();
    let authored = load_authored_records(repository_root, timeline_path)?
        .into_iter()
        .map(|record| record.projection)
        .collect();
    Ok(GraphTacticCatalog {
        schema: TACTIC_CATALOG_SCHEMA.into(),
        built_ins,
        authored,
    })
}

pub(super) fn load_authored_tactic_blueprints(
    repository_root: &Path,
    timeline_path: &Path,
) -> Result<Vec<TacticBlueprint>, WorkbenchError> {
    let records = load_authored_records(repository_root, timeline_path)?;
    if let Some(invalid) = records
        .iter()
        .find(|record| record.projection.error.is_some())
    {
        return Err(WorkbenchError::new(format!(
            "authored tactic {:?} is invalid: {}",
            invalid.projection.asset_id,
            invalid.projection.error.as_deref().unwrap_or("invalid")
        )));
    }
    Ok(records
        .into_iter()
        .filter_map(|record| record.blueprint)
        .collect())
}

pub(super) fn create_authored_tactic(
    config: &WorkbenchConfig,
    request: &BrowserTacticCreateRequest,
) -> Result<TacticMutationResponse, WorkbenchError> {
    let _edit = tactic_store_edits()
        .lock()
        .map_err(|_| WorkbenchError::new("tactic asset edit lock is unavailable"))?;
    let root = tactic_store_root(&config.repository_root, &config.timeline_path)?;
    fs::create_dir_all(&root).map_err(tactic_store_error)?;
    ensure_store_capacity(&root)?;
    let blueprint = sequence_blueprint(&request.asset_id, &request.steps)?;
    let path = tactic_asset_path(&root, &request.asset_id)?;
    write_new_blueprint(&path, &blueprint)?;
    mutation_response(blueprint, "created")
}

pub(super) fn update_authored_tactic(
    config: &WorkbenchConfig,
    request: &BrowserTacticUpdateRequest,
) -> Result<TacticMutationResponse, WorkbenchError> {
    let _edit = tactic_store_edits()
        .lock()
        .map_err(|_| WorkbenchError::new("tactic asset edit lock is unavailable"))?;
    let root = tactic_store_root(&config.repository_root, &config.timeline_path)?;
    let path = tactic_asset_path(&root, &request.asset_id)?;
    let current = read_blueprint(&path)?;
    require_revision(&current, &request.revision)?;
    let replacement = sequence_blueprint(&request.asset_id, &request.steps)?;
    replace_blueprint(&path, &replacement)?;
    mutation_response(replacement, "updated")
}

pub(super) fn rename_authored_tactic(
    config: &WorkbenchConfig,
    request: &BrowserTacticRenameRequest,
) -> Result<TacticMutationResponse, WorkbenchError> {
    let _edit = tactic_store_edits()
        .lock()
        .map_err(|_| WorkbenchError::new("tactic asset edit lock is unavailable"))?;
    let root = tactic_store_root(&config.repository_root, &config.timeline_path)?;
    let source = tactic_asset_path(&root, &request.asset_id)?;
    let current = read_blueprint(&source)?;
    require_revision(&current, &request.revision)?;
    let renamed = TacticBlueprint::new(request.new_asset_id.clone(), current.root.clone())
        .map_err(tactic_store_error)?;
    renamed
        .validate_against_catalog(&default_route_tactic_catalog().map_err(tactic_store_error)?)
        .map_err(tactic_store_error)?;
    let target = tactic_asset_path(&root, &request.new_asset_id)?;
    write_new_blueprint(&target, &renamed)?;
    if let Err(error) = move_to_trash(&root, &source) {
        let _ = fs::remove_file(&target);
        return Err(error);
    }
    mutation_response(renamed, "renamed")
}

pub(super) fn duplicate_authored_tactic(
    config: &WorkbenchConfig,
    request: &BrowserTacticDuplicateRequest,
) -> Result<TacticMutationResponse, WorkbenchError> {
    let _edit = tactic_store_edits()
        .lock()
        .map_err(|_| WorkbenchError::new("tactic asset edit lock is unavailable"))?;
    let root = tactic_store_root(&config.repository_root, &config.timeline_path)?;
    ensure_store_capacity(&root)?;
    let source = tactic_asset_path(&root, &request.asset_id)?;
    let current = read_blueprint(&source)?;
    require_revision(&current, &request.revision)?;
    let duplicate = TacticBlueprint::new(request.new_asset_id.clone(), current.root.clone())
        .map_err(tactic_store_error)?;
    duplicate
        .validate_against_catalog(&default_route_tactic_catalog().map_err(tactic_store_error)?)
        .map_err(tactic_store_error)?;
    write_new_blueprint(
        &tactic_asset_path(&root, &request.new_asset_id)?,
        &duplicate,
    )?;
    mutation_response(duplicate, "duplicated")
}

pub(super) fn delete_authored_tactic(
    config: &WorkbenchConfig,
    request: &BrowserTacticDeleteRequest,
) -> Result<TacticMutationResponse, WorkbenchError> {
    let _edit = tactic_store_edits()
        .lock()
        .map_err(|_| WorkbenchError::new("tactic asset edit lock is unavailable"))?;
    let root = tactic_store_root(&config.repository_root, &config.timeline_path)?;
    let path = tactic_asset_path(&root, &request.asset_id)?;
    let current = read_blueprint(&path)?;
    require_revision(&current, &request.revision)?;
    move_to_trash(&root, &path)?;
    Ok(TacticMutationResponse {
        schema: TACTIC_CATALOG_SCHEMA,
        asset_id: request.asset_id.clone(),
        revision: None,
        status: "deleted",
    })
}

#[derive(Debug)]
struct AuthoredRecord {
    projection: GraphTacticAsset,
    blueprint: Option<TacticBlueprint>,
}

fn load_authored_records(
    repository_root: &Path,
    timeline_path: &Path,
) -> Result<Vec<AuthoredRecord>, WorkbenchError> {
    let root = tactic_store_root(repository_root, timeline_path)?;
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let catalog = default_route_tactic_catalog().map_err(tactic_store_error)?;
    let mut paths = fs::read_dir(&root)
        .map_err(tactic_store_error)?
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_type()
                .is_ok_and(|kind| kind.is_file() && !kind.is_symlink())
                && entry
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".tactic.json")
        })
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    paths.sort();
    if paths.len() > MAX_AUTHORED_TACTICS {
        return Err(WorkbenchError::new("authored tactic capacity is exhausted"));
    }
    Ok(paths
        .into_iter()
        .map(|path| {
            match read_blueprint(&path).and_then(|blueprint| {
                blueprint
                    .validate_against_catalog(&catalog)
                    .map_err(tactic_store_error)?;
                Ok(blueprint)
            }) {
                Ok(blueprint) => {
                    let revision = blueprint
                        .content_sha256()
                        .map(|digest| digest.to_string())
                        .unwrap_or_default();
                    let (kind, steps, minimum_ticks, maximum_ticks) =
                        blueprint_projection_shape(&blueprint, &catalog);
                    AuthoredRecord {
                        projection: GraphTacticAsset {
                            asset_id: blueprint.asset_id.clone(),
                            source: "workspace".into(),
                            read_only: false,
                            revision,
                            kind,
                            minimum_ticks,
                            maximum_ticks,
                            steps,
                            error: None,
                        },
                        blueprint: Some(blueprint),
                    }
                }
                Err(error) => AuthoredRecord {
                    projection: GraphTacticAsset {
                        asset_id: path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("invalid")
                            .trim_end_matches(".tactic.json")
                            .into(),
                        source: "workspace".into(),
                        read_only: false,
                        revision: String::new(),
                        kind: "invalid".into(),
                        minimum_ticks: None,
                        maximum_ticks: None,
                        steps: Vec::new(),
                        error: Some(error.to_string()),
                    },
                    blueprint: None,
                },
            }
        })
        .collect())
}

fn blueprint_projection_shape(
    blueprint: &TacticBlueprint,
    catalog: &dusklight_learning::tactic_asset::TacticAssetCatalog,
) -> (String, Vec<String>, Option<u32>, Option<u32>) {
    let TacticBlueprintNode::Sequence { steps } = &blueprint.root else {
        return ("advanced".into(), Vec::new(), None, None);
    };
    let Some(option_ids) = steps
        .iter()
        .map(|step| match step {
            TacticBlueprintNode::Invoke { option_id } => Some(option_id.clone()),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()
    else {
        return ("advanced".into(), Vec::new(), None, None);
    };
    let bounds = option_ids.iter().try_fold((0_u32, 0_u32), |bounds, id| {
        let duration = catalog.entry(id)?.description().duration;
        Some((
            bounds.0.checked_add(duration.minimum_ticks)?,
            bounds.1.checked_add(duration.maximum_ticks)?,
        ))
    });
    let (minimum, maximum) = bounds.unwrap_or((0, 0));
    (
        "sequence".into(),
        option_ids,
        (minimum > 0).then_some(minimum),
        (maximum > 0).then_some(maximum),
    )
}

fn sequence_blueprint(asset_id: &str, steps: &[String]) -> Result<TacticBlueprint, WorkbenchError> {
    if steps.is_empty() || steps.len() > 64 {
        return Err(WorkbenchError::new(
            "authored tactic sequence must contain 1 to 64 steps",
        ));
    }
    let blueprint = TacticBlueprint::new(
        asset_id,
        TacticBlueprintNode::Sequence {
            steps: steps
                .iter()
                .map(|option_id| TacticBlueprintNode::Invoke {
                    option_id: option_id.clone(),
                })
                .collect(),
        },
    )
    .map_err(tactic_store_error)?;
    blueprint
        .validate_against_catalog(&default_route_tactic_catalog().map_err(tactic_store_error)?)
        .map_err(tactic_store_error)?;
    Ok(blueprint)
}

fn tactic_store_root(
    repository_root: &Path,
    timeline_path: &Path,
) -> Result<PathBuf, WorkbenchError> {
    let repository = repository_root.canonicalize().map_err(tactic_store_error)?;
    let timeline = timeline_path.canonicalize().map_err(tactic_store_error)?;
    let workspace = repository.join(PROJECT_WORKSPACE_PATH);
    if !timeline.starts_with(&workspace)
        || timeline.extension().and_then(|value| value.to_str()) != Some("timeline")
    {
        return Err(WorkbenchError::new(
            "active timeline is outside the route workspace",
        ));
    }
    Ok(timeline.with_extension("").join("tactics"))
}

fn tactic_asset_path(root: &Path, asset_id: &str) -> Result<PathBuf, WorkbenchError> {
    if asset_id.is_empty()
        || asset_id.len() > 96
        || !asset_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err(WorkbenchError::new(
            "tactic asset ID must use 1 to 96 letters, digits, dots, dashes, or underscores",
        ));
    }
    Ok(root.join(format!("{asset_id}.tactic.json")))
}

fn read_blueprint(path: &Path) -> Result<TacticBlueprint, WorkbenchError> {
    let metadata = fs::symlink_metadata(path).map_err(tactic_store_error)?;
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > MAX_TACTIC_FILE_BYTES
    {
        return Err(WorkbenchError::new(
            "authored tactic file is absent, linked, or oversized",
        ));
    }
    let blueprint: TacticBlueprint =
        serde_json::from_slice(&fs::read(path).map_err(tactic_store_error)?)
            .map_err(tactic_store_error)?;
    blueprint.validate().map_err(tactic_store_error)?;
    let expected = path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_suffix(".tactic.json"));
    if expected != Some(blueprint.asset_id.as_str()) {
        return Err(WorkbenchError::new(
            "authored tactic file name and asset ID disagree",
        ));
    }
    Ok(blueprint)
}

fn require_revision(blueprint: &TacticBlueprint, revision: &str) -> Result<(), WorkbenchError> {
    if blueprint
        .content_sha256()
        .map_err(tactic_store_error)?
        .to_string()
        != revision
    {
        return Err(WorkbenchError::new(
            "authored tactic changed; refresh before editing it",
        ));
    }
    Ok(())
}

fn write_new_blueprint(path: &Path, blueprint: &TacticBlueprint) -> Result<(), WorkbenchError> {
    if path.exists() {
        return Err(WorkbenchError::new("authored tactic asset already exists"));
    }
    let bytes = serde_json::to_vec_pretty(blueprint).map_err(tactic_store_error)?;
    let parent = path
        .parent()
        .ok_or_else(|| WorkbenchError::new("tactic asset path has no parent"))?;
    fs::create_dir_all(parent).map_err(tactic_store_error)?;
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(tactic_store_error)?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(tactic_store_error)
}

fn replace_blueprint(path: &Path, blueprint: &TacticBlueprint) -> Result<(), WorkbenchError> {
    let parent = path
        .parent()
        .ok_or_else(|| WorkbenchError::new("tactic asset path has no parent"))?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(tactic_store_error)?
        .as_nanos();
    let temporary = parent.join(format!(".tactic-edit.{nonce}.partial"));
    let backup = parent.join(format!(".tactic-edit.{nonce}.backup"));
    write_new_blueprint(&temporary, blueprint)?;
    fs::rename(path, &backup).map_err(tactic_store_error)?;
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::rename(&backup, path);
        return Err(tactic_store_error(error));
    }
    fs::remove_file(backup).map_err(tactic_store_error)
}

fn move_to_trash(root: &Path, path: &Path) -> Result<(), WorkbenchError> {
    let trash = root.join(".trash");
    fs::create_dir_all(&trash).map_err(tactic_store_error)?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(tactic_store_error)?
        .as_nanos();
    let name = path
        .file_name()
        .ok_or_else(|| WorkbenchError::new("tactic asset has no file name"))?;
    fs::rename(
        path,
        trash.join(format!("{nonce}-{}", name.to_string_lossy())),
    )
    .map_err(tactic_store_error)
}

fn ensure_store_capacity(root: &Path) -> Result<(), WorkbenchError> {
    let count = fs::read_dir(root)
        .map_err(tactic_store_error)?
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_type()
                .is_ok_and(|kind| kind.is_file() && !kind.is_symlink())
                && entry
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".tactic.json")
        })
        .count();
    if count >= MAX_AUTHORED_TACTICS {
        return Err(WorkbenchError::new("authored tactic capacity is exhausted"));
    }
    Ok(())
}

fn mutation_response(
    blueprint: TacticBlueprint,
    status: &'static str,
) -> Result<TacticMutationResponse, WorkbenchError> {
    Ok(TacticMutationResponse {
        schema: TACTIC_CATALOG_SCHEMA,
        asset_id: blueprint.asset_id.clone(),
        revision: Some(
            blueprint
                .content_sha256()
                .map_err(tactic_store_error)?
                .to_string(),
        ),
        status,
    })
}

fn tactic_store_edits() -> &'static Mutex<()> {
    static EDITS: OnceLock<Mutex<()>> = OnceLock::new();
    EDITS.get_or_init(|| Mutex::new(()))
}

fn tactic_store_error(error: impl fmt::Display) -> WorkbenchError {
    WorkbenchError::new(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authored_tactics_are_independent_typed_files_with_stale_safe_crud() {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../..")
            .canonicalize()
            .unwrap();
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let workspace = repository
            .join("routes")
            .join(format!(".tactic-store-test-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&workspace).unwrap();
        let timeline = workspace.join("test.timeline");
        fs::write(&timeline, b"test").unwrap();
        let config = WorkbenchConfig {
            timeline_path: timeline.clone(),
            repository_root: repository.clone(),
            working_directory: repository.clone(),
            game: repository.join("build/unused-game"),
            dvd: repository.join("build/unused-dvd"),
            world_context: None,
            state_root: repository.join("build/unused-state"),
        };

        let created = create_authored_tactic(
            &config,
            &BrowserTacticCreateRequest {
                asset_id: "safe_wait".into(),
                steps: vec!["wait.neutral.04".into()],
            },
        )
        .unwrap();
        let revision = created.revision.unwrap();
        let path = timeline
            .with_extension("")
            .join("tactics/safe_wait.tactic.json");
        assert!(path.is_file());
        assert!(
            update_authored_tactic(
                &config,
                &BrowserTacticUpdateRequest {
                    asset_id: "safe_wait".into(),
                    revision: "0".repeat(64),
                    steps: vec!["wait.neutral.04".into()],
                },
            )
            .is_err()
        );

        let updated = update_authored_tactic(
            &config,
            &BrowserTacticUpdateRequest {
                asset_id: "safe_wait".into(),
                revision,
                steps: vec!["wait.neutral.04".into(), "wait.neutral.04".into()],
            },
        )
        .unwrap();
        let renamed = rename_authored_tactic(
            &config,
            &BrowserTacticRenameRequest {
                asset_id: "safe_wait".into(),
                revision: updated.revision.unwrap(),
                new_asset_id: "double_wait".into(),
            },
        )
        .unwrap();
        assert!(!path.exists());
        let duplicated = duplicate_authored_tactic(
            &config,
            &BrowserTacticDuplicateRequest {
                asset_id: "double_wait".into(),
                revision: renamed.revision.unwrap(),
                new_asset_id: "double_wait_copy".into(),
            },
        )
        .unwrap();
        let projection = tactic_catalog_projection(&repository, &timeline).unwrap();
        assert_eq!(projection.built_ins.len(), 136);
        assert_eq!(projection.authored.len(), 2);
        assert_eq!(
            load_authored_tactic_blueprints(&repository, &timeline)
                .unwrap()
                .len(),
            2
        );

        delete_authored_tactic(
            &config,
            &BrowserTacticDeleteRequest {
                asset_id: "double_wait_copy".into(),
                revision: duplicated.revision.unwrap(),
            },
        )
        .unwrap();
        assert_eq!(
            load_authored_tactic_blueprints(&repository, &timeline)
                .unwrap()
                .len(),
            1
        );
        assert!(
            timeline
                .with_extension("")
                .join("tactics/.trash")
                .read_dir()
                .unwrap()
                .next()
                .is_some()
        );

        fs::remove_dir_all(workspace).unwrap();
    }
}
