//! Filesystem-backed workbench workspace.
//!
//! The directory tree is authoritative. Timeline files are route graphs and
//! standalone TAS/tape files are boot-rooted one-node graphs. A directory with
//! the same stem as a timeline is private route storage and is not projected as
//! a second set of standalone tapes.

use super::*;
use crate::scenario_fixture::ScenarioFixture;

const BOOT_OVERRIDE_SCHEMA: &str = "dusklight.route-workbench.boot-override.v1";
const NEW_TAPE_SOURCE: &str =
    "dusktape 1\nrate 30/1\nports 0x0f\n\nstate neutral {}\nframe neutral\n";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WorkspaceBootOverride {
    schema: String,
    enabled: bool,
    boot: TapeBoot,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ProjectKind {
    Timeline,
    Tas,
    Tape,
}

#[derive(Clone, Debug)]
pub(super) struct ProjectDefinition {
    pub id: String,
    pub label: String,
    pub group: String,
    pub artifact: PathBuf,
    pub kind: ProjectKind,
    pub fixture: Option<PathBuf>,
    pub launch: Option<PathBuf>,
}

#[derive(Clone, Debug, Default)]
pub(super) struct ProjectCatalog {
    pub groups: BTreeMap<String, String>,
    pub entries: BTreeMap<String, ProjectDefinition>,
}

pub(super) fn load_project_catalog(
    repository_root: &Path,
) -> Result<ProjectCatalog, WorkbenchError> {
    let root = repository_root.join(PROJECT_WORKSPACE_PATH);
    if !root.is_dir() {
        return Ok(ProjectCatalog::default());
    }
    let canonical_repository = fs::canonicalize(repository_root).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot resolve repository root {}: {error}",
            repository_root.display()
        ))
    })?;
    let canonical_root = fs::canonicalize(&root).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot resolve workspace {}: {error}",
            root.display()
        ))
    })?;
    if canonical_root.parent() != Some(canonical_repository.as_path()) {
        return Err(WorkbenchError::new("workspace root escapes the repository"));
    }

    let (files, directories) = scan_workspace(&canonical_root)?;
    let private_roots = files
        .iter()
        .filter(|path| extension(path) == Some("timeline"))
        .map(|path| path.with_extension(""))
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();

    let mut catalog = ProjectCatalog::default();
    catalog
        .groups
        .insert(PROJECT_WORKSPACE_PATH.into(), "Routes".into());
    for directory in directories {
        if private_roots
            .iter()
            .any(|private| directory == *private || directory.starts_with(private))
        {
            continue;
        }
        let relative = directory.strip_prefix(&canonical_repository).map_err(|_| {
            WorkbenchError::new(format!(
                "workspace directory {} escapes the repository",
                directory.display()
            ))
        })?;
        add_group_ancestry(&mut catalog.groups, relative)?;
    }
    for path in files {
        let kind = match extension(&path) {
            Some("timeline") => ProjectKind::Timeline,
            Some("tas") => ProjectKind::Tas,
            Some("tape") => ProjectKind::Tape,
            _ => continue,
        };
        if kind != ProjectKind::Timeline
            && private_roots
                .iter()
                .any(|private| path.starts_with(private))
        {
            continue;
        }
        let relative = path.strip_prefix(&canonical_repository).map_err(|_| {
            WorkbenchError::new(format!(
                "workspace artifact {} escapes the repository",
                path.display()
            ))
        })?;
        let without_extension = relative.with_extension("");
        let id = slash_path(&without_extension)?;
        let parent = relative
            .parent()
            .unwrap_or_else(|| Path::new(PROJECT_WORKSPACE_PATH));
        let group = slash_path(parent)?;
        add_group_ancestry(&mut catalog.groups, parent)?;
        let fixture = (kind != ProjectKind::Timeline)
            .then(|| path.with_extension("fixture.json"))
            .filter(|candidate| candidate.is_file())
            .map(|candidate| {
                candidate
                    .strip_prefix(&canonical_repository)
                    .expect("checked workspace fixture")
                    .to_path_buf()
            });
        let launch = (kind != ProjectKind::Timeline)
            .then(|| path.with_extension("launch"))
            .filter(|candidate| candidate.is_file())
            .map(|candidate| {
                candidate
                    .strip_prefix(&canonical_repository)
                    .expect("checked workspace launch profile")
                    .to_path_buf()
            });
        let label = if kind == ProjectKind::Timeline {
            load_authoritative_timeline(&path)
                .map(|timeline| human_label(&timeline.name))
                .unwrap_or_else(|_| {
                    human_label(path.file_stem().and_then(|v| v.to_str()).unwrap_or("route"))
                })
        } else {
            human_label(
                path.file_stem()
                    .and_then(|value| value.to_str())
                    .unwrap_or("tape"),
            )
        };
        let definition = ProjectDefinition {
            id: id.clone(),
            label,
            group,
            artifact: relative.to_path_buf(),
            kind,
            fixture,
            launch,
        };
        if catalog.entries.insert(id.clone(), definition).is_some() {
            return Err(WorkbenchError::new(format!(
                "workspace contains duplicate project id {id:?}"
            )));
        }
    }
    Ok(catalog)
}

