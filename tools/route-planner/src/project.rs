//! Planner-owned web project artifacts and conflict-safe workspace storage.

use dusklight_route_planner::PlannerContractError;
use dusklight_route_planner::artifact::Digest;
use dusklight_route_planner::execution::PlannerExecutionStateDocument;
use dusklight_route_planner::graph::PlannerGraph;
use dusklight_route_planner::identity::{
    CONTENT_IDENTITY_SCHEMA, ContentFingerprint, ContentIdentity, EquivalenceSet, GamePlatform,
    GameRegion, RUNTIME_CONFIGURATION_SCHEMA, RuntimeConfiguration,
};
use dusklight_route_planner::logic::{FACT_CATALOG_SCHEMA, FactCatalog};
use dusklight_route_planner::refinement::ComposedPlannerCatalog;
use dusklight_route_planner::return_place::{
    GZ2E01_CONTENT_SHA256, gz2e01_tower_return_place_mechanics,
};
use dusklight_route_planner::route_book::RouteBook;
use dusklight_route_planner::title_boundary::gz2e01_reset_to_opening_mechanics;
use dusklight_route_planner::transition::{MECHANICS_CATALOG_SCHEMA, MechanicsCatalog};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
#[cfg(not(windows))]
use std::fs::File;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub const WEB_PROJECT_SCHEMA: &str = "dusklight.route-planner.web-project/v1";
pub const WEB_PROJECT_LIST_SCHEMA: &str = "dusklight.route-planner.web-project-list/v1";
pub const WEB_PROJECT_RECORD_SCHEMA: &str = "dusklight.route-planner.web-project-record/v1";
pub const WEB_PROJECT_SAVE_SCHEMA: &str = "dusklight.route-planner.web-project-save/v1";
static NEXT_TEMPORARY_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlannerWebProject {
    pub schema: String,
    pub id: String,
    pub label: String,
    pub catalog: ComposedPlannerCatalog,
    #[serde(default)]
    pub route_book: Option<RouteBook>,
    #[serde(default)]
    pub start_state: Option<PlannerExecutionStateDocument>,
    #[serde(default)]
    pub equivalence_sets: Vec<EquivalenceSet>,
    #[serde(default)]
    pub presentation: ProjectPresentation,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectPresentation {
    #[serde(default)]
    pub positions: BTreeMap<String, NodePosition>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NodePosition {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectListing {
    pub id: String,
    pub label: String,
    pub revision_sha256: Digest,
    pub read_only: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectList {
    pub schema: String,
    pub projects: Vec<ProjectListing>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectRecord {
    pub schema: String,
    pub revision_sha256: Digest,
    pub read_only: bool,
    pub project: PlannerWebProject,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectSaveRequest {
    pub schema: String,
    pub expected_revision_sha256: Option<Digest>,
    pub project: PlannerWebProject,
}

#[derive(Debug)]
pub struct ProjectStore {
    root: PathBuf,
    builtins: BTreeMap<String, PlannerWebProject>,
}

impl PlannerWebProject {
    pub fn blank(id: impl Into<String>, label: impl Into<String>) -> Result<Self, ProjectError> {
        let facts = FactCatalog {
            schema: FACT_CATALOG_SCHEMA.into(),
            aliases: Vec::new(),
            derived_facts: Vec::new(),
        };
        let mechanics = empty_mechanics();
        let catalog = ComposedPlannerCatalog::compose(&facts, &mechanics, &[])?;
        let project = Self {
            schema: WEB_PROJECT_SCHEMA.into(),
            id: id.into(),
            label: label.into(),
            catalog,
            route_book: None,
            start_state: None,
            equivalence_sets: Vec::new(),
            presentation: ProjectPresentation::default(),
        };
        project.validate()?;
        Ok(project)
    }

    pub fn validate(&self) -> Result<(), ProjectError> {
        if self.schema != WEB_PROJECT_SCHEMA {
            return Err(project_error("project schema is unsupported"));
        }
        validate_project_id(&self.id)?;
        validate_label(&self.label)?;
        self.catalog.validate()?;
        if let Some(book) = &self.route_book {
            book.validate_against_composed(&self.catalog)?;
        }
        if let Some(state) = &self.start_state {
            state.clone().into_state()?;
        }
        for equivalence_set in &self.equivalence_sets {
            equivalence_set.validate()?;
        }
        let graph = if let Some(book) = &self.route_book {
            PlannerGraph::project_composed_with_route_book(&self.catalog, book)?
        } else {
            PlannerGraph::project_composed(&self.catalog)?
        };
        let node_ids = graph
            .nodes
            .iter()
            .map(|node| node.id.as_str())
            .collect::<BTreeSet<_>>();
        for (node_id, position) in &self.presentation.positions {
            if !node_ids.contains(node_id.as_str()) {
                return Err(project_error(format!(
                    "presentation position references unknown node {node_id}"
                )));
            }
            if !position.x.is_finite()
                || !position.y.is_finite()
                || position.x.abs() > 1_000_000.0
                || position.y.abs() > 1_000_000.0
            {
                return Err(project_error(format!(
                    "presentation position for {node_id} is outside the finite canvas"
                )));
            }
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, ProjectError> {
        self.validate()?;
        serde_json::to_vec(self).map_err(ProjectError::json)
    }

    pub fn digest(&self) -> Result<Digest, ProjectError> {
        let digest = Sha256::digest(self.canonical_bytes()?);
        let mut bytes = [0_u8; 32];
        bytes.copy_from_slice(&digest);
        Ok(Digest(bytes))
    }
}

impl ProjectStore {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, ProjectError> {
        let root = root.into();
        fs::create_dir_all(&root).map_err(ProjectError::io)?;
        let root = root.canonicalize().map_err(ProjectError::io)?;
        let builtins = builtin_projects()?
            .into_iter()
            .map(|project| (project.id.clone(), project))
            .collect();
        Ok(Self { root, builtins })
    }

    pub fn blank_template(&self) -> Result<ProjectRecord, ProjectError> {
        record(
            PlannerWebProject::blank("untitled", "Untitled route")?,
            false,
        )
    }

    pub fn list(&self) -> Result<ProjectList, ProjectError> {
        let mut projects = self
            .builtins
            .values()
            .map(|project| {
                Ok(ProjectListing {
                    id: project.id.clone(),
                    label: project.label.clone(),
                    revision_sha256: project.digest()?,
                    read_only: true,
                })
            })
            .collect::<Result<Vec<_>, ProjectError>>()?;
        for entry in fs::read_dir(&self.root).map_err(ProjectError::io)? {
            let entry = entry.map_err(ProjectError::io)?;
            let path = entry.path();
            if !entry.file_type().map_err(ProjectError::io)?.is_file()
                || path.extension().and_then(|value| value.to_str()) != Some("json")
            {
                continue;
            }
            let project = read_project(&path)?;
            let expected = project_path(&self.root, &project.id)?;
            if path != expected {
                return Err(project_error(format!(
                    "project file {} does not match project id {}",
                    path.display(),
                    project.id
                )));
            }
            projects.push(ProjectListing {
                id: project.id.clone(),
                label: project.label.clone(),
                revision_sha256: project.digest()?,
                read_only: false,
            });
        }
        projects.sort_by(|left, right| {
            left.read_only
                .cmp(&right.read_only)
                .reverse()
                .then_with(|| left.label.cmp(&right.label))
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(ProjectList {
            schema: WEB_PROJECT_LIST_SCHEMA.into(),
            projects,
        })
    }

    pub fn load(&self, id: &str) -> Result<ProjectRecord, ProjectError> {
        validate_project_id(id)?;
        if let Some(project) = self.builtins.get(id) {
            return record(project.clone(), true);
        }
        let path = project_path(&self.root, id)?;
        if !path.is_file() {
            return Err(project_error(format!("project {id} does not exist")));
        }
        record(read_project(&path)?, false)
    }

    pub fn save(
        &self,
        id: &str,
        request: ProjectSaveRequest,
    ) -> Result<ProjectRecord, ProjectError> {
        validate_project_id(id)?;
        if request.schema != WEB_PROJECT_SAVE_SCHEMA {
            return Err(project_error("project save schema is unsupported"));
        }
        if request.project.id != id {
            return Err(project_error("URL project id does not match the document"));
        }
        if self.builtins.contains_key(id) {
            return Err(project_error(
                "built-in projects are read-only; use Save As",
            ));
        }
        request.project.validate()?;
        let path = project_path(&self.root, id)?;
        let current_revision = if path.is_file() {
            Some(read_project(&path)?.digest()?)
        } else {
            None
        };
        if current_revision != request.expected_revision_sha256 {
            return Err(project_error(format!(
                "project revision conflict: expected {}, current {}",
                display_optional_digest(request.expected_revision_sha256),
                display_optional_digest(current_revision)
            )));
        }
        let bytes = serde_json::to_vec_pretty(&request.project).map_err(ProjectError::json)?;
        write_atomically(&path, &[bytes.as_slice(), b"\n"].concat())?;
        record(request.project, false)
    }
}

fn record(project: PlannerWebProject, read_only: bool) -> Result<ProjectRecord, ProjectError> {
    Ok(ProjectRecord {
        schema: WEB_PROJECT_RECORD_SCHEMA.into(),
        revision_sha256: project.digest()?,
        read_only,
        project,
    })
}

fn read_project(path: &Path) -> Result<PlannerWebProject, ProjectError> {
    let bytes = fs::read(path).map_err(ProjectError::io)?;
    let project: PlannerWebProject = serde_json::from_slice(&bytes).map_err(ProjectError::json)?;
    project.validate()?;
    Ok(project)
}

fn project_path(root: &Path, id: &str) -> Result<PathBuf, ProjectError> {
    validate_project_id(id)?;
    Ok(root.join(format!("{id}.json")))
}

fn validate_project_id(id: &str) -> Result<(), ProjectError> {
    if id.is_empty()
        || id.len() > 96
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        || matches!(id, "." | "..")
    {
        return Err(project_error(
            "project id must be 1-96 ASCII letters, digits, '.', '-' or '_'",
        ));
    }
    Ok(())
}

fn validate_label(label: &str) -> Result<(), ProjectError> {
    if label.trim().is_empty() || label.len() > 160 || label.chars().any(char::is_control) {
        return Err(project_error(
            "project label must be 1-160 printable characters",
        ));
    }
    Ok(())
}

fn display_optional_digest(value: Option<Digest>) -> String {
    value
        .map(|digest| digest.to_string())
        .unwrap_or_else(|| "none".into())
}

fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), ProjectError> {
    let temporary = path.with_extension(format!(
        "json.tmp-{}-{}",
        std::process::id(),
        NEXT_TEMPORARY_ID.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(ProjectError::io)?;
        output.write_all(bytes).map_err(ProjectError::io)?;
        output.sync_all().map_err(ProjectError::io)?;
        drop(output);
        replace_file(&temporary, path)?;
        sync_parent(path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> Result<(), ProjectError> {
    fs::rename(source, destination).map_err(ProjectError::io)
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> Result<(), ProjectError> {
    use std::os::windows::ffi::OsStrExt;
    const MOVEFILE_REPLACE_EXISTING: u32 = 0x0000_0001;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x0000_0008;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(existing: *const u16, replacement: *const u16, flags: u32) -> i32;
    }
    let source = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        Err(ProjectError::io(std::io::Error::last_os_error()))
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn sync_parent(path: &Path) -> Result<(), ProjectError> {
    let parent = path
        .parent()
        .ok_or_else(|| project_error("project path has no parent"))?;
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(ProjectError::io)
}

#[cfg(windows)]
fn sync_parent(path: &Path) -> Result<(), ProjectError> {
    if path.parent().is_some_and(Path::is_dir) {
        Ok(())
    } else {
        Err(project_error("project path has no directory parent"))
    }
}

fn builtin_projects() -> Result<Vec<PlannerWebProject>, ProjectError> {
    let (content, runtime) = gz2e01_context();
    let facts = FactCatalog {
        schema: FACT_CATALOG_SCHEMA.into(),
        aliases: Vec::new(),
        derived_facts: Vec::new(),
    };
    let fanadi = ComposedPlannerCatalog::compose(
        &facts,
        &gz2e01_tower_return_place_mechanics(&content, &runtime)?,
        &[],
    )?;
    let opening = ComposedPlannerCatalog::compose(
        &facts,
        &gz2e01_reset_to_opening_mechanics(&content, &runtime)?,
        &[],
    )?;
    let projects = vec![
        PlannerWebProject {
            schema: WEB_PROJECT_SCHEMA.into(),
            id: "demo-fanadi-return-place".into(),
            label: "Fanadi return-place locking".into(),
            catalog: fanadi,
            route_book: None,
            start_state: None,
            equivalence_sets: Vec::new(),
            presentation: ProjectPresentation::default(),
        },
        PlannerWebProject {
            schema: WEB_PROJECT_SCHEMA.into(),
            id: "demo-opening-flow".into(),
            label: "Opening and file-selection flow".into(),
            catalog: opening,
            route_book: None,
            start_state: None,
            equivalence_sets: Vec::new(),
            presentation: ProjectPresentation::default(),
        },
    ];
    for project in &projects {
        project.validate()?;
    }
    Ok(projects)
}

fn empty_mechanics() -> MechanicsCatalog {
    MechanicsCatalog {
        schema: MECHANICS_CATALOG_SCHEMA.into(),
        transitions: Vec::new(),
        obligations: Vec::new(),
        writers: Vec::new(),
        gates: Vec::new(),
        readers: Vec::new(),
        reconstruction_rules: Vec::new(),
        obstructions: Vec::new(),
        resolvers: Vec::new(),
        techniques: Vec::new(),
        microtraces: Vec::new(),
        goals: Vec::new(),
    }
}

fn gz2e01_context() -> (ContentIdentity, RuntimeConfiguration) {
    let parse = |value: &str| value.parse::<Digest>().expect("checked digest literal");
    let content = ContentIdentity {
        schema: CONTENT_IDENTITY_SCHEMA.into(),
        id: "gcn-us-1.0-gz2e01".into(),
        fingerprint: ContentFingerprint {
            platform: GamePlatform::GameCube,
            region: GameRegion::Usa,
            revision: "1.0".into(),
            product_id: "GZ2E01".into(),
            executable_sha256: parse(
                "e7f197436815e66c4a11df3d7bd557d66083b641ff8a8e76439f3caba7ae60e8",
            ),
            game_data_sha256: parse(
                "0bc3bb229279d4b8a8c7cbe962b0bffdfecd35ff21e2d6761ad42e90a070f772",
            ),
            resource_manifest_sha256: parse(
                "2ab36f6c1d9d551c1397e1cf59e13288d2684c973cb7bd0ad6878f5a3b3a2ab1",
            ),
        },
    };
    let runtime = RuntimeConfiguration {
        schema: RUNTIME_CONFIGURATION_SCHEMA.into(),
        content_sha256: GZ2E01_CONTENT_SHA256,
        language: "en".into(),
        settings: BTreeMap::new(),
    };
    (content, runtime)
}

#[derive(Debug)]
pub struct ProjectError(String);

impl ProjectError {
    fn io(error: std::io::Error) -> Self {
        Self(error.to_string())
    }

    fn json(error: serde_json::Error) -> Self {
        Self(error.to_string())
    }
}

impl From<PlannerContractError> for ProjectError {
    fn from(error: PlannerContractError) -> Self {
        Self(error.to_string())
    }
}

impl fmt::Display for ProjectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for ProjectError {}

fn project_error(message: impl Into<String>) -> ProjectError {
    ProjectError(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_root(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "dusklight-route-project-{label}-{}-{nonce}",
            std::process::id()
        ))
    }

    #[test]
    fn builtins_are_valid_read_only_planner_artifacts() {
        let root = temporary_root("builtins");
        let store = ProjectStore::open(&root).unwrap();
        let list = store.list().unwrap();
        assert_eq!(list.projects.len(), 2);
        assert!(list.projects.iter().all(|project| project.read_only));
        assert!(
            list.projects
                .iter()
                .any(|project| project.id == "demo-fanadi-return-place")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn save_is_atomic_revision_checked_and_path_confined() {
        let root = temporary_root("save");
        let store = ProjectStore::open(&root).unwrap();
        let mut project = PlannerWebProject::blank("my-route", "My route").unwrap();
        let created = store
            .save(
                "my-route",
                ProjectSaveRequest {
                    schema: WEB_PROJECT_SAVE_SCHEMA.into(),
                    expected_revision_sha256: None,
                    project: project.clone(),
                },
            )
            .unwrap();
        assert_eq!(store.load("my-route").unwrap(), created);
        assert!(
            store
                .save(
                    "my-route",
                    ProjectSaveRequest {
                        schema: WEB_PROJECT_SAVE_SCHEMA.into(),
                        expected_revision_sha256: None,
                        project: project.clone(),
                    },
                )
                .unwrap_err()
                .to_string()
                .contains("revision conflict")
        );
        project.label = "Renamed route".into();
        let updated = store
            .save(
                "my-route",
                ProjectSaveRequest {
                    schema: WEB_PROJECT_SAVE_SCHEMA.into(),
                    expected_revision_sha256: Some(created.revision_sha256),
                    project,
                },
            )
            .unwrap();
        assert_ne!(updated.revision_sha256, created.revision_sha256);
        assert!(store.load("../escape").is_err());
        assert!(!root.parent().unwrap().join("escape.json").exists());
        fs::remove_dir_all(root).unwrap();
    }
}
