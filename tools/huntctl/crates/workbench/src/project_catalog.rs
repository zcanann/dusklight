//! Checked project catalog and standalone boot-tape loading.

use super::*;
use crate::scenario_fixture::ScenarioFixture;

#[derive(Clone, Debug)]
pub(super) struct ProjectDefinition {
    pub id: String,
    pub label: String,
    pub group: String,
    pub artifact: ArtifactSource,
    pub fixture: Option<PathBuf>,
}

#[derive(Clone, Debug, Default)]
pub(super) struct ProjectCatalog {
    pub groups: BTreeMap<String, String>,
    pub entries: BTreeMap<String, ProjectDefinition>,
}

pub(super) fn load_project_catalog(
    repository_root: &Path,
) -> Result<ProjectCatalog, WorkbenchError> {
    let path = repository_root.join(PROJECT_CATALOG_PATH);
    if !path.is_file() {
        return Ok(ProjectCatalog::default());
    }
    let source = fs::read_to_string(&path)
        .map_err(|error| WorkbenchError::new(format!("cannot read {}: {error}", path.display())))?;
    parse_project_catalog(&source)
}

pub(super) fn project_catalog_projection(
    repository_root: &Path,
) -> Result<GraphProjectCatalog, WorkbenchError> {
    let catalog = load_project_catalog(repository_root)?;
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
            let loaded = load_project_tape(repository_root, entry);
            let (boot, frame_count, error) = match loaded {
                Ok(tape) => (Some(tape.boot), Some(tape.frames.len() as u64), None),
                Err(error) => (None, None, Some(error.to_string())),
            };
            GraphProject {
                id: entry.id.clone(),
                label: entry.label.clone(),
                group: entry.group.clone(),
                artifact: graph_artifact(&entry.artifact),
                boot,
                frame_count,
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
        .ok_or_else(|| WorkbenchError::new(format!("unknown project tape {project_id:?}")))?;
    let tape = load_project_tape(repository_root, project)?;
    Ok(MaterializedPlayback {
        lineage: None,
        segment: Some(format!("project:{project_id}")),
        tape,
        seed_stage: None,
    })
}

fn parse_project_catalog(source: &str) -> Result<ProjectCatalog, WorkbenchError> {
    let mut catalog = ProjectCatalog::default();
    let mut saw_header = false;
    for (index, raw) in source.lines().enumerate() {
        let line_number = index + 1;
        let tokens = tokenize(raw, line_number).map_err(|error| {
            WorkbenchError::new(format!("project catalog line {line_number}: {error}"))
        })?;
        if tokens.is_empty() {
            continue;
        }
        match tokens[0].as_str() {
            "projects" if !saw_header && tokens.as_slice() == ["projects", "1"] => {
                saw_header = true;
            }
            "group" if saw_header && tokens.len() == 3 => {
                validate_catalog_id(&tokens[1], true, line_number)?;
                if catalog
                    .groups
                    .insert(tokens[1].clone(), tokens[2].clone())
                    .is_some()
                {
                    return Err(WorkbenchError::new(format!(
                        "project catalog line {line_number}: duplicate group {:?}",
                        tokens[1]
                    )));
                }
            }
            "project" if saw_header => {
                if tokens.len() != 8 || tokens[3] != "in" || tokens[5] != "uses" {
                    return Err(WorkbenchError::new(format!(
                        "project catalog line {line_number}: expected `project ID LABEL in GROUP uses tas|tape PATH [fixture PATH]`"
                    )));
                }
                validate_catalog_id(&tokens[1], false, line_number)?;
                let artifact = match tokens[6].as_str() {
                    "tas" => ArtifactSource::Tas(PathBuf::from(&tokens[7])),
                    "tape" => ArtifactSource::Tape(PathBuf::from(&tokens[7])),
                    other => {
                        return Err(WorkbenchError::new(format!(
                            "project catalog line {line_number}: unsupported artifact kind {other:?}"
                        )));
                    }
                };
                let definition = ProjectDefinition {
                    id: tokens[1].clone(),
                    label: tokens[2].clone(),
                    group: tokens[4].clone(),
                    artifact,
                    fixture: None,
                };
                if catalog
                    .entries
                    .insert(definition.id.clone(), definition)
                    .is_some()
                {
                    return Err(WorkbenchError::new(format!(
                        "project catalog line {line_number}: duplicate project {:?}",
                        tokens[1]
                    )));
                }
            }
            "project" if !saw_header => {
                return Err(WorkbenchError::new(
                    "project catalog must begin with `projects 1`",
                ));
            }
            "fixture" if saw_header && tokens.len() == 3 => {
                let project = catalog.entries.get_mut(&tokens[1]).ok_or_else(|| {
                    WorkbenchError::new(format!(
                        "project catalog line {line_number}: fixture references unknown project {:?}", tokens[1]
                    ))
                })?;
                if project.fixture.replace(PathBuf::from(&tokens[2])).is_some() {
                    return Err(WorkbenchError::new(format!(
                        "project catalog line {line_number}: duplicate fixture for {:?}",
                        tokens[1]
                    )));
                }
            }
            _ => {
                return Err(WorkbenchError::new(format!(
                    "project catalog line {line_number}: invalid declaration"
                )));
            }
        }
    }
    if !saw_header {
        return Err(WorkbenchError::new(
            "project catalog must begin with `projects 1`",
        ));
    }
    for group in catalog.groups.keys() {
        if let Some((parent, _)) = group.rsplit_once('/')
            && !catalog.groups.contains_key(parent)
        {
            return Err(WorkbenchError::new(format!(
                "project group {group:?} has missing parent {parent:?}"
            )));
        }
    }
    for project in catalog.entries.values() {
        if !catalog.groups.contains_key(&project.group) {
            return Err(WorkbenchError::new(format!(
                "project {:?} references missing group {:?}",
                project.id, project.group
            )));
        }
    }
    Ok(catalog)
}

fn validate_catalog_id(id: &str, allow_slash: bool, line: usize) -> Result<(), WorkbenchError> {
    let valid = !id.is_empty()
        && id.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || byte == b'_'
                || byte == b'-'
                || (allow_slash && byte == b'/')
        })
        && !id.starts_with('/')
        && !id.ends_with('/')
        && !id.contains("//");
    if valid {
        Ok(())
    } else {
        Err(WorkbenchError::new(format!(
            "project catalog line {line}: invalid id {id:?}"
        )))
    }
}