pub(super) fn project_catalog_projection(
    repository_root: &Path,
    active_timeline: &Path,
) -> Result<GraphProjectCatalog, WorkbenchError> {
    let catalog = load_project_catalog(repository_root)?;
    let inventory = crate::inventory_catalog::load(repository_root)?;
    let active_timeline = fs::canonicalize(active_timeline).ok();
    let groups = catalog
        .groups
        .iter()
        .map(|(id, label)| GraphProjectGroup {
            id: id.clone(),
            label: label.clone(),
            parent: id.rsplit_once('/').map(|(parent, _)| parent.to_owned()),
        })
        .collect();
    let entries = catalog
        .entries
        .values()
        .map(|entry| {
            let artifact_path = repository_root.join(&entry.artifact);
            let (boot_override, override_error) = match load_boot_override(repository_root, entry) {
                Ok(configuration) => (configuration, None),
                Err(error) => (None, Some(error.to_string())),
            };
            if entry.kind == ProjectKind::Timeline {
                let error = load_authoritative_timeline(&artifact_path)
                    .and_then(|timeline| {
                        timeline
                            .inspect()
                            .map_err(|error| WorkbenchError::new(error.to_string()))
                    })
                    .err()
                    .map(|error| error.to_string())
                    .or(override_error);
                return GraphProject {
                    id: entry.id.clone(),
                    label: entry.label.clone(),
                    group: entry.group.clone(),
                    kind: "timeline".into(),
                    active: active_timeline.as_ref().is_some_and(|active| {
                        *active == artifact_path.canonicalize().unwrap_or_default()
                    }),
                    boot_override,
                    artifact: GraphArtifact {
                        kind: "timeline".into(),
                        value: entry.artifact.display().to_string(),
                    },
                    boot: None,
                    frame_count: None,
                    materialization_sha256: None,
                    thumbnail: None,
                    playable: error.is_none(),
                    fixture_source: None,
                    error,
                };
            }

            let loaded = load_project_tape(repository_root, entry).and_then(|tape| {
                load_project_native_oracle(repository_root, entry).map(|oracle| (tape, oracle))
            });
            let (boot, frame_count, materialization_sha256, error) = match loaded {
                Ok((mut tape, _profile)) => {
                    let authored_boot = tape.boot.clone();
                    if let Some(configuration) =
                        boot_override.as_ref().filter(|value| value.enabled)
                    {
                        tape.boot = configuration.boot.clone();
                    }
                    match tape.encode() {
                        Ok(encoded) => (
                            Some(authored_boot),
                            Some(tape.frames.len() as u64),
                            Some(format!("{:x}", Sha256::digest(encoded))),
                            override_error,
                        ),
                        Err(error) => (None, None, None, Some(error.to_string())),
                    }
                }
                Err(error) => (None, None, None, Some(error.to_string())),
            };
            GraphProject {
                id: entry.id.clone(),
                label: entry.label.clone(),
                group: entry.group.clone(),
                kind: match entry.kind {
                    ProjectKind::Tas => "tas",
                    ProjectKind::Tape => "tape",
                    ProjectKind::Timeline => unreachable!(),
                }
                .into(),
                active: false,
                boot_override,
                artifact: GraphArtifact {
                    kind: match entry.kind {
                        ProjectKind::Tas => "tas",
                        ProjectKind::Tape => "tape",
                        ProjectKind::Timeline => unreachable!(),
                    }
                    .into(),
                    value: entry.artifact.display().to_string(),
                },
                boot,
                frame_count,
                materialization_sha256,
                thumbnail: None,
                playable: error.is_none(),
                fixture_source: entry
                    .fixture
                    .as_ref()
                    .map(|path| path.display().to_string()),
                error,
            }
        })
        .collect();
    Ok(GraphProjectCatalog {
        schema: PROJECT_CATALOG_SCHEMA.into(),
        groups,
        entries,
        stages: crate::stage_catalog::stage_summaries(repository_root),
        inventory_items: inventory.items,
        inventory_slots: inventory.slots,
    })
}

pub(super) fn project_materialized_playback(
    repository_root: &Path,
    project_id: &str,
) -> Result<MaterializedPlayback, WorkbenchError> {
    let catalog = load_project_catalog(repository_root)?;
    let project = catalog
        .entries
        .get(project_id)
        .ok_or_else(|| WorkbenchError::new(format!("unknown workspace tape {project_id:?}")))?;
    if project.kind == ProjectKind::Timeline {
        return Err(WorkbenchError::new(
            "a timeline must be opened as a route graph",
        ));
    }
    let mut tape = load_project_tape(repository_root, project)?;
    apply_boot_override(repository_root, project, &mut tape)?;
    let native_oracle = load_project_native_oracle(repository_root, project)?;
    Ok(MaterializedPlayback {
        lineage: None,
        segment: Some(format!("project:{project_id}")),
        tape,
        seed_stage: None,
        native_oracle,
    })
}

fn load_project_native_oracle(
    repository_root: &Path,
    project: &ProjectDefinition,
) -> Result<NativePlaybackOracle, WorkbenchError> {
    let Some(relative) = &project.launch else {
        return Ok(NativePlaybackOracle::None);
    };
    let path = checked_artifact_path(repository_root, relative)?;
    let source = fs::read_to_string(&path)
        .map_err(|error| WorkbenchError::new(format!("cannot read {}: {error}", path.display())))?;
    let lines = source
        .lines()
        .map(|line| line.split_once('#').map_or(line, |(body, _)| body).trim())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.first().copied() != Some("dusklaunch 1") {
        return Err(WorkbenchError::new(format!(
            "{} must begin with `dusklaunch 1`",
            path.display()
        )));
    }
    match lines.as_slice() {
        [_, "oracle eye_shredder"] => Ok(NativePlaybackOracle::EyeShredder),
        [_, profile] => Err(WorkbenchError::new(format!(
            "{} has unsupported launch directive {profile:?}",
            path.display()
        ))),
        _ => Err(WorkbenchError::new(format!(
            "{} must contain exactly one `oracle NAME` directive",
            path.display()
        ))),
    }
}

