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
mod tests {
    use super::*;
    use crate::RuntimeEvidenceMode;
    use crate::context_compare::{ContextRelation, compare_semantic_contexts};
    use crate::service::{
        ComponentTransferDestination, PlannerServiceOutcome, PlannerServicePayload,
        PlannerServiceRequest, TheorycraftOverlayEdit, handle_request,
    };
    use dusklight_route_planner::evaluation::EvidencePolicy;
    use dusklight_route_planner::logic::TruthStatus;
    use dusklight_route_planner::route_evidence_coverage::RouteEvidenceCoverageReport;
    use dusklight_route_planner::route_observation::{
        ObservationArtifact, ObservationArtifactKind, PLANNED_EDGE_OBSERVATION_MANIFEST_SCHEMA,
        PlannedEdgeObservation, PlannedEdgeObservationManifest, RouteObservationMatchReport,
    };
    use dusklight_route_planner::route_observation_validation::{
        ComponentDisposition, RouteObservationValidationReport, VerificationStatus,
    };
    use dusklight_route_planner::route_suite_coverage::{RouteSuiteCoverageReport, RouteSuiteKind};
    use dusklight_route_planner::state::{
        ComponentBinding, ComponentPayload, ExecutionContext, RuntimeFileOrigin, SceneLocation,
        StateValue,
    };
    use dusklight_route_planner::witness_promotion::{
        RequestedActionPromotion, RequestedWitness, WITNESS_PROMOTION_REQUEST_SCHEMA,
        WitnessPromotionPackMetadata, WitnessPromotionRequest, promote_witnessed_actions,
    };
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
        assert_eq!(list.projects.len(), 6);
        assert!(list.projects.iter().all(|project| project.read_only));
        assert!(
            list.projects
                .iter()
                .any(|project| project.id == "demo-fanadi-return-place")
        );
        let fanadi = store.load("demo-fanadi-return-place").unwrap();
        assert!(fanadi.project.start_state.is_some());
        assert_eq!(fanadi.project.catalog.mechanics.goals.len(), 1);
        let immutable_error = store
            .save(
                "demo-fanadi-return-place",
                ProjectSaveRequest {
                    schema: WEB_PROJECT_SAVE_SCHEMA.into(),
                    expected_revision_sha256: Some(fanadi.revision_sha256),
                    project: fanadi.project.clone(),
                },
            )
            .unwrap_err();
        assert!(immutable_error.to_string().contains("read-only"));
        let opening = store.load("demo-opening-flow").unwrap();
        assert!(opening.project.start_state.is_some());
        assert_eq!(opening.project.catalog.mechanics.goals.len(), 2);
        assert_eq!(
            opening
                .project
                .catalog
                .mechanics
                .goals
                .iter()
                .map(|goal| goal.id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "goal.enter-opening-process",
                dusklight_route_planner::title_boundary::GZ2E01_UNSAVED_FILE_ZERO_GOAL_ID,
            ]
        );
        let keyed_door = store.load("demo-forest-keyed-door").unwrap();
        assert!(keyed_door.project.start_state.is_some());
        assert_eq!(keyed_door.project.catalog.mechanics.transitions.len(), 9);
        assert_eq!(keyed_door.project.catalog.mechanics.goals.len(), 1);
        let rebind = store.load("demo-hypothetical-local-bank-rebind").unwrap();
        assert_eq!(rebind.project.evidence_mode, RuntimeEvidenceMode::Research);
        assert_eq!(rebind.project.catalog.mechanics.transitions.len(), 2);
        let auru = store.load("demo-auru-recent-item-transfer").unwrap();
        assert_eq!(auru.project.evidence_mode, RuntimeEvidenceMode::Research);
        assert_eq!(auru.project.catalog.mechanics.transitions.len(), 4);
        let text_displacement = store.load("demo-text-displacement-goron-mines").unwrap();
        assert_eq!(
            text_displacement
                .project
                .catalog
                .mechanics
                .transitions
                .len(),
            8
        );
        assert_eq!(text_displacement.project.catalog.mechanics.readers.len(), 4);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn semantic_context_comparison_never_falls_back_to_a_nearby_language() {
        let root = temporary_root("semantic-context-comparison");
        let store = ProjectStore::open(&root).unwrap();
        let project = store
            .load("demo-text-displacement-goron-mines")
            .unwrap()
            .project;
        let left = project.start_state.clone().unwrap().into_state().unwrap();
        let mut right_document = project.start_state.unwrap();
        right_document
            .snapshot
            .environment
            .runtime_configuration
            .language = "fr".into();
        let right = right_document.into_state().unwrap();
        let report = compare_semantic_contexts(
            &left,
            &project.catalog,
            &[],
            &right,
            &project.catalog,
            &[],
            RuntimeEvidenceMode::EstablishedOnly,
        )
        .unwrap();

        assert_eq!(
            report.relation,
            ContextRelation::SameContentDifferentRuntime
        );
        assert!(!report.fallback_used);
        assert_eq!(report.left.runtime_configuration.language, "en");
        assert_eq!(report.right.runtime_configuration.language, "fr");
        assert!(report.summary.left_inapplicable_fact_ids.is_empty());
        assert_eq!(
            report.summary.right_inapplicable_fact_ids.len(),
            project.catalog.facts.aliases.len() + project.catalog.facts.derived_facts.len()
        );
        assert!(report.mechanics.iter().all(|row| {
            row.comparison == crate::context_compare::MechanicsComparisonKind::Equivalent
        }));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn text_displacement_demo_replays_each_raw_consumer_in_order() {
        let root = temporary_root("text-displacement");
        let store = ProjectStore::open(&root).unwrap();
        let project = store
            .load("demo-text-displacement-goron-mines")
            .unwrap()
            .project;
        let start = project.start_state.unwrap();
        for producer in [
            "transition.td-producer-auru",
            "transition.td-producer-coro",
            "transition.td-producer-ooccoo",
            "transition.td-producer-yeta",
        ] {
            let response = handle_request(PlannerServiceRequest::EvaluateTransition {
                request_id: format!("request.evaluate-{producer}"),
                state: Box::new(start.clone()),
                catalog: Box::new(project.catalog.clone()),
                equivalence_sets: Vec::new(),
                transition_id: producer.into(),
                evidence_mode: project.evidence_mode,
            });
            let PlannerServiceOutcome::Ok { payload } = response.outcome else {
                panic!("{producer} should be independently executable");
            };
            let PlannerServicePayload::TransitionEvaluation { assessment, .. } = *payload else {
                panic!("producer evaluation should return a typed assessment");
            };
            assert_eq!(
                assessment.classification,
                dusklight_route_planner::evaluation::TransitionClassification::Executable
            );
        }
        let mut route_book = None;
        let mut final_state = None;
        for transition_id in [
            "transition.td-producer-coro",
            "transition.enter-r-sp110-with-displaced-bit",
            "transition.gor-coron-flow6-b-to-c",
            "transition.gor-coron-flow9-prime-a",
            "transition.gor-coron-flow9-write-m029",
        ] {
            let response = handle_request(PlannerServiceRequest::AppendTransition {
                request_id: format!("request.{transition_id}"),
                state: Box::new(start.clone()),
                catalog: Box::new(project.catalog.clone()),
                equivalence_sets: Vec::new(),
                route_book,
                route_book_id: "route.text-displacement-demo".into(),
                route_book_label: "Text Displacement demo route".into(),
                transition_id: transition_id.into(),
                evidence_mode: project.evidence_mode,
            });
            let PlannerServiceOutcome::Ok { payload } = response.outcome else {
                panic!("{transition_id} should append after replaying its raw-bit prefix");
            };
            let PlannerServicePayload::AppendedTransition { after, book, .. } = *payload else {
                panic!("Text Displacement demo should append an ordinary transition");
            };
            final_state = Some(after);
            route_book = Some(book);
        }
        let final_state = final_state.unwrap();
        let persistent = final_state
            .snapshot
            .environment
            .components
            .iter()
            .find(|component| component.id == "persistent.event-flags")
            .unwrap();
        let ComponentPayload::Raw { bytes, .. } = &persistent.payload else {
            panic!("persistent events should remain raw byte-backed state");
        };
        assert_ne!(bytes[7] & 0x04, 0);
        let response = handle_request(PlannerServiceRequest::RemoveAuthoredStep {
            request_id: "request.remove-text-displacement-producer".into(),
            state: Box::new(start),
            catalog: Box::new(project.catalog),
            equivalence_sets: Vec::new(),
            route_book: route_book.unwrap(),
            step_id: "step.route-0000".into(),
            evidence_mode: project.evidence_mode,
        });
        let PlannerServiceOutcome::Ok { payload } = response.outcome else {
            panic!("removing the producer should preserve the rejected edit witness");
        };
        let PlannerServicePayload::RejectedRouteEdit { failed_step_id, .. } = *payload else {
            panic!("the hall entry must require an actual displaced bit producer");
        };
        assert_eq!(failed_step_id, "step.route-0001");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn auru_demo_preserves_session_item_across_runtime_file_lifetimes() {
        let root = temporary_root("auru-recent-item");
        let store = ProjectStore::open(&root).unwrap();
        let project = store
            .load("demo-auru-recent-item-transfer")
            .unwrap()
            .project;
        let start = project.start_state.unwrap();
        let mut route_book = None;
        let mut final_state = None;
        for transition_id in [
            "transition.auru-demo-01-present-fishing-rod",
            "transition.auru-demo-02-begin-file-b",
            "transition.auru-demo-03-hypothetical-gcn-geometry",
            "transition.auru-demo-04-generic-get-item",
        ] {
            let response = handle_request(PlannerServiceRequest::AppendTransition {
                request_id: format!("request.{transition_id}"),
                state: Box::new(start.clone()),
                catalog: Box::new(project.catalog.clone()),
                equivalence_sets: project.equivalence_sets.clone(),
                route_book,
                route_book_id: "route.auru-recent-item-demo".into(),
                route_book_label: "Auru recent-item demo route".into(),
                transition_id: transition_id.into(),
                evidence_mode: project.evidence_mode,
            });
            let PlannerServiceOutcome::Ok { payload } = response.outcome else {
                panic!("{transition_id} should append after replaying its prefix");
            };
            let PlannerServicePayload::AppendedTransition { after, book, .. } = *payload else {
                panic!("Auru demo should return an appended transition");
            };
            final_state = Some(after);
            route_book = Some(book);
        }
        let final_state = final_state.unwrap();
        assert_ne!(
            final_state.snapshot.environment.active_runtime_file.id,
            "file-a"
        );
        let recent_item = final_state
            .snapshot
            .environment
            .components
            .iter()
            .find(|component| component.id == "event.recent-item")
            .unwrap();
        assert!(matches!(
            recent_item.binding,
            ComponentBinding::Session { .. }
        ));
        let inventory = final_state
            .snapshot
            .environment
            .components
            .iter()
            .find(|component| component.id == "inventory.active")
            .unwrap();
        let ComponentPayload::Structured { fields } = &inventory.payload else {
            panic!("inventory should remain structured");
        };
        let StateValue::Bytes(items) = &fields["owned_item_ids"] else {
            panic!("owned item set should remain byte-backed");
        };
        assert_ne!(items[0x4a / 8] & (1 << (0x4a % 8)), 0);
        let response = handle_request(PlannerServiceRequest::RemoveAuthoredStep {
            request_id: "request.remove-auru-producer".into(),
            state: Box::new(start),
            catalog: Box::new(project.catalog),
            equivalence_sets: project.equivalence_sets,
            route_book: route_book.unwrap(),
            step_id: "step.route-0000".into(),
            evidence_mode: project.evidence_mode,
        });
        let PlannerServiceOutcome::Ok { payload } = response.outcome else {
            panic!("removing the recent-item producer should return a typed rejection");
        };
        let PlannerServicePayload::RejectedRouteEdit { failed_step_id, .. } = *payload else {
            panic!("file B must not inherit a Fishing Rod that was never presented");
        };
        assert_eq!(failed_step_id, "step.route-0001");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn hypothetical_rebind_demo_changes_binding_without_changing_payload() {
        let root = temporary_root("hypothetical-rebind");
        let store = ProjectStore::open(&root).unwrap();
        let project = store
            .load("demo-hypothetical-local-bank-rebind")
            .unwrap()
            .project;
        let start = project.start_state.unwrap();
        let append = |route_book, transition_id: &str| {
            handle_request(PlannerServiceRequest::AppendTransition {
                request_id: format!("request.{transition_id}"),
                state: Box::new(start.clone()),
                catalog: Box::new(project.catalog.clone()),
                equivalence_sets: project.equivalence_sets.clone(),
                route_book,
                route_book_id: "route.hypothetical-rebind-demo".into(),
                route_book_label: "Hypothetical rebind demo route".into(),
                transition_id: transition_id.into(),
                evidence_mode: project.evidence_mode,
            })
        };
        let response = append(None, "transition.hypothetical-local-bank-rebind");
        let PlannerServiceOutcome::Ok { payload } = response.outcome else {
            panic!("research mode should admit the explicit hypothetical rebind");
        };
        let PlannerServicePayload::AppendedTransition { after, book, .. } = *payload else {
            panic!("rebind should append as an ordinary typed transition step");
        };
        let first_after_snapshot = after.snapshot.clone();
        let before = start.clone().into_state().unwrap();
        let after_state = after.clone().into_state().unwrap();
        let before_bank = before
            .snapshot
            .environment
            .components
            .iter()
            .find(|component| component.id == "stage.local-bank")
            .unwrap();
        let after_bank = after_state
            .snapshot
            .environment
            .components
            .iter()
            .find(|component| component.id == "stage.local-bank")
            .unwrap();
        assert_eq!(before_bank.payload, after_bank.payload);
        assert_ne!(before_bank.binding, after_bank.binding);
        assert_eq!(
            after_bank.binding,
            ComponentBinding::Stage {
                stage: "D_MN06".into()
            }
        );
        let response = append(Some(book), "transition.enter-temple-path");
        let PlannerServiceOutcome::Ok { payload } = response.outcome else {
            panic!("the rebound alias should authorize the unchanged Temple path");
        };
        let PlannerServicePayload::AppendedTransition { after, book, .. } = *payload else {
            panic!("Temple path should append after replaying the rebind");
        };
        assert_eq!(after.snapshot.environment.location.stage, "STAGE_B");
        let mut weak_facts = project.catalog.facts.clone();
        weak_facts
            .derived_facts
            .iter_mut()
            .find(|fact| fact.id == "path.tot-open")
            .unwrap()
            .evidence
            .truth = TruthStatus::Contested;
        let weak_catalog =
            ComposedPlannerCatalog::compose(&weak_facts, &project.catalog.mechanics, &[]).unwrap();
        let mut second_book = book.clone();
        second_book.manifest.id = "route.hypothetical-rebind-demo-copy".into();
        let coverage =
            RouteEvidenceCoverageReport::build(&weak_catalog, &[*book.clone(), *second_book], 2)
                .unwrap();
        assert_eq!(coverage.weak_high_usage_fact_ids, ["path.tot-open"]);
        assert_eq!(
            coverage
                .facts
                .iter()
                .find(|fact| fact.fact_id == "local.tot-switch")
                .unwrap()
                .route_book_ids
                .len(),
            2
        );
        let suite_coverage = RouteSuiteCoverageReport::build(
            &weak_catalog,
            &[(RouteSuiteKind::Hypothetical, *book.clone())],
        )
        .unwrap();
        assert_eq!(suite_coverage.suites.len(), 4);
        assert!(
            suite_coverage.suites[..3]
                .iter()
                .all(|suite| !suite.reported)
        );
        assert_eq!(
            suite_coverage.suites[3].exercised_fact_ids,
            ["local.forest-switch", "local.tot-switch", "path.tot-open"]
        );
        let trace = ObservationArtifact {
            id: "trace.hypothetical-rebind".into(),
            kind: ObservationArtifactKind::Trace,
            sha256: Digest([0x41; 32]),
        };
        let tape = ObservationArtifact {
            id: "tape.hypothetical-rebind".into(),
            kind: ObservationArtifactKind::Tape,
            sha256: Digest([0x42; 32]),
        };
        let manifest = PlannedEdgeObservationManifest {
            schema: PLANNED_EDGE_OBSERVATION_MANIFEST_SCHEMA.into(),
            artifacts: vec![tape, trace],
            observations: vec![
                PlannedEdgeObservation {
                    id: "observation.rebind".into(),
                    step_id: "step.route-0000".into(),
                    trace_artifact_id: "trace.hypothetical-rebind".into(),
                    tape_artifact_id: Some("tape.hypothetical-rebind".into()),
                    before_snapshot_sha256: before.snapshot.digest().unwrap(),
                    after_snapshot_sha256: first_after_snapshot.digest().unwrap(),
                    start_tick: 10,
                    end_tick: 20,
                    start_tape_frame: Some(9),
                    end_tape_frame: Some(19),
                },
                PlannedEdgeObservation {
                    id: "observation.temple-path".into(),
                    step_id: "step.route-0001".into(),
                    trace_artifact_id: "trace.hypothetical-rebind".into(),
                    tape_artifact_id: Some("tape.hypothetical-rebind".into()),
                    before_snapshot_sha256: first_after_snapshot.digest().unwrap(),
                    after_snapshot_sha256: after.snapshot.digest().unwrap(),
                    start_tick: 21,
                    end_tick: 30,
                    start_tape_frame: Some(20),
                    end_tape_frame: Some(29),
                },
            ],
        };
        let observation_snapshots = vec![
            before.snapshot.clone(),
            first_after_snapshot,
            after.snapshot.clone(),
        ];
        let observation_report = RouteObservationMatchReport::build(
            &weak_catalog,
            &book,
            &manifest,
            &observation_snapshots,
        )
        .unwrap();
        assert!(observation_report.steps.iter().all(|step| step.observed));
        assert_eq!(observation_report.steps[1].observations[0].start_tick, 21);
        let validation = RouteObservationValidationReport::build(
            &weak_catalog,
            &book,
            &observation_report,
            &observation_snapshots,
            &project.equivalence_sets,
            EvidencePolicy::RESEARCH,
        )
        .unwrap();
        assert_eq!(validation.validations.len(), 2);
        assert!(validation.validations.iter().all(|row| {
            row.model_replay_status == VerificationStatus::Verified
                && row.snapshot_effects_status == VerificationStatus::Verified
                && row.component_preservation_status == VerificationStatus::Verified
        }));
        let local_bank = validation.validations[0]
            .component_checks
            .iter()
            .find(|check| check.component_id == "stage.local-bank")
            .unwrap();
        assert_eq!(
            local_bank.modeled_disposition,
            ComponentDisposition::Changed
        );
        assert!(local_bank.matches_model);
        let source_transition = weak_catalog
            .mechanics
            .transitions
            .iter()
            .find(|record| record.id == "transition.hypothetical-local-bank-rebind")
            .unwrap()
            .clone();
        let untouched_transition = weak_catalog
            .mechanics
            .transitions
            .iter()
            .find(|record| record.id == "transition.enter-temple-path")
            .unwrap()
            .clone();
        let promotion_request = WitnessPromotionRequest {
            schema: WITNESS_PROMOTION_REQUEST_SCHEMA.into(),
            pack: WitnessPromotionPackMetadata {
                id: "pack.witnessed-rebind".into(),
                version: "1.0.0".into(),
                author: "Dusklight regression".into(),
                source: "Validated hypothetical rebind observation".into(),
                precedence: 100,
                conflicts: Vec::new(),
            },
            promotions: vec![RequestedActionPromotion {
                action: book.steps[0].action.clone(),
                promotion_rule_id: "rule.promote-rebind".into(),
                replacement_rule_id: "rule.replace-rebind-evidence".into(),
                witnesses: vec![RequestedWitness {
                    observation_id: "observation.rebind".into(),
                    evidence_id: "evidence.witnessed-rebind".into(),
                }],
            }],
        };
        let (promotion_pack, promotion_receipt) =
            promote_witnessed_actions(&weak_catalog, &validation, &promotion_request).unwrap();
        assert_eq!(
            promotion_receipt.action_ids_before,
            promotion_receipt.action_ids_after
        );
        let promoted_catalog = ComposedPlannerCatalog::compose(
            &weak_catalog.facts,
            &weak_catalog.mechanics,
            &[promotion_pack],
        )
        .unwrap();
        let promoted_transition = promoted_catalog
            .mechanics
            .transitions
            .iter()
            .find(|record| record.id == source_transition.id)
            .unwrap();
        assert_eq!(promoted_transition.evidence.truth, TruthStatus::Established);
        assert!(source_transition.evidence.records.iter().all(|prior| {
            promoted_transition
                .evidence
                .records
                .iter()
                .any(|record| record == prior)
        }));
        assert!(promoted_transition.evidence.records.iter().any(|record| {
            record.id == "evidence.witnessed-rebind"
                && record.kind == dusklight_route_planner::logic::EvidenceKind::RouteWitnessed
        }));
        assert_eq!(
            promoted_catalog
                .mechanics
                .transitions
                .iter()
                .find(|record| record.id == untouched_transition.id)
                .unwrap(),
            &untouched_transition
        );
        let response = handle_request(PlannerServiceRequest::RemoveAuthoredStep {
            request_id: "request.remove-hypothetical-rebind".into(),
            state: Box::new(start),
            catalog: Box::new(project.catalog),
            equivalence_sets: project.equivalence_sets,
            route_book: book,
            step_id: "step.route-0000".into(),
            evidence_mode: project.evidence_mode,
        });
        let PlannerServiceOutcome::Ok { payload } = response.outcome else {
            panic!("removing the rebind should return the broken downstream join");
        };
        let PlannerServicePayload::RejectedRouteEdit { failed_step_id, .. } = *payload else {
            panic!("the Temple path must remain causally dependent on the rebind");
        };
        assert_eq!(failed_step_id, "step.route-0001");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn keyed_door_demo_replays_every_audited_actor_phase() {
        let root = temporary_root("keyed-door-propagation");
        let store = ProjectStore::open(&root).unwrap();
        let project = store.load("demo-forest-keyed-door").unwrap().project;
        let start = project.start_state.unwrap();
        let mut route_book = None;
        let mut final_state = None;
        for transition_id in [
            "transition.gz2e01-door1-01-offer-event",
            "transition.gz2e01-door1-02-demo-action8",
            "transition.gz2e01-door1-03-finish-keyhole",
            "transition.gz2e01-door1-04-flush-key-delta",
            "transition.gz2e01-door1-05-open-init",
            "transition.gz2e01-door1-06-open-proc",
            "transition.gz2e01-door1-07-cross-room-adjacency",
            "transition.gz2e01-door1-08-close-init",
            "transition.gz2e01-door1-09-close-end",
        ] {
            let response = handle_request(PlannerServiceRequest::AppendTransition {
                request_id: format!("request.{transition_id}"),
                state: Box::new(start.clone()),
                catalog: Box::new(project.catalog.clone()),
                equivalence_sets: project.equivalence_sets.clone(),
                route_book,
                route_book_id: "route.keyed-door-demo".into(),
                route_book_label: "Forest keyed-door demo route".into(),
                transition_id: transition_id.into(),
                evidence_mode: RuntimeEvidenceMode::EstablishedOnly,
            });
            let PlannerServiceOutcome::Ok { payload } = response.outcome else {
                panic!("{transition_id} should append after replaying its exact prefix");
            };
            let PlannerServicePayload::AppendedTransition { after, book, .. } = *payload else {
                panic!("keyed-door demo should return an appended transition");
            };
            route_book = Some(book);
            final_state = Some(after);
        }
        let final_state = final_state.unwrap();
        assert_eq!(final_state.snapshot.environment.location.stage, "D_MN05");
        assert_eq!(final_state.snapshot.environment.location.room, 2);
        let dungeon = final_state
            .snapshot
            .environment
            .components
            .iter()
            .find(|component| component.id == "dungeon.d-mn05-memory")
            .unwrap();
        let ComponentPayload::Structured { fields } = &dungeon.payload else {
            panic!("dungeon memory should remain structured");
        };
        assert_eq!(fields["small_keys"], StateValue::Unsigned(0));
        assert_eq!(fields["switch_0b"], StateValue::Boolean(true));
        let route_book = route_book.unwrap();
        assert_eq!(route_book.methods[0].step_ids.len(), 9);
        let response = handle_request(PlannerServiceRequest::RemoveAuthoredStep {
            request_id: "request.remove-keyed-action8".into(),
            state: Box::new(start),
            catalog: Box::new(project.catalog),
            equivalence_sets: project.equivalence_sets,
            route_book,
            step_id: "step.route-0001".into(),
            evidence_mode: RuntimeEvidenceMode::EstablishedOnly,
        });
        let PlannerServiceOutcome::Ok { payload } = response.outcome else {
            panic!("removing the unlock producer should return a typed rejection");
        };
        let PlannerServicePayload::RejectedRouteEdit {
            failed_step_id,
            assessment,
            ..
        } = *payload
        else {
            panic!("the keyed-door continuation should reject without action 8");
        };
        assert_eq!(failed_step_id, "step.route-0002");
        assert_eq!(
            assessment.classification,
            dusklight_route_planner::evaluation::TransitionClassification::GuardBlocked
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn fanadi_demo_savewarp_propagates_from_its_exact_start_state() {
        let root = temporary_root("fanadi-propagation");
        let store = ProjectStore::open(&root).unwrap();
        let record = store.load("demo-fanadi-return-place").unwrap();
        let project = record.project;
        let response = handle_request(PlannerServiceRequest::AppendTransition {
            request_id: "request.fanadi-savewarp".into(),
            state: Box::new(project.start_state.unwrap()),
            catalog: Box::new(project.catalog),
            equivalence_sets: project.equivalence_sets,
            route_book: None,
            route_book_id: "route.fanadi-demo".into(),
            route_book_label: "Fanadi demo route".into(),
            transition_id: "transition.savewarp.from-player-return-place".into(),
            evidence_mode: RuntimeEvidenceMode::EstablishedOnly,
        });
        let PlannerServiceOutcome::Ok { payload } = response.outcome else {
            panic!("Fanadi demo savewarp should be executable from its checked start state");
        };
        let PlannerServicePayload::AppendedTransition { after, book, .. } = *payload else {
            panic!("Fanadi demo should append one propagated transition");
        };
        assert_eq!(after.snapshot.environment.location.stage, "R_SP107");
        assert_eq!(after.snapshot.environment.location.room, 3);
        assert_eq!(after.snapshot.environment.location.layer, -1);
        assert_eq!(after.snapshot.environment.location.spawn, 1);
        assert_eq!(book.methods[0].step_ids, ["step.route-0000"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn opening_demo_reset_propagates_into_the_pending_opening_process() {
        let root = temporary_root("opening-propagation");
        let store = ProjectStore::open(&root).unwrap();
        let record = store.load("demo-opening-flow").unwrap();
        let project = record.project;
        let response = handle_request(PlannerServiceRequest::AppendTransition {
            request_id: "request.opening-reset".into(),
            state: Box::new(project.start_state.unwrap()),
            catalog: Box::new(project.catalog),
            equivalence_sets: project.equivalence_sets,
            route_book: None,
            route_book_id: "route.opening-demo".into(),
            route_book_label: "Opening demo route".into(),
            transition_id: "transition.gz2e01.reset-to-opening".into(),
            evidence_mode: RuntimeEvidenceMode::EstablishedOnly,
        });
        let PlannerServiceOutcome::Ok { payload } = response.outcome else {
            panic!("opening reset should be executable from its checked start state");
        };
        let PlannerServicePayload::AppendedTransition { after, book, .. } = *payload else {
            panic!("opening demo should append one propagated transition");
        };
        assert_eq!(
            after.snapshot.environment.execution_context,
            ExecutionContext::Process {
                process_name: "PROC_OPENING_SCENE".into(),
                pending_world_load: Some(SceneLocation {
                    stage: "F_SP102".into(),
                    room: 0,
                    layer: 10,
                    spawn: 100,
                }),
            }
        );
        let restart = after
            .snapshot
            .environment
            .components
            .iter()
            .find(|component| component.id == "restart")
            .unwrap();
        let ComponentPayload::Structured { fields } = &restart.payload else {
            panic!("opening restart component should remain structured");
        };
        assert_eq!(fields["room_param"], StateValue::Unsigned(0));
        assert_eq!(book.methods[0].step_ids, ["step.route-0000"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn opening_demo_replays_through_blank_slot_selection() {
        let root = temporary_root("opening-file-select-propagation");
        let store = ProjectStore::open(&root).unwrap();
        let project = store.load("demo-opening-flow").unwrap().project;
        let start = project.start_state.unwrap();
        let mut route_book = None;
        let mut final_state = None;
        for transition_id in [
            "transition.gz2e01.reset-to-opening",
            "transition.gz2e01.observe-opening-phase-4",
            "transition.gz2e01.opening-enter-and-initialize-file0",
            "transition.gz2e01.title-key-accept",
            "transition.gz2e01.title-request-name-scene",
            "transition.gz2e01.observe-name-scene-create",
            "transition.gz2e01.name-scene-file-select-initialize",
            "transition.gz2e01.file-select-focus-blank-slot-1",
            "transition.gz2e01.file-select-blank-slot-1",
        ] {
            let response = handle_request(PlannerServiceRequest::AppendTransition {
                request_id: format!("request.{transition_id}"),
                state: Box::new(start.clone()),
                catalog: Box::new(project.catalog.clone()),
                equivalence_sets: project.equivalence_sets.clone(),
                route_book,
                route_book_id: "route.opening-file-select-demo".into(),
                route_book_label: "Opening file-select demo route".into(),
                transition_id: transition_id.into(),
                evidence_mode: project.evidence_mode,
            });
            let PlannerServiceOutcome::Ok { payload } = response.outcome else {
                panic!("{transition_id} should append after replaying its exact prefix");
            };
            let PlannerServicePayload::AppendedTransition { after, book, .. } = *payload else {
                panic!("opening demo should return an appended transition");
            };
            final_state = Some(after);
            route_book = Some(book);
        }
        let final_state = final_state.unwrap();
        assert_eq!(
            final_state.snapshot.environment.execution_context,
            ExecutionContext::Process {
                process_name: "PROC_NAME_SCENE".into(),
                pending_world_load: None,
            }
        );
        assert_eq!(
            final_state.snapshot.environment.active_runtime_file.origin,
            RuntimeFileOrigin::TitleFile0
        );
        let fields = |component_id: &str| {
            let component = final_state
                .snapshot
                .environment
                .components
                .iter()
                .find(|component| component.id == component_id)
                .unwrap();
            let ComponentPayload::Structured { fields } = &component.payload else {
                panic!("{component_id} should remain structured");
            };
            fields
        };
        assert_eq!(
            fields("runtime-file.header")["new_file_raw"],
            StateValue::Unsigned(128)
        );
        assert_eq!(
            fields("runtime-file.header")["data_num_raw"],
            StateValue::Unsigned(0)
        );
        assert_eq!(
            fields("name-scene-control")["phase"],
            StateValue::Text("name_entry".into())
        );
        assert_eq!(route_book.unwrap().methods[0].step_ids.len(), 9);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn presentation_regions_group_nodes_without_changing_the_planner_graph() {
        let root = temporary_root("presentation-region");
        let store = ProjectStore::open(&root).unwrap();
        let mut project = store.load("demo-forest-keyed-door").unwrap().project;
        let graph = PlannerGraph::project_composed(&project.catalog).unwrap();
        let transition_node = graph
            .nodes
            .iter()
            .find(|node| {
                matches!(
                    &node.payload,
                    dusklight_route_planner::graph::PlannerNodePayload::Transition {
                        transition_id,
                        ..
                    } if transition_id == "transition.gz2e01-door1-09-close-end"
                )
            })
            .unwrap()
            .id
            .clone();
        let graph_sha256 = graph.digest().unwrap();
        project.presentation.regions.push(PresentationRegion {
            id: "region.presentation-shutter-close".into(),
            label: "Shutter close".into(),
            parent_region_id: None,
            version: 1,
            snapshot_node_ids: Vec::new(),
            derivation: None,
        });
        project
            .presentation
            .node_region_ids
            .insert(transition_node, "region.presentation-shutter-close".into());
        project.presentation.regions.push(PresentationRegion {
            id: "region.presentation-shutter-close-reference".into(),
            label: "Shutter close reference".into(),
            parent_region_id: None,
            version: 1,
            snapshot_node_ids: Vec::new(),
            derivation: Some(PresentationRegionDerivation {
                kind: PresentationRegionDerivationKind::Reference,
                source_region_id: "region.presentation-shutter-close".into(),
                source_version: 1,
            }),
        });
        project.validate().unwrap();
        let decoded: PlannerWebProject =
            serde_json::from_slice(&project.canonical_bytes().unwrap()).unwrap();
        assert_eq!(decoded.presentation, project.presentation);
        assert_eq!(
            PlannerGraph::project_composed(&decoded.catalog)
                .unwrap()
                .digest()
                .unwrap(),
            graph_sha256
        );
        let mut legacy = decoded.clone();
        legacy.schema = LEGACY_WEB_PROJECT_SCHEMAS[1].into();
        legacy.id = "legacy-presentation-region".into();
        fs::write(
            root.join("legacy-presentation-region.json"),
            serde_json::to_vec_pretty(&legacy).unwrap(),
        )
        .unwrap();
        assert_eq!(
            store
                .load("legacy-presentation-region")
                .unwrap()
                .project
                .schema,
            WEB_PROJECT_SCHEMA
        );

        project.presentation.regions[0].parent_region_id =
            Some("region.presentation-shutter-close".into());
        assert!(
            project
                .validate()
                .unwrap_err()
                .to_string()
                .contains("cycle")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn theorycraft_overlay_base_and_pack_survive_save_and_reload() {
        let root = temporary_root("theorycraft-save");
        let store = ProjectStore::open(&root).unwrap();
        let mut project = builtin_projects()
            .unwrap()
            .into_iter()
            .find(|project| project.id == "demo-hypothetical-local-bank-rebind")
            .unwrap();
        project.id = "theorycraft-save".into();
        project.label = "Theorycraft save".into();
        let base = project.catalog.clone();
        let state = project.start_state.clone().unwrap();
        let source = state.snapshot.environment.components[0].id.clone();
        let response = handle_request(PlannerServiceRequest::EditTheorycraftOverlays {
            request_id: "project.theorycraft-save".into(),
            base_catalog: Box::new(base.clone()),
            overlays: Vec::new(),
            state: Box::new(state),
            route_book: None,
            edit: TheorycraftOverlayEdit::AddComponentTransfer {
                pack_id: "what-if.saved-rebind".into(),
                label: "Saved exact-context rebind".into(),
                source_component_id: source,
                destination: ComponentTransferDestination::Rebind {
                    binding: ComponentBinding::Stage {
                        stage: "D_MN06".into(),
                    },
                },
            },
        });
        let PlannerServiceOutcome::Ok { payload } = response.outcome else {
            panic!("theorycraft overlay should compose");
        };
        let PlannerServicePayload::TheorycraftOverlaysEdited {
            base_catalog,
            overlays,
            catalog,
            ..
        } = *payload
        else {
            panic!("theorycraft edit should return persisted ingredients");
        };
        project.catalog = *catalog;
        project.theorycraft_base_catalog = Some(base_catalog);
        project.theorycraft_overlays = overlays;
        project.validate().unwrap();
        let created = store
            .save(
                &project.id.clone(),
                ProjectSaveRequest {
                    schema: WEB_PROJECT_SAVE_SCHEMA.into(),
                    expected_revision_sha256: None,
                    project,
                },
            )
            .unwrap();
        let reloaded = store.load("theorycraft-save").unwrap().project;
        assert_eq!(reloaded, created.project);
        assert_eq!(reloaded.theorycraft_overlays.len(), 1);
        assert_eq!(
            reloaded.theorycraft_base_catalog.as_ref().unwrap().as_ref(),
            &base
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