fn load_project_tape(
    repository_root: &Path,
    project: &ProjectDefinition,
) -> Result<InputTape, WorkbenchError> {
    let mut tape = match &project.artifact {
        ArtifactSource::Tas(path) => {
            let path = checked_artifact_path(repository_root, path)?;
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
        ArtifactSource::Tape(path) => {
            let path = checked_artifact_path(repository_root, path)?;
            let bytes = fs::read(&path).map_err(|error| {
                WorkbenchError::new(format!("cannot read {}: {error}", path.display()))
            })?;
            InputTape::decode(&bytes)
                .map_err(|error| {
                    WorkbenchError::new(format!("cannot decode {}: {error}", path.display()))
                })?
                .tape
        }
        _ => {
            return Err(WorkbenchError::new(
                "project artifacts must be TAS source or compiled tape",
            ));
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
                    "project {:?} tape already embeds a fixture",
                    project.id
                )));
            }
            TapeBoot::Process => {
                return Err(WorkbenchError::new(format!(
                    "project {:?} fixture requires a stage-boot tape",
                    project.id
                )));
            }
        }
    }
    Ok(tape)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_groups_and_fixture_metadata() {
        let catalog = parse_project_catalog(
            "projects 1\ngroup qa \"QA\"\ngroup qa/maps \"Map boots\"\nproject stage \"Stage\" in qa/maps uses tas tests/stage.tas\nfixture stage tests/stage.fixture.json\n",
        )
        .unwrap();
        assert_eq!(catalog.groups["qa/maps"], "Map boots");
        assert_eq!(catalog.entries["stage"].group, "qa/maps");
        assert_eq!(
            catalog.entries["stage"].fixture.as_deref(),
            Some(Path::new("tests/stage.fixture.json"))
        );
    }

    #[test]
    fn rejects_projects_in_unknown_groups() {
        let error = parse_project_catalog(
            "projects 1\nproject loose \"Loose\" in missing uses tas tests/loose.tas\n",
        )
        .unwrap_err();
        assert!(error.to_string().contains("missing group"));
    }

    #[test]
    fn checked_catalog_covers_and_loads_every_standalone_qa_tape() {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../..")
            .canonicalize()
            .unwrap();
        let catalog = load_project_catalog(&repository).unwrap();
        let listed = catalog
            .entries
            .values()
            .map(|entry| match &entry.artifact {
                ArtifactSource::Tas(path) | ArtifactSource::Tape(path) => path.clone(),
                _ => unreachable!(),
            })
            .collect::<BTreeSet<_>>();
        let fixture_root = repository.join("tests/fixtures/automation");
        let expected = fs::read_dir(&fixture_root)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                matches!(
                    path.extension().and_then(|value| value.to_str()),
                    Some("tas" | "tape")
                )
            })
            .map(|path| path.strip_prefix(&repository).unwrap().to_path_buf())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            listed, expected,
            "every checked standalone QA tape must appear in Projects"
        );

        let projection = project_catalog_projection(&repository).unwrap();
        assert_eq!(projection.entries.len(), expected.len());
        assert!(projection.entries.iter().all(|entry| entry.playable));
        let stage = projection
            .entries
            .iter()
            .find(|entry| entry.id == "fsp103_next_map_seed")
            .unwrap();
        assert!(matches!(
            stage.boot,
            Some(TapeBoot::Stage {
                fixture: Some(_),
                ..
            })
        ));
    }
}