pub(super) fn update_boot_override(
    repository_root: &Path,
    request: &BrowserBootOverrideUpdateRequest,
) -> Result<GraphBootOverride, WorkbenchError> {
    let _guard = workspace_edits()
        .lock()
        .map_err(|_| WorkbenchError::new("workspace edit lock is poisoned"))?;
    let catalog = load_project_catalog(repository_root)?;
    let project = catalog.entries.get(&request.project).ok_or_else(|| {
        WorkbenchError::new(format!("unknown workspace entry {:?}", request.project))
    })?;
    InputTape {
        boot: request.boot.clone(),
        ..InputTape::default()
    }
    .validate()
    .map_err(|error| WorkbenchError::new(format!("invalid boot configuration: {error}")))?;
    validate_native_inventory_limits(&request.boot)?;
    let path = boot_override_path(repository_root, project)?;
    if path.exists() {
        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            WorkbenchError::new(format!("cannot inspect {}: {error}", path.display()))
        })?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(WorkbenchError::new("boot override is not a regular file"));
        }
    }
    let document = WorkspaceBootOverride {
        schema: BOOT_OVERRIDE_SCHEMA.into(),
        enabled: request.enabled,
        boot: request.boot.clone(),
    };
    let mut bytes = serde_json::to_vec_pretty(&document)
        .map_err(|error| WorkbenchError::new(error.to_string()))?;
    bytes.push(b'\n');
    replace_workspace_file(&path, &bytes)?;
    Ok(GraphBootOverride {
        enabled: request.enabled,
        boot: request.boot.clone(),
    })
}

fn validate_native_inventory_limits(boot: &TapeBoot) -> Result<(), WorkbenchError> {
    let TapeBoot::Stage {
        fixture: Some(fixture),
        ..
    } = boot
    else {
        return Ok(());
    };
    for entry in &fixture.inventory {
        if entry.slot >= 24 || entry.item > u8::MAX.into() || entry.quantity > u8::MAX.into() {
            return Err(WorkbenchError::new(
                "inventory slot, item, or quantity exceeds native save limits",
            ));
        }
        let quantity_slot = matches!(entry.slot, 4 | 11..=17 | 23);
        if entry.quantity != 1 && !quantity_slot {
            return Err(WorkbenchError::new(
                "inventory quantity is only defined for bow, bottle, bomb-bag, and slingshot slots",
            ));
        }
    }
    Ok(())
}

pub(super) fn active_timeline_boot_override(
    repository_root: &Path,
    timeline_path: &Path,
) -> Result<Option<GraphBootOverride>, WorkbenchError> {
    let catalog = load_project_catalog(repository_root)?;
    let canonical_timeline = fs::canonicalize(timeline_path).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot resolve {}: {error}",
            timeline_path.display()
        ))
    })?;
    let project = catalog
        .entries
        .values()
        .find(|entry| {
            entry.kind == ProjectKind::Timeline
                && fs::canonicalize(repository_root.join(&entry.artifact))
                    .ok()
                    .as_ref()
                    == Some(&canonical_timeline)
        })
        .ok_or_else(|| WorkbenchError::new("active timeline is not in the workspace"))?;
    load_boot_override(repository_root, project)
}

fn apply_boot_override(
    repository_root: &Path,
    project: &ProjectDefinition,
    tape: &mut InputTape,
) -> Result<(), WorkbenchError> {
    if let Some(configuration) = load_boot_override(repository_root, project)?
        && configuration.enabled
    {
        tape.boot = configuration.boot;
    }
    Ok(())
}

fn load_boot_override(
    repository_root: &Path,
    project: &ProjectDefinition,
) -> Result<Option<GraphBootOverride>, WorkbenchError> {
    let path = boot_override_path(repository_root, project)?;
    if !path.exists() {
        return Ok(None);
    }
    let metadata = fs::symlink_metadata(&path).map_err(|error| {
        WorkbenchError::new(format!("cannot inspect {}: {error}", path.display()))
    })?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > MAX_HTTP_BODY as u64
    {
        return Err(WorkbenchError::new(
            "boot override is not a bounded regular file",
        ));
    }
    let document: WorkspaceBootOverride =
        serde_json::from_slice(&fs::read(&path).map_err(|error| {
            WorkbenchError::new(format!("cannot read {}: {error}", path.display()))
        })?)
        .map_err(|error| {
            WorkbenchError::new(format!("cannot decode {}: {error}", path.display()))
        })?;
    if document.schema != BOOT_OVERRIDE_SCHEMA {
        return Err(WorkbenchError::new(format!(
            "unsupported boot override schema in {}",
            path.display()
        )));
    }
    InputTape {
        boot: document.boot.clone(),
        ..InputTape::default()
    }
    .validate()
    .map_err(|error| {
        WorkbenchError::new(format!("invalid boot override {}: {error}", path.display()))
    })?;
    Ok(Some(GraphBootOverride {
        enabled: document.enabled,
        boot: document.boot,
    }))
}

fn boot_override_path(
    repository_root: &Path,
    project: &ProjectDefinition,
) -> Result<PathBuf, WorkbenchError> {
    Ok(checked_artifact_path(repository_root, &project.artifact)?.with_extension("boot.json"))
}

