//! Resolves workbench entities to trusted local paths and reveals them in the host file browser.

use super::*;

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrowserRevealTarget {
    Segment { id: String },
    Subgraph { id: String },
    Project { id: String },
    Folder { id: String },
    Draft { id: String },
    Campaign { id: String },
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserRevealRequest {
    pub target: BrowserRevealTarget,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeRevealKind {
    File,
    Folder,
}

#[derive(Clone, Debug, Serialize)]
pub struct NativeRevealResult {
    pub kind: NativeRevealKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct NativeRevealTarget {
    pub kind: NativeRevealKind,
    pub path: PathBuf,
}

pub(super) fn reveal_entity(
    config: &WorkbenchConfig,
    request: &BrowserRevealRequest,
) -> Result<NativeRevealResult, WorkbenchError> {
    let target = resolve_reveal_target(config, &request.target)?;
    spawn_native_reveal(&target)?;
    Ok(NativeRevealResult { kind: target.kind })
}

pub(super) fn resolve_reveal_target(
    config: &WorkbenchConfig,
    target: &BrowserRevealTarget,
) -> Result<NativeRevealTarget, WorkbenchError> {
    match target {
        BrowserRevealTarget::Segment { id } => {
            let timeline = load_authoritative_timeline(&config.timeline_path)?;
            let segment = timeline
                .segments
                .get(id)
                .ok_or_else(|| WorkbenchError::new(format!("unknown segment {id:?}")))?;
            let path = match &segment.artifact {
                ArtifactSource::Candidate(path)
                | ArtifactSource::Tas(path)
                | ArtifactSource::Tape(path) => {
                    checked_artifact_path(&configured_artifact_root(config)?, path)?
                }
                ArtifactSource::Baseline(_) => checked_repository_file(
                    &config.repository_root,
                    &config.timeline_path,
                    "timeline",
                )?,
                #[allow(unreachable_patterns)]
                _ => {
                    return Err(WorkbenchError::new(
                        "segment artifact cannot be revealed by this workbench build",
                    ));
                }
            };
            Ok(NativeRevealTarget {
                kind: NativeRevealKind::File,
                path,
            })
        }
        BrowserRevealTarget::Subgraph { id } => {
            let timeline = load_authoritative_timeline(&config.timeline_path)?;
            let subgraph = timeline
                .subgraphs
                .get(id)
                .ok_or_else(|| WorkbenchError::new(format!("unknown subgraph {id:?}")))?;
            let artifact_root = configured_artifact_root(config)?;
            let mut folders = Vec::new();
            for segment_id in timeline.subgraph_segment_closure(&subgraph.id) {
                let path = match &timeline.segments[&segment_id].artifact {
                    ArtifactSource::Candidate(path)
                    | ArtifactSource::Tas(path)
                    | ArtifactSource::Tape(path) => path,
                    ArtifactSource::Baseline(_) => continue,
                    #[allow(unreachable_patterns)]
                    _ => continue,
                };
                let artifact = checked_artifact_path(&artifact_root, path)?;
                folders.push(
                    artifact
                        .parent()
                        .ok_or_else(|| WorkbenchError::new("segment artifact has no folder"))?
                        .to_path_buf(),
                );
            }
            let timeline_file = checked_repository_file(
                &config.repository_root,
                &config.timeline_path,
                "timeline",
            )?;
            let mut common = folders.first().cloned().unwrap_or_else(|| {
                timeline_file
                    .parent()
                    .expect("canonical timeline file has a parent")
                    .to_path_buf()
            });
            for folder in folders.iter().skip(1) {
                while !folder.starts_with(&common) {
                    if !common.pop() {
                        return Err(WorkbenchError::new(
                            "subgraph artifacts have no common repository folder",
                        ));
                    }
                }
            }
            Ok(NativeRevealTarget {
                kind: NativeRevealKind::Folder,
                path: common,
            })
        }
        BrowserRevealTarget::Project { id } => {
            let catalog = load_project_catalog(&config.repository_root)?;
            let project = catalog
                .entries
                .get(id)
                .ok_or_else(|| WorkbenchError::new(format!("unknown workspace project {id:?}")))?;
            Ok(NativeRevealTarget {
                kind: NativeRevealKind::File,
                path: checked_artifact_path(&config.repository_root, &project.artifact)?,
            })
        }
        BrowserRevealTarget::Folder { id } => {
            let catalog = load_project_catalog(&config.repository_root)?;
            if !catalog.groups.contains_key(id) {
                return Err(WorkbenchError::new(format!(
                    "unknown workspace folder {id:?}"
                )));
            }
            Ok(NativeRevealTarget {
                kind: NativeRevealKind::Folder,
                path: checked_repository_directory(&config.repository_root, Path::new(id))?,
            })
        }
        BrowserRevealTarget::Draft { id } => {
            let root = validated_drafts_root(&config.state_root)?;
            Ok(NativeRevealTarget {
                kind: NativeRevealKind::Folder,
                path: validated_draft_directory(&root, id)?,
            })
        }
        BrowserRevealTarget::Campaign { id } => {
            let timeline = load_authoritative_timeline(&config.timeline_path)?;
            let artifact_root = configured_artifact_root(config)?;
            let mut graph = graph_from_timeline(&timeline, &artifact_root)?;
            append_optimization_campaigns(
                &mut graph,
                &config.repository_root,
                &config.timeline_path,
            )?;
            let campaign = graph
                .campaigns
                .iter()
                .find(|campaign| campaign.id == *id)
                .ok_or_else(|| WorkbenchError::new(format!("unknown campaign {id:?}")))?;
            Ok(NativeRevealTarget {
                kind: NativeRevealKind::File,
                path: checked_repository_file(
                    &config.repository_root,
                    &config.repository_root.join(&campaign.request),
                    "optimization request",
                )?,
            })
        }
    }
}

fn checked_repository_file(
    repository_root: &Path,
    path: &Path,
    label: &str,
) -> Result<PathBuf, WorkbenchError> {
    let root = fs::canonicalize(repository_root).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot resolve repository root {}: {error}",
            repository_root.display()
        ))
    })?;
    let path = fs::canonicalize(path)
        .map_err(|error| WorkbenchError::new(format!("cannot resolve {label}: {error}")))?;
    if !path.starts_with(&root) || !path.is_file() {
        return Err(WorkbenchError::new(format!(
            "{label} is not a file inside the repository"
        )));
    }
    Ok(path)
}

