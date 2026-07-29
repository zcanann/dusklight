//! Planner-owned web project artifacts and conflict-safe workspace storage.

use dusklight_route_planner::PlannerContractError;
use dusklight_route_planner::artifact::Digest;
use dusklight_route_planner::execution::PlannerExecutionStateDocument;
use dusklight_route_planner::graph::PlannerGraph;
use dusklight_route_planner::identity::EquivalenceSet;
use dusklight_route_planner::logic::{FACT_CATALOG_SCHEMA, FactCatalog};
use dusklight_route_planner::refinement::{ComposedPlannerCatalog, RefinementPack};
use dusklight_route_planner::route_book::RouteBook;
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

pub const WEB_PROJECT_SCHEMA: &str = "dusklight.route-planner.web-project/v3";
const LEGACY_WEB_PROJECT_SCHEMAS: &[&str] = &[
    "dusklight.route-planner.web-project/v1",
    "dusklight.route-planner.web-project/v2",
];
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
    pub theorycraft_base_catalog: Option<Box<ComposedPlannerCatalog>>,
    #[serde(default)]
    pub theorycraft_overlays: Vec<RefinementPack>,
    #[serde(default = "established_evidence_mode")]
    pub evidence_mode: crate::RuntimeEvidenceMode,
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
    #[serde(default)]
    pub regions: Vec<PresentationRegion>,
    #[serde(default)]
    pub node_region_ids: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PresentationRegion {
    pub id: String,
    pub label: String,
    pub parent_region_id: Option<String>,
    #[serde(default = "initial_presentation_region_version")]
    pub version: u32,
    #[serde(default)]
    pub snapshot_node_ids: Vec<String>,
    #[serde(default)]
    pub derivation: Option<PresentationRegionDerivation>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PresentationRegionDerivation {
    pub kind: PresentationRegionDerivationKind,
    pub source_region_id: String,
    pub source_version: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PresentationRegionDerivationKind {
    Copy,
    Fork,
    Reference,
    Version,
    Replacement,
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

fn established_evidence_mode() -> crate::RuntimeEvidenceMode {
    crate::RuntimeEvidenceMode::EstablishedOnly
}

fn initial_presentation_region_version() -> u32 {
    1
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
            theorycraft_base_catalog: None,
            theorycraft_overlays: Vec::new(),
            evidence_mode: crate::RuntimeEvidenceMode::EstablishedOnly,
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
        match (
            &self.theorycraft_base_catalog,
            self.theorycraft_overlays.is_empty(),
        ) {
            (None, true) => {}
            (None, false) => {
                return Err(project_error(
                    "theorycraft overlays require their immutable base catalog",
                ));
            }
            (Some(_), true) => {
                return Err(project_error(
                    "theorycraft base catalog must be absent when no overlays are active",
                ));
            }
            (Some(base), false) => {
                let recomposed = base.extend_ephemeral_what_if(&self.theorycraft_overlays)?;
                if recomposed != self.catalog {
                    return Err(project_error(
                        "catalog does not match its persisted theorycraft base and overlays",
                    ));
                }
            }
        }
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
        let graph_region_ids = graph
            .regions
            .iter()
            .map(|region| region.id.as_str())
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
        let mut presentation_region_ids = BTreeSet::new();
        for region in &self.presentation.regions {
            validate_project_id(&region.id)?;
            validate_label(&region.label)?;
            if region.version == 0 {
                return Err(project_error(format!(
                    "presentation region {} has version zero",
                    region.id
                )));
            }
            if region
                .snapshot_node_ids
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
            {
                return Err(project_error(format!(
                    "presentation region {} snapshot nodes must be unique and sorted",
                    region.id
                )));
            }
            if graph_region_ids.contains(region.id.as_str())
                || !presentation_region_ids.insert(region.id.as_str())
            {
                return Err(project_error(format!(
                    "presentation region {} is duplicated or collides with a planner region",
                    region.id
                )));
            }
        }
        for region in &self.presentation.regions {
            if let Some(parent) = &region.parent_region_id
                && !graph_region_ids.contains(parent.as_str())
                && !presentation_region_ids.contains(parent.as_str())
            {
                return Err(project_error(format!(
                    "presentation region {} references unknown parent {parent}",
                    region.id
                )));
            }
            let mut ancestor = region.parent_region_id.as_deref();
            let mut visited = BTreeSet::from([region.id.as_str()]);
            while let Some(parent) = ancestor {
                if !visited.insert(parent) {
                    return Err(project_error(format!(
                        "presentation region {} has a parent cycle",
                        region.id
                    )));
                }
                ancestor = self
                    .presentation
                    .regions
                    .iter()
                    .find(|candidate| candidate.id == parent)
                    .and_then(|candidate| candidate.parent_region_id.as_deref());
            }
            if let Some(derivation) = &region.derivation {
                let source = self
                    .presentation
                    .regions
                    .iter()
                    .find(|candidate| candidate.id == derivation.source_region_id)
                    .ok_or_else(|| {
                        project_error(format!(
                            "presentation region {} references unknown derivation source {}",
                            region.id, derivation.source_region_id
                        ))
                    })?;
                if derivation.source_region_id == region.id
                    || derivation.source_version == 0
                    || derivation.source_version > source.version
                    || (derivation.kind == PresentationRegionDerivationKind::Reference
                        && !region.snapshot_node_ids.is_empty())
                {
                    return Err(project_error(format!(
                        "presentation region {} has invalid derivation provenance",
                        region.id
                    )));
                }
                let mut source_id = Some(derivation.source_region_id.as_str());
                let mut sources = BTreeSet::from([region.id.as_str()]);
                while let Some(candidate_id) = source_id {
                    if !sources.insert(candidate_id) {
                        return Err(project_error(format!(
                            "presentation region {} has a derivation cycle",
                            region.id
                        )));
                    }
                    source_id = self
                        .presentation
                        .regions
                        .iter()
                        .find(|candidate| candidate.id == candidate_id)
                        .and_then(|candidate| candidate.derivation.as_ref())
                        .map(|candidate| candidate.source_region_id.as_str());
                }
            }
        }
        let route_step_ids = self
            .route_book
            .iter()
            .flat_map(|book| &book.steps)
            .map(|step| step.id.as_str())
            .collect::<BTreeSet<_>>();
        let valid_node_id = |node_id: &str| {
            node_ids.contains(node_id)
                || node_id == "execution-state/start"
                || node_id
                    .strip_prefix("execution-state/after/")
                    .is_some_and(|step_id| route_step_ids.contains(step_id))
        };
        for region in &self.presentation.regions {
            for node_id in &region.snapshot_node_ids {
                if !valid_node_id(node_id) {
                    return Err(project_error(format!(
                        "presentation region {} snapshot references unknown node {node_id}",
                        region.id
                    )));
                }
            }
        }
        for (node_id, region_id) in &self.presentation.node_region_ids {
            if !valid_node_id(node_id) {
                return Err(project_error(format!(
                    "presentation region assignment references unknown node {node_id}"
                )));
            }
            if !presentation_region_ids.contains(region_id.as_str()) {
                return Err(project_error(format!(
                    "presentation region assignment references unknown region {region_id}"
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
    let mut project: PlannerWebProject =
        serde_json::from_slice(&bytes).map_err(ProjectError::json)?;
    if LEGACY_WEB_PROJECT_SCHEMAS.contains(&project.schema.as_str()) {
        project.schema = WEB_PROJECT_SCHEMA.into();
    }
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

const BUILTIN_PROJECT_ASSETS: &[(&str, &[u8])] = &[
    (
        "demo-auru-recent-item-transfer",
        include_bytes!("../libraries/demo-auru-recent-item-transfer.json"),
    ),
    (
        "demo-fanadi-return-place",
        include_bytes!("../libraries/demo-fanadi-return-place.json"),
    ),
    (
        "demo-forest-keyed-door",
        include_bytes!("../libraries/demo-forest-keyed-door.json"),
    ),
    (
        "demo-hypothetical-local-bank-rebind",
        include_bytes!("../libraries/demo-hypothetical-local-bank-rebind.json"),
    ),
    (
        "demo-opening-flow",
        include_bytes!("../libraries/demo-opening-flow.json"),
    ),
    (
        "demo-text-displacement-goron-mines",
        include_bytes!("../libraries/demo-text-displacement-goron-mines.json"),
    ),
];

fn builtin_projects() -> Result<Vec<PlannerWebProject>, ProjectError> {
    let mut projects = Vec::with_capacity(BUILTIN_PROJECT_ASSETS.len());
    let mut identities = BTreeSet::new();
    for (expected_id, bytes) in BUILTIN_PROJECT_ASSETS {
        let project: PlannerWebProject =
            serde_json::from_slice(bytes).map_err(ProjectError::json)?;
        project.validate()?;
        if project.id != *expected_id {
            return Err(project_error(format!(
                "serialized Library asset {expected_id} contains project identity {}",
                project.id
            )));
        }
        if !identities.insert(project.id.clone()) {
            return Err(project_error(format!(
                "serialized Library identity {} is duplicated",
                project.id
            )));
        }
        projects.push(project);
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
#[path = "project_tests.rs"]
mod tests;