fn replace_workspace_file(path: &Path, bytes: &[u8]) -> Result<(), WorkbenchError> {
    let temporary = path.with_extension(format!("json.{}.tmp", std::process::id()));
    fs::write(&temporary, bytes).map_err(|error| {
        WorkbenchError::new(format!("cannot write {}: {error}", temporary.display()))
    })?;
    if !path.exists() {
        return fs::rename(&temporary, path).map_err(|error| {
            let _ = fs::remove_file(&temporary);
            WorkbenchError::new(format!("cannot install {}: {error}", path.display()))
        });
    }
    let backup = path.with_extension(format!("json.{}.bak", std::process::id()));
    fs::rename(path, &backup).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        WorkbenchError::new(format!("cannot stage {}: {error}", path.display()))
    })?;
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::rename(&backup, path);
        let _ = fs::remove_file(&temporary);
        return Err(WorkbenchError::new(format!(
            "cannot replace {}: {error}",
            path.display()
        )));
    }
    let _ = fs::remove_file(backup);
    Ok(())
}

pub(super) fn create_workspace_folder(
    repository_root: &Path,
    request: &BrowserWorkspaceFolderCreateRequest,
) -> Result<WorkspaceMutationResult, WorkbenchError> {
    let _guard = workspace_edits()
        .lock()
        .map_err(|_| WorkbenchError::new("workspace edit lock is poisoned"))?;
    validate_workspace_name(&request.name)?;
    let parent = public_workspace_group_path(repository_root, &request.parent)?;
    let destination = parent.join(&request.name);
    if destination.exists() {
        return Err(WorkbenchError::new("workspace destination already exists"));
    }
    fs::create_dir(&destination).map_err(|error| {
        WorkbenchError::new(format!("cannot create {}: {error}", destination.display()))
    })?;
    let id = format!("{}/{}", request.parent, request.name);
    Ok(WorkspaceMutationResult {
        operation: "create_folder".into(),
        id: id.clone(),
        destination: Some(id),
        trash: None,
    })
}

pub(super) fn create_workspace_tape(
    repository_root: &Path,
    request: &BrowserWorkspaceTapeCreateRequest,
) -> Result<WorkspaceMutationResult, WorkbenchError> {
    let _guard = workspace_edits()
        .lock()
        .map_err(|_| WorkbenchError::new("workspace edit lock is poisoned"))?;
    validate_workspace_name(&request.name)?;
    let parent = public_workspace_group_path(repository_root, &request.parent)?;
    let destination = parent.join(format!("{}.tas", request.name));
    if destination.exists() {
        return Err(WorkbenchError::new("workspace destination already exists"));
    }
    crate::tape_dsl::parse(NEW_TAPE_SOURCE)
        .map_err(|error| WorkbenchError::new(format!("invalid built-in tape template: {error}")))?
        .compile()
        .map_err(|error| WorkbenchError::new(format!("invalid built-in tape template: {error}")))?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&destination)
        .map_err(|error| {
            WorkbenchError::new(format!("cannot create {}: {error}", destination.display()))
        })?;
    file.write_all(NEW_TAPE_SOURCE.as_bytes())
        .map_err(|error| {
            let _ = fs::remove_file(&destination);
            WorkbenchError::new(format!("cannot write {}: {error}", destination.display()))
        })?;
    let id = format!("{}/{}", request.parent, request.name);
    Ok(WorkspaceMutationResult {
        operation: "create_tape".into(),
        id: id.clone(),
        destination: Some(id),
        trash: None,
    })
}

pub(super) fn clone_workspace_tape(
    repository_root: &Path,
    request: &BrowserWorkspaceTapeCloneRequest,
) -> Result<WorkspaceMutationResult, WorkbenchError> {
    let _guard = workspace_edits()
        .lock()
        .map_err(|_| WorkbenchError::new("workspace edit lock is poisoned"))?;
    validate_workspace_name(&request.name)?;
    let catalog = load_project_catalog(repository_root)?;
    let project = catalog.entries.get(&request.source).ok_or_else(|| {
        WorkbenchError::new(format!("unknown workspace tape {:?}", request.source))
    })?;
    if project.kind == ProjectKind::Timeline {
        return Err(WorkbenchError::new(
            "route timelines cannot be cloned as standalone tapes",
        ));
    }
    let destination = public_workspace_group_path(repository_root, &request.destination)?;
    let artifact = checked_artifact_path(repository_root, &project.artifact)?;
    let stem = artifact
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| WorkbenchError::new("workspace filename is not UTF-8"))?;
    let sources =
        workspace_node_sources(repository_root, &request.source, WorkspaceNodeKind::Project)?;
    let copies = sources
        .iter()
        .map(|source| {
            let metadata = fs::symlink_metadata(source)
                .map_err(|error| WorkbenchError::new(error.to_string()))?;
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err(WorkbenchError::new(
                    "standalone tape cloning only accepts regular artifact files",
                ));
            }
            let filename = source
                .file_name()
                .and_then(|value| value.to_str())
                .ok_or_else(|| WorkbenchError::new("workspace filename is not UTF-8"))?;
            let suffix = filename
                .strip_prefix(stem)
                .ok_or_else(|| WorkbenchError::new("workspace sidecar stem changed"))?;
            Ok((
                source.clone(),
                destination.join(format!("{}{suffix}", request.name)),
            ))
        })
        .collect::<Result<Vec<_>, WorkbenchError>>()?;
    if copies.iter().any(|(_, target)| target.exists()) {
        return Err(WorkbenchError::new(
            "workspace clone destination already exists",
        ));
    }
    let mut completed: Vec<PathBuf> = Vec::new();
    for (source, target) in &copies {
        if let Err(error) = fs::copy(source, target) {
            for copied in completed.iter().rev() {
                let _ = fs::remove_file(copied);
            }
            return Err(WorkbenchError::new(format!(
                "cannot clone {} to {}: {error}",
                source.display(),
                target.display()
            )));
        }
        completed.push(target.clone());
    }
    let id = format!("{}/{}", request.destination, request.name);
    Ok(WorkspaceMutationResult {
        operation: "clone_tape".into(),
        id: id.clone(),
        destination: Some(id),
        trash: None,
    })
}