fn checked_repository_directory(
    repository_root: &Path,
    relative: &Path,
) -> Result<PathBuf, WorkbenchError> {
    if relative.is_absolute() {
        return Err(WorkbenchError::new(
            "workspace folder must be repository-relative",
        ));
    }
    let root = fs::canonicalize(repository_root).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot resolve repository root {}: {error}",
            repository_root.display()
        ))
    })?;
    let path = fs::canonicalize(root.join(relative)).map_err(|error| {
        WorkbenchError::new(format!("cannot resolve workspace folder: {error}"))
    })?;
    if !path.starts_with(&root) || !path.is_dir() {
        return Err(WorkbenchError::new(
            "workspace folder is not a directory inside the repository",
        ));
    }
    Ok(path)
}

fn spawn_native_reveal(target: &NativeRevealTarget) -> Result<(), WorkbenchError> {
    let mut command = native_reveal_command(target);
    command.spawn().map(|_| ()).map_err(|error| {
        WorkbenchError::new(format!("cannot open the native file browser: {error}"))
    })
}

pub(super) fn native_reveal_command(target: &NativeRevealTarget) -> Command {
    #[cfg(target_os = "windows")]
    {
        let mut command = Command::new("explorer.exe");
        if target.kind == NativeRevealKind::File {
            command.arg("/select,");
        }
        command.arg(&target.path);
        command
    }
    #[cfg(target_os = "macos")]
    {
        let mut command = Command::new("open");
        if target.kind == NativeRevealKind::File {
            command.arg("-R");
        }
        command.arg(&target.path);
        command
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let mut command = Command::new("xdg-open");
        command.arg(if target.kind == NativeRevealKind::File {
            target.path.parent().unwrap_or(&target.path)
        } else {
            &target.path
        });
        command
    }
}
