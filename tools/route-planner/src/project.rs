//! Planner-owned web project artifacts and conflict-safe workspace storage.

use dusklight_route_planner::PlannerContractError;
use dusklight_route_planner::artifact::Digest;
use dusklight_route_planner::execution::{PlannerExecutionState, PlannerExecutionStateDocument};
use dusklight_route_planner::graph::PlannerGraph;
use dusklight_route_planner::identity::{
    CONTENT_IDENTITY_SCHEMA, ContentFingerprint, ContentIdentity, EquivalenceSet, GamePlatform,
    GameRegion, RUNTIME_CONFIGURATION_SCHEMA, RuntimeConfiguration,
};
use dusklight_route_planner::logic::{
    ComparisonOperator, FACT_CATALOG_SCHEMA, FactCatalog, PredicateExpression, ValueReference,
};
use dusklight_route_planner::refinement::ComposedPlannerCatalog;
use dusklight_route_planner::return_place::{
    GZ2E01_CONTENT_SHA256, gz2e01_tower_return_place_mechanics,
};
use dusklight_route_planner::route_book::RouteBook;
use dusklight_route_planner::snapshot::{STATE_SNAPSHOT_SCHEMA, StateSnapshot};
use dusklight_route_planner::state::{
    BackingAttachment, ComponentBinding, ComponentKind, ComponentPayload, ComponentProvenance,
    EXECUTION_ENVIRONMENT_SCHEMA, ExecutionContext, ExecutionEnvironment, PhysicalSlotId,
    PlayerForm, PlayerState, ProvenanceSourceKind, RuntimeFile, RuntimeFileLifecycle,
    RuntimeFileOrigin, SceneLocation, SemanticLifetime, SerializationOwner, StateComponent,
    StateValue,
};
use dusklight_route_planner::title_boundary::gz2e01_reset_to_opening_mechanics;
use dusklight_route_planner::transition::{Goal, MECHANICS_CATALOG_SCHEMA, MechanicsCatalog};
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
    let mut fanadi_mechanics = gz2e01_tower_return_place_mechanics(&content, &runtime)?;
    fanadi_mechanics.goals.push(Goal {
        id: "goal.reach-fanadi-return-place".into(),
        label: "Reach the stored Fanadi return place".into(),
        predicate: PredicateExpression::All {
            terms: vec![
                PredicateExpression::Compare {
                    left: ValueReference::LocationStage,
                    operator: ComparisonOperator::Equal,
                    right: ValueReference::Literal {
                        value: StateValue::Text("R_SP107".into()),
                    },
                },
                PredicateExpression::Compare {
                    left: ValueReference::LocationRoom,
                    operator: ComparisonOperator::Equal,
                    right: ValueReference::Literal {
                        value: StateValue::Signed(3),
                    },
                },
            ],
        },
    });
    let fanadi = ComposedPlannerCatalog::compose(&facts, &fanadi_mechanics, &[])?;
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
            start_state: Some(fanadi_start_state(runtime.clone())?),
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

fn fanadi_start_state(
    runtime_configuration: RuntimeConfiguration,
) -> Result<PlannerExecutionStateDocument, ProjectError> {
    let runtime_file_id = "file-0".to_owned();
    let return_place = StateComponent {
        id: "return-place".into(),
        component_kind: ComponentKind::Custom {
            id: "return-place".into(),
        },
        payload: ComponentPayload::Structured {
            fields: BTreeMap::from([
                ("player_status".into(), StateValue::Unsigned(1)),
                ("room".into(), StateValue::Signed(3)),
                ("stage".into(), StateValue::Text("R_SP107".into())),
            ]),
        },
        binding: ComponentBinding::RuntimeFile {
            runtime_file_id: runtime_file_id.clone(),
        },
        lifetime: SemanticLifetime::RuntimeFile,
        serialization_owner: SerializationOwner::RuntimeFile {
            runtime_file_id: runtime_file_id.clone(),
        },
        provenance: vec![ComponentProvenance {
            source_kind: ProvenanceSourceKind::ExtractedFact,
            source_id: "demo.fanadi-return-place".into(),
            source_sha256: None,
            transition_id: None,
        }],
    };
    let snapshot = StateSnapshot {
        schema: STATE_SNAPSHOT_SCHEMA.into(),
        id: "snapshot.fanadi-before-savewarp".into(),
        sequence: 0,
        environment: ExecutionEnvironment {
            schema: EXECUTION_ENVIRONMENT_SCHEMA.into(),
            runtime_configuration,
            active_runtime_file: RuntimeFile {
                id: runtime_file_id,
                origin: RuntimeFileOrigin::NewFile,
                backing: BackingAttachment::MemoryOnly,
                allowed_serialization_targets: vec![PhysicalSlotId(1)],
                lifecycle: RuntimeFileLifecycle::Active,
            },
            inactive_runtime_files: Vec::new(),
            physical_slots: Vec::new(),
            physical_slot_observations: Vec::new(),
            execution_context: ExecutionContext::World,
            location: SceneLocation {
                stage: "F_SP103".into(),
                room: 0,
                layer: 0,
                spawn: 0,
            },
            player: PlayerState {
                form: PlayerForm::Human,
                mount: None,
                position: [0.0; 3],
                rotation: [0; 3],
                has_control: Some(true),
                action: "idle".into(),
            },
            components: vec![return_place],
            static_world_objects: Vec::new(),
            spatial_volumes: Vec::new(),
            spatial_connections: Vec::new(),
            spatial_planes: Vec::new(),
            persisted_object_controls: Vec::new(),
            live_world_objects: Vec::new(),
        },
        semantic_observations: Vec::new(),
    };
    PlannerExecutionState::new(snapshot)
        .and_then(|state| state.to_document())
        .map_err(ProjectError::from)
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
    use crate::RuntimeEvidenceMode;
    use crate::service::{
        PlannerServiceOutcome, PlannerServicePayload, PlannerServiceRequest, handle_request,
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
        assert_eq!(list.projects.len(), 2);
        assert!(list.projects.iter().all(|project| project.read_only));
        assert!(
            list.projects
                .iter()
                .any(|project| project.id == "demo-fanadi-return-place")
        );
        let fanadi = store.load("demo-fanadi-return-place").unwrap();
        assert!(fanadi.project.start_state.is_some());
        assert_eq!(fanadi.project.catalog.mechanics.goals.len(), 1);
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