pub(super) fn move_workspace_node(
    repository_root: &Path,
    active_timeline: &Path,
    request: &BrowserWorkspaceMoveRequest,
) -> Result<WorkspaceMutationResult, WorkbenchError> {
    let _guard = workspace_edits()
        .lock()
        .map_err(|_| WorkbenchError::new("workspace edit lock is poisoned"))?;
    let destination = public_workspace_group_path(repository_root, &request.destination)?;
    let sources = workspace_node_sources(repository_root, &request.id, request.kind)?;
    reject_active_timeline_move(active_timeline, &sources)?;
    if sources
        .iter()
        .filter(|source| source.is_dir())
        .any(|source| destination == *source || destination.starts_with(source))
    {
        return Err(WorkbenchError::new(
            "a workspace folder cannot be moved into itself",
        ));
    }
    let moves = sources
        .iter()
        .map(|source| {
            let name = source
                .file_name()
                .ok_or_else(|| WorkbenchError::new("workspace source has no filename"))?;
            Ok((source.clone(), destination.join(name)))
        })
        .collect::<Result<Vec<_>, WorkbenchError>>()?;
    if moves.iter().any(|(_, target)| target.exists()) {
        return Err(WorkbenchError::new(
            "workspace move destination already exists",
        ));
    }
    let mut completed = Vec::new();
    for (source, target) in &moves {
        if let Err(error) = fs::rename(source, target) {
            for (original, moved) in completed.iter().rev() {
                let _ = fs::rename(moved, original);
            }
            return Err(WorkbenchError::new(format!(
                "cannot move {} to {}: {error}",
                source.display(),
                target.display()
            )));
        }
        completed.push((source.clone(), target.clone()));
    }
    Ok(WorkspaceMutationResult {
        operation: "move".into(),
        id: request.id.clone(),
        destination: Some(request.destination.clone()),
        trash: None,
    })
}

pub(super) fn delete_workspace_node(
    repository_root: &Path,
    active_timeline: &Path,
    state_root: &Path,
    request: &BrowserWorkspaceDeleteRequest,
) -> Result<WorkspaceMutationResult, WorkbenchError> {
    let _guard = workspace_edits()
        .lock()
        .map_err(|_| WorkbenchError::new("workspace edit lock is poisoned"))?;
    let sources = workspace_node_sources(repository_root, &request.id, request.kind)?;
    reject_active_timeline_move(active_timeline, &sources)?;
    fs::create_dir_all(state_root).map_err(|error| {
        WorkbenchError::new(format!("cannot create {}: {error}", state_root.display()))
    })?;
    let trash_root = fs::canonicalize(state_root)
        .map_err(|error| WorkbenchError::new(error.to_string()))?
        .join("workspace-trash");
    fs::create_dir_all(&trash_root).map_err(|error| {
        WorkbenchError::new(format!("cannot create {}: {error}", trash_root.display()))
    })?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let transaction = trash_root.join(format!("{}-{nonce}", std::process::id()));
    fs::create_dir(&transaction).map_err(|error| {
        WorkbenchError::new(format!("cannot create {}: {error}", transaction.display()))
    })?;
    let mut completed = Vec::new();
    for source in &sources {
        let target = transaction.join(
            source
                .file_name()
                .ok_or_else(|| WorkbenchError::new("workspace source has no filename"))?,
        );
        if let Err(error) = fs::rename(source, &target) {
            for (original, moved) in completed.iter().rev() {
                let _ = fs::rename(moved, original);
            }
            let _ = fs::remove_dir(&transaction);
            return Err(WorkbenchError::new(format!(
                "cannot move {} to workspace trash: {error}",
                source.display()
            )));
        }
        completed.push((source.clone(), target));
    }
    Ok(WorkspaceMutationResult {
        operation: "delete".into(),
        id: request.id.clone(),
        destination: None,
        trash: Some(transaction),
    })
}

fn workspace_node_sources(
    repository_root: &Path,
    id: &str,
    kind: WorkspaceNodeKind,
) -> Result<Vec<PathBuf>, WorkbenchError> {
    match kind {
        WorkspaceNodeKind::Folder => {
            let source = public_workspace_group_path(repository_root, id)?;
            if id == PROJECT_WORKSPACE_PATH {
                return Err(WorkbenchError::new(
                    "workspace root cannot be moved or deleted",
                ));
            }
            Ok(vec![source])
        }
        WorkspaceNodeKind::Project => {
            let catalog = load_project_catalog(repository_root)?;
            let project = catalog
                .entries
                .get(id)
                .ok_or_else(|| WorkbenchError::new(format!("unknown workspace entry {id:?}")))?;
            let artifact = checked_artifact_path(repository_root, &project.artifact)?;
            let stem = artifact
                .file_stem()
                .and_then(|value| value.to_str())
                .ok_or_else(|| WorkbenchError::new("workspace filename is not UTF-8"))?;
            let mut sources = fs::read_dir(
                artifact
                    .parent()
                    .ok_or_else(|| WorkbenchError::new("workspace artifact has no parent"))?,
            )
            .map_err(|error| WorkbenchError::new(error.to_string()))?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|value| value.to_str())
                    .is_some_and(|name| {
                        name == artifact
                            .file_name()
                            .and_then(|value| value.to_str())
                            .unwrap_or_default()
                            || name == stem
                            || name.starts_with(&format!("{stem}."))
                    })
            })
            .collect::<Vec<_>>();
            sources.sort();
            if !sources.iter().any(|source| source == &artifact) {
                return Err(WorkbenchError::new("workspace artifact disappeared"));
            }
            Ok(sources)
        }
    }
}

fn public_workspace_group_path(
    repository_root: &Path,
    id: &str,
) -> Result<PathBuf, WorkbenchError> {
    let catalog = load_project_catalog(repository_root)?;
    if !catalog.groups.contains_key(id) {
        return Err(WorkbenchError::new(format!(
            "unknown or private workspace folder {id:?}"
        )));
    }
    workspace_group_path(repository_root, id)
}

fn workspace_group_path(repository_root: &Path, id: &str) -> Result<PathBuf, WorkbenchError> {
    let relative = Path::new(id);
    if !id.starts_with(&format!("{PROJECT_WORKSPACE_PATH}/")) && id != PROJECT_WORKSPACE_PATH {
        return Err(WorkbenchError::new(
            "workspace folder is outside the routes root",
        ));
    }
    if relative
        .components()
        .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(WorkbenchError::new("workspace folder path is invalid"));
    }
    let path = repository_root.join(relative);
    let canonical = fs::canonicalize(&path).map_err(|error| {
        WorkbenchError::new(format!("cannot resolve {}: {error}", path.display()))
    })?;
    let root = fs::canonicalize(repository_root.join(PROJECT_WORKSPACE_PATH))
        .map_err(|error| WorkbenchError::new(error.to_string()))?;
    if !canonical.starts_with(&root) || !canonical.is_dir() {
        return Err(WorkbenchError::new("workspace folder is invalid"));
    }
    Ok(canonical)
}

fn reject_active_timeline_move(
    active_timeline: &Path,
    sources: &[PathBuf],
) -> Result<(), WorkbenchError> {
    let active = fs::canonicalize(active_timeline).ok();
    if active.as_ref().is_some_and(|active| {
        sources.iter().any(|source| {
            fs::canonicalize(source)
                .ok()
                .is_some_and(|source| *active == source || active.starts_with(&source))
        })
    }) {
        return Err(WorkbenchError::new(
            "the currently open timeline cannot be moved or deleted while the workbench is running",
        ));
    }
    Ok(())
}

fn validate_workspace_name(name: &str) -> Result<(), WorkbenchError> {
    let invalid_character = name
        .chars()
        .any(|character| character.is_control() || r#"<>:"/\|?*"#.contains(character));
    let windows_stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    let windows_reserved = matches!(windows_stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || windows_stem
            .strip_prefix("COM")
            .or_else(|| windows_stem.strip_prefix("LPT"))
            .is_some_and(|suffix| {
                matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
            });
    if name.is_empty()
        || name.len() > 64
        || name.trim() != name
        || name.ends_with('.')
        || matches!(name, "." | "..")
        || invalid_character
        || windows_reserved
    {
        return Err(WorkbenchError::new(
            "folder names must be 1-64 characters and portable across Git, Windows, and macOS",
        ));
    }
    Ok(())
}

fn load_project_tape(
    repository_root: &Path,
    project: &ProjectDefinition,
) -> Result<InputTape, WorkbenchError> {
    let path = checked_artifact_path(repository_root, &project.artifact)?;
    let mut tape = match project.kind {
        ProjectKind::Tas => {
            let source = fs::read_to_string(&path).map_err(|error| {
                WorkbenchError::new(format!("cannot read {}: {error}", path.display()))
            })?;
            crate::tape_dsl::parse(&source)
                .map_err(|error| {
                    WorkbenchError::new(format!("cannot parse {}: {error}", path.display()))
                })?
                .compile()
                .map(|compiled| compiled.tape)
                .map_err(|error| {
                    WorkbenchError::new(format!("cannot compile {}: {error}", path.display()))
                })?
        }
        ProjectKind::Tape => {
            InputTape::decode(&fs::read(&path).map_err(|error| {
                WorkbenchError::new(format!("cannot read {}: {error}", path.display()))
            })?)
            .map_err(|error| {
                WorkbenchError::new(format!("cannot decode {}: {error}", path.display()))
            })?
            .tape
        }
        ProjectKind::Timeline => {
            return Err(WorkbenchError::new("timeline is not a standalone tape"));
        }
    };
    if let Some(fixture_path) = &project.fixture {
        let path = checked_artifact_path(repository_root, fixture_path)?;
        let fixture: ScenarioFixture =
            serde_json::from_slice(&fs::read(&path).map_err(|error| {
                WorkbenchError::new(format!("cannot read {}: {error}", path.display()))
            })?)
            .map_err(|error| {
                WorkbenchError::new(format!("cannot decode {}: {error}", path.display()))
            })?;
        fixture.validate().map_err(|error| {
            WorkbenchError::new(format!("invalid fixture {}: {error}", path.display()))
        })?;
        match &mut tape.boot {
            TapeBoot::Stage {
                fixture: target, ..
            } if target.is_none() => *target = Some(fixture),
            TapeBoot::Stage { .. } => {
                return Err(WorkbenchError::new(format!(
                    "workspace tape {:?} already embeds a fixture",
                    project.id
                )));
            }
            TapeBoot::Process => {
                return Err(WorkbenchError::new(format!(
                    "workspace tape {:?} fixture requires stage boot",
                    project.id
                )));
            }
        }
    }
    Ok(tape)
}

fn scan_workspace(root: &Path) -> Result<(Vec<PathBuf>, Vec<PathBuf>), WorkbenchError> {
    fn visit(
        path: &Path,
        files: &mut Vec<PathBuf>,
        directories: &mut Vec<PathBuf>,
    ) -> Result<(), WorkbenchError> {
        let mut entries = fs::read_dir(path)
            .map_err(|error| {
                WorkbenchError::new(format!("cannot scan {}: {error}", path.display()))
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| {
                WorkbenchError::new(format!("cannot scan {}: {error}", path.display()))
            })?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let file_type = entry
                .file_type()
                .map_err(|error| WorkbenchError::new(error.to_string()))?;
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                directories.push(entry.path());
                visit(&entry.path(), files, directories)?;
            } else if file_type.is_file() {
                files.push(entry.path());
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    let mut directories = Vec::new();
    visit(root, &mut files, &mut directories)?;
    Ok((files, directories))
}

fn extension(path: &Path) -> Option<&str> {
    path.extension().and_then(|value| value.to_str())
}

fn slash_path(path: &Path) -> Result<String, WorkbenchError> {
    let components = path
        .components()
        .map(|component| component.as_os_str().to_str())
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| WorkbenchError::new("workspace path is not UTF-8"))?;
    if components.is_empty()
        || components.iter().any(|component| {
            component.is_empty()
                || *component == "."
                || *component == ".."
                || component.chars().any(char::is_control)
        })
    {
        return Err(WorkbenchError::new("workspace path is invalid"));
    }
    Ok(components.join("/"))
}

fn add_group_ancestry(
    groups: &mut BTreeMap<String, String>,
    path: &Path,
) -> Result<(), WorkbenchError> {
    let mut cursor = PathBuf::new();
    for component in path.components() {
        cursor.push(component.as_os_str());
        let id = slash_path(&cursor)?;
        let label = human_label(component.as_os_str().to_str().unwrap_or("folder"));
        groups.entry(id).or_insert(label);
    }
    Ok(())
}

fn human_label(value: &str) -> String {
    let mut output = String::new();
    for word in value.split(['_', '-']).filter(|word| !word.is_empty()) {
        if !output.is_empty() {
            output.push(' ');
        }
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            output.extend(first.to_uppercase());
            output.extend(chars);
        }
    }
    if output.is_empty() {
        value.into()
    } else {
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_repository(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let repository = std::env::temp_dir().join(format!(
            "dusklight-workspace-{name}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(repository.join("routes")).unwrap();
        repository
    }

    #[test]
    fn checked_workspace_projects_real_folders_and_hides_route_internals() {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../..")
            .canonicalize()
            .unwrap();
        let catalog = load_project_catalog(&repository).unwrap();
        assert!(catalog.entries.contains_key("routes/intro"));
        assert_eq!(catalog.entries["routes/intro"].kind, ProjectKind::Timeline);
        assert!(
            !catalog
                .entries
                .contains_key("routes/intro/segments/golf439")
        );
        assert!(catalog.groups.contains_key("routes"));
        assert!(!catalog.groups.contains_key("routes/intro"));
    }

    #[test]
    fn route_private_storage_is_hidden_and_rejected_by_workspace_crud() {
        let repository = temporary_repository("private-route-storage");
        fs::write(
            repository.join("routes/private.timeline"),
            "timeline private\n",
        )
        .unwrap();
        fs::create_dir_all(repository.join("routes/private/segments")).unwrap();
        fs::create_dir_all(repository.join("routes/private/variants")).unwrap();

        let catalog = load_project_catalog(&repository).unwrap();
        assert!(catalog.entries.contains_key("routes/private"));
        assert!(!catalog.groups.contains_key("routes/private"));
        assert!(!catalog.groups.contains_key("routes/private/segments"));
        assert!(!catalog.groups.contains_key("routes/private/variants"));

        let error = workspace_node_sources(
            &repository,
            "routes/private/segments",
            WorkspaceNodeKind::Folder,
        )
        .unwrap_err();
        assert!(error.to_string().contains("private workspace folder"));
        let error = create_workspace_folder(
            &repository,
            &BrowserWorkspaceFolderCreateRequest {
                parent: "routes/private".into(),
                name: "forged-child".into(),
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("private workspace folder"));

        fs::remove_dir_all(repository).unwrap();
    }

    #[test]
    fn every_workspace_entry_loads_or_parses() {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../..")
            .canonicalize()
            .unwrap();
        let catalog = load_project_catalog(&repository).unwrap();
        for entry in catalog.entries.values() {
            match entry.kind {
                ProjectKind::Timeline => {
                    load_authoritative_timeline(&repository.join(&entry.artifact)).unwrap();
                }
                ProjectKind::Tas | ProjectKind::Tape => {
                    load_project_tape(&repository, entry).unwrap();
                    load_project_native_oracle(&repository, entry).unwrap();
                }
            }
        }
    }

    #[test]
    fn boot_overrides_and_workspace_crud_move_artifacts_as_one_git_unit() {
        let repository = temporary_repository("crud");
        fs::create_dir_all(repository.join("routes/qa")).unwrap();
        fs::create_dir_all(repository.join("routes/empty")).unwrap();
        fs::write(
            repository.join("routes/qa/canary.tape"),
            InputTape::default().encode().unwrap(),
        )
        .unwrap();
        fs::write(
            repository.join("routes/qa/canary.launch"),
            "dusklaunch 1\noracle eye_shredder\n",
        )
        .unwrap();

        let catalog = load_project_catalog(&repository).unwrap();
        assert!(catalog.groups.contains_key("routes/empty"));
        assert!(catalog.entries.contains_key("routes/qa/canary"));

        let stage_boot = TapeBoot::Stage {
            stage: "F_SP103".into(),
            room: 1,
            point: 0,
            layer: -1,
            save_slot: None,
            fixture: None,
        };
        update_boot_override(
            &repository,
            &BrowserBootOverrideUpdateRequest {
                project: "routes/qa/canary".into(),
                enabled: true,
                boot: stage_boot.clone(),
            },
        )
        .unwrap();
        let materialized = project_materialized_playback(&repository, "routes/qa/canary").unwrap();
        assert!(matches!(
            materialized.tape.boot,
            TapeBoot::Stage { ref stage, .. } if stage == "F_SP103"
        ));
        assert_eq!(
            materialized.native_oracle,
            NativePlaybackOracle::EyeShredder
        );

        create_workspace_folder(
            &repository,
            &BrowserWorkspaceFolderCreateRequest {
                parent: "routes".into(),
                name: "moved".into(),
            },
        )
        .unwrap();
        move_workspace_node(
            &repository,
            &repository.join("not-active.timeline"),
            &BrowserWorkspaceMoveRequest {
                id: "routes/qa/canary".into(),
                kind: WorkspaceNodeKind::Project,
                destination: "routes/moved".into(),
            },
        )
        .unwrap();
        assert!(repository.join("routes/moved/canary.tape").is_file());
        assert!(repository.join("routes/moved/canary.boot.json").is_file());
        assert!(repository.join("routes/moved/canary.launch").is_file());
        assert!(!repository.join("routes/qa/canary.tape").exists());

        let state_root = repository.join("state");
        let deletion = delete_workspace_node(
            &repository,
            &repository.join("not-active.timeline"),
            &state_root,
            &BrowserWorkspaceDeleteRequest {
                id: "routes/moved/canary".into(),
                kind: WorkspaceNodeKind::Project,
            },
        )
        .unwrap();
        let trash = deletion.trash.unwrap();
        assert!(trash.join("canary.tape").is_file());
        assert!(trash.join("canary.boot.json").is_file());
        assert!(trash.join("canary.launch").is_file());
        assert!(!repository.join("routes/moved/canary.tape").exists());
        fs::remove_dir_all(repository).unwrap();
    }

    #[test]
    fn workspace_rejects_moving_a_folder_into_its_descendant() {
        let repository = temporary_repository("cycle");
        fs::create_dir_all(repository.join("routes/a/b")).unwrap();
        let error = move_workspace_node(
            &repository,
            &repository.join("not-active.timeline"),
            &BrowserWorkspaceMoveRequest {
                id: "routes/a".into(),
                kind: WorkspaceNodeKind::Folder,
                destination: "routes/a/b".into(),
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("cannot be moved into itself"));
        fs::remove_dir_all(repository).unwrap();
    }

    #[test]
    fn workspace_creates_playable_tapes_and_clones_every_sidecar() {
        let repository = temporary_repository("create-clone");
        fs::create_dir_all(repository.join("routes/QA")).unwrap();
        let created = create_workspace_tape(
            &repository,
            &BrowserWorkspaceTapeCreateRequest {
                parent: "routes/QA".into(),
                name: "Blank Boot".into(),
            },
        )
        .unwrap();
        assert_eq!(created.id, "routes/QA/Blank Boot");
        let catalog = load_project_catalog(&repository).unwrap();
        let entry = &catalog.entries[&created.id];
        let tape = load_project_tape(&repository, entry).unwrap();
        assert_eq!(tape.frames.len(), 1);

        let invalid_boot: TapeBoot = serde_json::from_value(serde_json::json!({
            "kind": "stage",
            "stage": "F_SP103",
            "room": 1,
            "point": 0,
            "layer": -1,
            "fixture": {
                "schema": "dusklight-scenario-fixture/v1",
                "name": "invalid native slot",
                "inventory": [{"slot": 24, "item": 64, "quantity": 1}]
            }
        }))
        .unwrap();
        let error = update_boot_override(
            &repository,
            &BrowserBootOverrideUpdateRequest {
                project: created.id.clone(),
                enabled: true,
                boot: invalid_boot,
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("native save limits"));

        let boot = br#"{"schema":"dusklight.route-workbench.boot-override.v1","enabled":false,"boot":{"kind":"process"}}"#;
        let launch = b"dusklaunch 1\noracle eye_shredder\n";
        fs::write(repository.join("routes/QA/Blank Boot.boot.json"), boot).unwrap();
        fs::write(repository.join("routes/QA/Blank Boot.launch"), launch).unwrap();
        let cloned = clone_workspace_tape(
            &repository,
            &BrowserWorkspaceTapeCloneRequest {
                source: created.id,
                destination: "routes/QA".into(),
                name: "Configured Copy".into(),
            },
        )
        .unwrap();
        assert_eq!(cloned.id, "routes/QA/Configured Copy");
        assert_eq!(
            fs::read(repository.join("routes/QA/Configured Copy.boot.json")).unwrap(),
            boot
        );
        assert_eq!(
            fs::read(repository.join("routes/QA/Configured Copy.launch")).unwrap(),
            launch
        );
        assert!(repository.join("routes/QA/Configured Copy.tas").is_file());

        fs::remove_dir_all(repository).unwrap();
    }

    #[test]
    fn workspace_folder_names_preserve_human_casing_and_spaces() {
        for valid in ["QA", "Intro Segments", "Glitch-Hunt_01", "Élite Routes"] {
            validate_workspace_name(valid).unwrap();
        }
        for invalid in [
            "",
            ".",
            "..",
            " trailing ",
            "trailing.",
            "a/b",
            r"a\b",
            "CON",
            "con.txt",
            "LPT9",
        ] {
            assert!(
                validate_workspace_name(invalid).is_err(),
                "accepted unsafe workspace folder {invalid:?}"
            );
        }
    }
}
