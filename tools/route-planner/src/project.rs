//! Planner-owned web project artifacts and conflict-safe workspace storage.

use dusklight_route_planner::PlannerContractError;
use dusklight_route_planner::artifact::Digest;
use dusklight_route_planner::execution::{PlannerExecutionState, PlannerExecutionStateDocument};
use dusklight_route_planner::graph::PlannerGraph;
use dusklight_route_planner::identity::{
    CONTENT_IDENTITY_SCHEMA, ContentFingerprint, ContentIdentity, ContextSelector, EquivalenceSet,
    GamePlatform, GameRegion, RUNTIME_CONFIGURATION_SCHEMA, RuntimeConfiguration,
};
use dusklight_route_planner::logic::{
    ComparisonOperator, ContextScope, DerivedFact, EvidenceKind, EvidenceRecord,
    FACT_CATALOG_SCHEMA, FactCatalog, FriendlyAlias, PredicateExpression, RawFactBinding,
    RuleEvidence, TruthStatus, ValueReference,
};
use dusklight_route_planner::refinement::ComposedPlannerCatalog;
use dusklight_route_planner::return_place::{
    GZ2E01_CONTENT_SHA256, gz2e01_tower_return_place_mechanics,
};
use dusklight_route_planner::route_book::RouteBook;
use dusklight_route_planner::snapshot::{STATE_SNAPSHOT_SCHEMA, StateSnapshot};
use dusklight_route_planner::state::{
    BackingAttachment, ComponentBinding, ComponentBindingReference, ComponentKind,
    ComponentPayload, ComponentProvenance, ComponentSelector, EXECUTION_ENVIRONMENT_SCHEMA,
    ExecutionContext, ExecutionEnvironment, PhysicalSlotId, PlayerForm, PlayerState,
    ProvenanceSourceKind, RuntimeFile, RuntimeFileLifecycle, RuntimeFileOrigin, SceneLocation,
    SemanticLifetime, SerializationOwner, StateComponent, StateValue,
};
use dusklight_route_planner::title_boundary::gz2e01_reset_to_opening_mechanics;
use dusklight_route_planner::transition::{
    ActivationContract, ActorReconstructionRule, CandidateTransition, ComponentFieldTarget, Goal,
    MECHANICS_CATALOG_SCHEMA, MechanicsCatalog, StateOperation, TransitionKind,
};
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
    let mut opening_mechanics = gz2e01_reset_to_opening_mechanics(&content, &runtime)?;
    opening_mechanics.goals.push(Goal {
        id: "goal.enter-opening-process".into(),
        label: "Enter the opening process with its exact pending load".into(),
        predicate: PredicateExpression::All {
            terms: vec![
                PredicateExpression::Compare {
                    left: ValueReference::ExecutionProcess,
                    operator: ComparisonOperator::Equal,
                    right: ValueReference::Literal {
                        value: StateValue::Text("PROC_OPENING_SCENE".into()),
                    },
                },
                PredicateExpression::Compare {
                    left: ValueReference::PendingWorldLoadStage,
                    operator: ComparisonOperator::Equal,
                    right: ValueReference::Literal {
                        value: StateValue::Text("F_SP102".into()),
                    },
                },
            ],
        },
    });
    let opening = ComposedPlannerCatalog::compose(&facts, &opening_mechanics, &[])?;
    let (keyed_door, keyed_door_start) = keyed_door_demo(&facts, runtime.clone())?;
    let (rebind, rebind_start) = hypothetical_rebind_demo(runtime.clone())?;
    let projects = vec![
        PlannerWebProject {
            schema: WEB_PROJECT_SCHEMA.into(),
            id: "demo-forest-keyed-door".into(),
            label: "Forest Temple small-key door".into(),
            catalog: keyed_door,
            evidence_mode: crate::RuntimeEvidenceMode::EstablishedOnly,
            route_book: None,
            start_state: Some(keyed_door_start),
            equivalence_sets: Vec::new(),
            presentation: ProjectPresentation::default(),
        },
        PlannerWebProject {
            schema: WEB_PROJECT_SCHEMA.into(),
            id: "demo-hypothetical-local-bank-rebind".into(),
            label: "Hypothetical local-bank rebind".into(),
            catalog: rebind,
            evidence_mode: crate::RuntimeEvidenceMode::Research,
            route_book: None,
            start_state: Some(rebind_start),
            equivalence_sets: Vec::new(),
            presentation: ProjectPresentation::default(),
        },
        PlannerWebProject {
            schema: WEB_PROJECT_SCHEMA.into(),
            id: "demo-fanadi-return-place".into(),
            label: "Fanadi return-place locking".into(),
            catalog: fanadi,
            evidence_mode: crate::RuntimeEvidenceMode::EstablishedOnly,
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
            evidence_mode: crate::RuntimeEvidenceMode::EstablishedOnly,
            route_book: None,
            start_state: Some(opening_start_state(runtime)?),
            equivalence_sets: Vec::new(),
            presentation: ProjectPresentation::default(),
        },
    ];
    for project in &projects {
        project.validate()?;
    }
    Ok(projects)
}

fn hypothetical_rebind_demo(
    runtime_configuration: RuntimeConfiguration,
) -> Result<(ComposedPlannerCatalog, PlannerExecutionStateDocument), ProjectError> {
    const BANK_ID: &str = "stage.local-bank";
    let scope = ContextScope {
        selectors: vec![ContextSelector::Exact {
            context: runtime_configuration.exact_context()?,
        }],
    };
    let established = RuleEvidence {
        truth: TruthStatus::Established,
        records: vec![EvidenceRecord {
            id: "source.local-bank-binding-model".into(),
            kind: EvidenceKind::SourceAudited,
            source_sha256: Some(Digest([0x61; 32])),
            note: "Stage-local values derive meaning from their current backing binding.".into(),
        }],
    };
    let hypothetical = RuleEvidence {
        truth: TruthStatus::Hypothetical,
        records: vec![EvidenceRecord {
            id: "hypothesis.local-bank-rebind".into(),
            kind: EvidenceKind::Theorycraft,
            source_sha256: None,
            note: "Explicit research-only transfer of one preserved local bank.".into(),
        }],
    };
    let raw_binding = |stage: &str| RawFactBinding {
        component_kind: ComponentKind::StageMemory,
        binding: ComponentBindingReference::Exact {
            binding: ComponentBinding::Stage {
                stage: stage.into(),
            },
        },
        byte_offset: 0,
        mask: vec![0x01],
        expected: vec![0x01],
    };
    let facts = FactCatalog {
        schema: FACT_CATALOG_SCHEMA.into(),
        aliases: vec![
            FriendlyAlias {
                id: "local.forest-switch".into(),
                label: "Forest local switch".into(),
                scope: scope.clone(),
                raw: raw_binding("D_MN05"),
                evidence: established.clone(),
            },
            FriendlyAlias {
                id: "local.tot-switch".into(),
                label: "Temple of Time local switch".into(),
                scope: scope.clone(),
                raw: raw_binding("D_MN06"),
                evidence: established.clone(),
            },
        ],
        derived_facts: vec![DerivedFact {
            id: "path.tot-open".into(),
            label: "Temple of Time local path is open".into(),
            scope: scope.clone(),
            rule: PredicateExpression::Fact {
                fact_id: "local.tot-switch".into(),
            },
            evidence: established.clone(),
        }],
    };
    let stage_is = |stage: &str| PredicateExpression::Compare {
        left: ValueReference::LocationStage,
        operator: ComparisonOperator::Equal,
        right: ValueReference::Literal {
            value: StateValue::Text(stage.into()),
        },
    };
    let mut mechanics = empty_mechanics();
    mechanics.transitions = vec![
        CandidateTransition {
            id: "transition.hypothetical-local-bank-rebind".into(),
            label: "Hypothetically preserve Forest memory and rebind it to Temple of Time".into(),
            scope: scope.clone(),
            transition_kind: TransitionKind::Technique,
            approach_id: "approach.hypothetical-local-bank-rebind".into(),
            activation: ActivationContract {
                hard_guards: PredicateExpression::All {
                    terms: vec![
                        stage_is("STAGE_A"),
                        PredicateExpression::Fact {
                            fact_id: "local.forest-switch".into(),
                        },
                    ],
                },
                physical_obligation_ids: Vec::new(),
                effects: vec![
                    StateOperation::Preserve {
                        selector: ComponentSelector::Id {
                            component_id: BANK_ID.into(),
                        },
                    },
                    StateOperation::Rebind {
                        selector: ComponentSelector::Id {
                            component_id: BANK_ID.into(),
                        },
                        binding: ComponentBinding::Stage {
                            stage: "D_MN06".into(),
                        },
                    },
                ],
                unknown_requirements: Vec::new(),
            },
            evidence: hypothetical,
        },
        CandidateTransition {
            id: "transition.enter-temple-path".into(),
            label: "Enter the Temple of Time path opened by the rebound value".into(),
            scope: scope.clone(),
            transition_kind: TransitionKind::ActorDriven,
            approach_id: "approach.temple-local-path".into(),
            activation: ActivationContract {
                hard_guards: PredicateExpression::All {
                    terms: vec![
                        stage_is("STAGE_A"),
                        PredicateExpression::Fact {
                            fact_id: "path.tot-open".into(),
                        },
                    ],
                },
                physical_obligation_ids: Vec::new(),
                effects: vec![StateOperation::SetLocation {
                    location: SceneLocation {
                        stage: "STAGE_B".into(),
                        room: 0,
                        layer: 0,
                        spawn: 0,
                    },
                }],
                unknown_requirements: Vec::new(),
            },
            evidence: established,
        },
    ];
    mechanics
        .transitions
        .sort_by(|left, right| left.id.cmp(&right.id));
    mechanics.goals.push(Goal {
        id: "goal.enter-rebound-temple-path".into(),
        label: "Enter the path exposed by the hypothetical rebind".into(),
        predicate: stage_is("STAGE_B"),
    });
    let catalog = ComposedPlannerCatalog::compose(&facts, &mechanics, &[])?;
    let snapshot = StateSnapshot {
        schema: STATE_SNAPSHOT_SCHEMA.into(),
        id: "snapshot.before-hypothetical-local-bank-rebind".into(),
        sequence: 0,
        environment: ExecutionEnvironment {
            schema: EXECUTION_ENVIRONMENT_SCHEMA.into(),
            runtime_configuration,
            active_runtime_file: RuntimeFile {
                id: "file-0".into(),
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
                stage: "STAGE_A".into(),
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
            components: vec![StateComponent {
                id: BANK_ID.into(),
                component_kind: ComponentKind::StageMemory,
                payload: ComponentPayload::Raw {
                    bytes: vec![0x01],
                    known_mask: vec![0xff],
                },
                binding: ComponentBinding::Stage {
                    stage: "D_MN05".into(),
                },
                lifetime: SemanticLifetime::StageLoad,
                serialization_owner: SerializationOwner::StageBank {
                    runtime_file_id: "file-0".into(),
                    stage: "D_MN05".into(),
                },
                provenance: vec![ComponentProvenance {
                    source_kind: ProvenanceSourceKind::TraceObservation,
                    source_id: "trace.forest-local-bank".into(),
                    source_sha256: Some(Digest([0x62; 32])),
                    transition_id: None,
                }],
            }],
            static_world_objects: Vec::new(),
            spatial_volumes: Vec::new(),
            spatial_connections: Vec::new(),
            spatial_planes: Vec::new(),
            persisted_object_controls: Vec::new(),
            live_world_objects: Vec::new(),
        },
        semantic_observations: Vec::new(),
    };
    let document = PlannerExecutionState::new(snapshot)?.to_document()?;
    Ok((catalog, document))
}

fn keyed_door_demo(
    facts: &FactCatalog,
    runtime_configuration: RuntimeConfiguration,
) -> Result<(ComposedPlannerCatalog, PlannerExecutionStateDocument), ProjectError> {
    const ACTOR_ID: &str = "actor.gz2e01-d-mn05-door-1";
    const DUNGEON_ID: &str = "dungeon.d-mn05-memory";
    const KEY_DELTA_ID: &str = "session.pending-key-delta";
    let scope = ContextScope {
        selectors: vec![ContextSelector::Exact {
            context: runtime_configuration.exact_context()?,
        }],
    };
    let evidence = RuleEvidence {
        truth: TruthStatus::Established,
        records: vec![EvidenceRecord {
            id: "source.gz2e01-forest-door-1-audit".into(),
            kind: EvidenceKind::SourceAudited,
            source_sha256: Some(
                "9d08ac55fce27a6a741a6a502a4a2502146c3ff91abeb7d8c44824a6df8325a4"
                    .parse()
                    .map_err(|_| project_error("keyed-door evidence digest is invalid"))?,
            ),
            note: "GZ2E01 D_MN05 Door[1] placement and DOOR20 event program audit.".into(),
        }],
    };
    let actor_field = |field: &str| ValueReference::ComponentField {
        component_id: ACTOR_ID.into(),
        field: field.into(),
    };
    let dungeon_field = |field: &str| ValueReference::BoundComponentField {
        component_kind: ComponentKind::DungeonMemory,
        binding: ComponentBindingReference::Exact {
            binding: ComponentBinding::Dungeon {
                dungeon: "forest-temple".into(),
            },
        },
        field: field.into(),
    };
    let compare = |left: ValueReference, operator: ComparisonOperator, value: StateValue| {
        PredicateExpression::Compare {
            left,
            operator,
            right: ValueReference::Literal { value },
        }
    };
    let actor_is = |field: &str, value: StateValue| {
        compare(actor_field(field), ComparisonOperator::Equal, value)
    };
    let dungeon_is = |field: &str, value: StateValue| {
        compare(dungeon_field(field), ComparisonOperator::Equal, value)
    };
    let stage_is = || {
        compare(
            ValueReference::LocationStage,
            ComparisonOperator::Equal,
            StateValue::Text("D_MN05".into()),
        )
    };
    let room_is = |room: i64| {
        compare(
            ValueReference::LocationRoom,
            ComparisonOperator::Equal,
            StateValue::Signed(room),
        )
    };
    let at_front = || PredicateExpression::All {
        terms: vec![stage_is(), room_is(1)],
    };
    let target = |component_id: &str, field: &str| ComponentFieldTarget {
        component_id: component_id.into(),
        field: field.into(),
    };
    let candidate = |id: &str,
                     label: &str,
                     hard_guards: PredicateExpression,
                     effects: Vec<StateOperation>| CandidateTransition {
        id: id.into(),
        label: label.into(),
        scope: scope.clone(),
        transition_kind: TransitionKind::Door,
        approach_id: "approach.gz2e01-d-mn05-door1-front".into(),
        activation: ActivationContract {
            hard_guards,
            physical_obligation_ids: Vec::new(),
            effects,
            unknown_requirements: Vec::new(),
        },
        evidence: evidence.clone(),
    };

    let mut mechanics = empty_mechanics();
    mechanics.transitions = vec![
        candidate(
            "transition.gz2e01-door1-01-offer-event",
            "Offer the front-side shutter event",
            PredicateExpression::All {
                terms: vec![
                    at_front(),
                    actor_is("adjoining_room_loaded", StateValue::Boolean(true)),
                    actor_is("approach_permitted", StateValue::Boolean(true)),
                    actor_is("event_offered", StateValue::Boolean(false)),
                    PredicateExpression::Any {
                        terms: vec![
                            dungeon_is("switch_0b", StateValue::Boolean(true)),
                            compare(
                                dungeon_field("small_keys"),
                                ComparisonOperator::GreaterThan,
                                StateValue::Unsigned(0),
                            ),
                        ],
                    },
                ],
            },
            vec![StateOperation::Write {
                target: target(ACTOR_ID, "event_offered"),
                value: StateValue::Boolean(true),
            }],
        ),
        candidate(
            "transition.gz2e01-door1-02-demo-action8",
            "Run keyed shutter demo action 8",
            PredicateExpression::All {
                terms: vec![
                    at_front(),
                    actor_is("event_offered", StateValue::Boolean(true)),
                    actor_is("front_option", StateValue::Unsigned(2)),
                    actor_is("locked", StateValue::Boolean(true)),
                    dungeon_is("switch_0b", StateValue::Boolean(false)),
                    compare(
                        dungeon_field("small_keys"),
                        ComparisonOperator::GreaterThan,
                        StateValue::Unsigned(0),
                    ),
                    compare(
                        ValueReference::ComponentField {
                            component_id: KEY_DELTA_ID.into(),
                            field: "pending_delta".into(),
                        },
                        ComparisonOperator::Equal,
                        StateValue::Signed(0),
                    ),
                ],
            },
            vec![
                StateOperation::Write {
                    target: target(DUNGEON_ID, "switch_0b"),
                    value: StateValue::Boolean(true),
                },
                StateOperation::Write {
                    target: target(KEY_DELTA_ID, "pending_delta"),
                    value: StateValue::Signed(-1),
                },
                StateOperation::Write {
                    target: target(ACTOR_ID, "keyhole_animation"),
                    value: StateValue::Text("keyhole-opening".into()),
                },
            ],
        ),
        candidate(
            "transition.gz2e01-door1-03-finish-keyhole",
            "Finish the keyhole child animation",
            PredicateExpression::All {
                terms: vec![
                    at_front(),
                    dungeon_is("switch_0b", StateValue::Boolean(true)),
                    actor_is("locked", StateValue::Boolean(true)),
                    actor_is(
                        "keyhole_animation",
                        StateValue::Text("keyhole-opening".into()),
                    ),
                ],
            },
            vec![
                StateOperation::Write {
                    target: target(ACTOR_ID, "locked"),
                    value: StateValue::Boolean(false),
                },
                StateOperation::Write {
                    target: target(ACTOR_ID, "keyhole_animation"),
                    value: StateValue::Text("open".into()),
                },
            ],
        ),
        candidate(
            "transition.gz2e01-door1-04-flush-key-delta",
            "Apply the pending key delta to active stage memory",
            compare(
                ValueReference::ComponentField {
                    component_id: KEY_DELTA_ID.into(),
                    field: "pending_delta".into(),
                },
                ComparisonOperator::Equal,
                StateValue::Signed(-1),
            ),
            vec![
                StateOperation::Adjust {
                    target: target(DUNGEON_ID, "small_keys"),
                    delta: -1,
                },
                StateOperation::Write {
                    target: target(KEY_DELTA_ID, "pending_delta"),
                    value: StateValue::Signed(0),
                },
            ],
        ),
        candidate(
            "transition.gz2e01-door1-05-open-init",
            "Release shutter collision and select room 2",
            PredicateExpression::All {
                terms: vec![
                    at_front(),
                    actor_is("event_offered", StateValue::Boolean(true)),
                    actor_is("locked", StateValue::Boolean(false)),
                    actor_is("collision_registered", StateValue::Boolean(true)),
                    actor_is("door_animation", StateValue::Text("closed".into())),
                    dungeon_is("switch_0b", StateValue::Boolean(true)),
                ],
            },
            vec![
                StateOperation::Write {
                    target: target(ACTOR_ID, "collision_registered"),
                    value: StateValue::Boolean(false),
                },
                StateOperation::Write {
                    target: target(ACTOR_ID, "door_animation"),
                    value: StateValue::Text("opening".into()),
                },
                StateOperation::Write {
                    target: target(ACTOR_ID, "next_room"),
                    value: StateValue::Unsigned(2),
                },
            ],
        ),
        candidate(
            "transition.gz2e01-door1-06-open-proc",
            "Finish the wooden shutter opening animation",
            PredicateExpression::All {
                terms: vec![
                    at_front(),
                    actor_is("collision_registered", StateValue::Boolean(false)),
                    actor_is("door_animation", StateValue::Text("opening".into())),
                ],
            },
            vec![StateOperation::Write {
                target: target(ACTOR_ID, "door_animation"),
                value: StateValue::Text("open".into()),
            }],
        ),
        candidate(
            "transition.gz2e01-door1-07-cross-room-adjacency",
            "Cross the encoded room-1 to room-2 adjacency",
            PredicateExpression::All {
                terms: vec![
                    at_front(),
                    actor_is("next_room", StateValue::Unsigned(2)),
                    actor_is("collision_registered", StateValue::Boolean(false)),
                    actor_is("door_animation", StateValue::Text("open".into())),
                ],
            },
            vec![StateOperation::SetLocation {
                location: SceneLocation {
                    stage: "D_MN05".into(),
                    room: 2,
                    layer: 0,
                    spawn: 0,
                },
            }],
        ),
        candidate(
            "transition.gz2e01-door1-08-close-init",
            "Re-register shutter collision after crossing",
            PredicateExpression::All {
                terms: vec![
                    stage_is(),
                    room_is(2),
                    actor_is("collision_registered", StateValue::Boolean(false)),
                    actor_is("door_animation", StateValue::Text("open".into())),
                ],
            },
            vec![
                StateOperation::Write {
                    target: target(ACTOR_ID, "collision_registered"),
                    value: StateValue::Boolean(true),
                },
                StateOperation::Write {
                    target: target(ACTOR_ID, "door_animation"),
                    value: StateValue::Text("closing".into()),
                },
            ],
        ),
        candidate(
            "transition.gz2e01-door1-09-close-end",
            "Finish closing the unlocked shutter",
            PredicateExpression::All {
                terms: vec![
                    stage_is(),
                    room_is(2),
                    actor_is("collision_registered", StateValue::Boolean(true)),
                    actor_is("door_animation", StateValue::Text("closing".into())),
                ],
            },
            vec![
                StateOperation::Write {
                    target: target(ACTOR_ID, "door_animation"),
                    value: StateValue::Text("closed".into()),
                },
                StateOperation::Write {
                    target: target(ACTOR_ID, "keyhole_present"),
                    value: StateValue::Boolean(false),
                },
                StateOperation::Write {
                    target: target(ACTOR_ID, "keyhole_animation"),
                    value: StateValue::Text("deleted".into()),
                },
                StateOperation::Write {
                    target: target(ACTOR_ID, "event_offered"),
                    value: StateValue::Boolean(false),
                },
            ],
        ),
    ];
    let reconstruct =
        |id: &str, unlocked: bool, locked: bool, keyhole: bool| ActorReconstructionRule {
            id: id.into(),
            label: format!(
                "Reconstruct GZ2E01 Forest door 1 as {}",
                if unlocked { "unlocked" } else { "locked" }
            ),
            scope: scope.clone(),
            actor_type: "door20".into(),
            instantiate_when: dungeon_is("switch_0b", StateValue::Boolean(unlocked)),
            initialization_operations: vec![
                StateOperation::Write {
                    target: target(ACTOR_ID, "locked"),
                    value: StateValue::Boolean(locked),
                },
                StateOperation::Write {
                    target: target(ACTOR_ID, "keyhole_present"),
                    value: StateValue::Boolean(keyhole),
                },
                StateOperation::Write {
                    target: target(ACTOR_ID, "collision_registered"),
                    value: StateValue::Boolean(true),
                },
                StateOperation::Write {
                    target: target(ACTOR_ID, "door_animation"),
                    value: StateValue::Text("closed".into()),
                },
                StateOperation::Write {
                    target: target(ACTOR_ID, "keyhole_animation"),
                    value: StateValue::Text(if keyhole { "closed" } else { "deleted" }.into()),
                },
            ],
            evidence: evidence.clone(),
        };
    mechanics.reconstruction_rules = vec![
        reconstruct("reconstruct.gz2e01-door1-locked", false, true, true),
        reconstruct("reconstruct.gz2e01-door1-unlocked", true, false, false),
    ];
    mechanics.goals.push(Goal {
        id: "goal.gz2e01-forest-door1-complete".into(),
        label: "Cross and close Forest Temple door 1".into(),
        predicate: PredicateExpression::All {
            terms: vec![
                stage_is(),
                room_is(2),
                dungeon_is("switch_0b", StateValue::Boolean(true)),
                dungeon_is("small_keys", StateValue::Unsigned(0)),
                actor_is("collision_registered", StateValue::Boolean(true)),
                actor_is("door_animation", StateValue::Text("closed".into())),
                actor_is("keyhole_present", StateValue::Boolean(false)),
            ],
        },
    });
    let catalog = ComposedPlannerCatalog::compose(facts, &mechanics, &[])?;
    let snapshot = keyed_door_start_snapshot(runtime_configuration);
    let document = PlannerExecutionState::new(snapshot)?.to_document()?;
    Ok((catalog, document))
}

fn keyed_door_start_snapshot(runtime_configuration: RuntimeConfiguration) -> StateSnapshot {
    const ACTOR_ID: &str = "actor.gz2e01-d-mn05-door-1";
    let runtime_file_id = "file-0".to_owned();
    StateSnapshot {
        schema: STATE_SNAPSHOT_SCHEMA.into(),
        id: "snapshot.gz2e01-d-mn05-r01-door1-closed".into(),
        sequence: 0,
        environment: ExecutionEnvironment {
            schema: EXECUTION_ENVIRONMENT_SCHEMA.into(),
            runtime_configuration,
            active_runtime_file: RuntimeFile {
                id: runtime_file_id.clone(),
                origin: RuntimeFileOrigin::LoadedSlot {
                    slot: PhysicalSlotId(1),
                },
                backing: BackingAttachment::CardBacked {
                    slot: PhysicalSlotId(1),
                },
                allowed_serialization_targets: vec![PhysicalSlotId(1)],
                lifecycle: RuntimeFileLifecycle::Active,
            },
            inactive_runtime_files: Vec::new(),
            physical_slots: Vec::new(),
            physical_slot_observations: Vec::new(),
            execution_context: ExecutionContext::World,
            location: SceneLocation {
                stage: "D_MN05".into(),
                room: 1,
                layer: 0,
                spawn: 0,
            },
            player: PlayerState {
                form: PlayerForm::Human,
                mount: None,
                position: [10436.0, 3752.0, 4870.0],
                rotation: [0, -16384, 0],
                has_control: Some(true),
                action: "door-approach".into(),
            },
            components: vec![
                StateComponent {
                    id: ACTOR_ID.into(),
                    component_kind: ComponentKind::ActorInstance,
                    payload: ComponentPayload::Structured {
                        fields: BTreeMap::from([
                            ("adjoining_room_loaded".into(), StateValue::Boolean(true)),
                            ("approach_permitted".into(), StateValue::Boolean(true)),
                            ("back_option".into(), StateValue::Unsigned(0)),
                            ("back_room".into(), StateValue::Unsigned(2)),
                            ("collision_registered".into(), StateValue::Boolean(true)),
                            ("door_animation".into(), StateValue::Text("closed".into())),
                            ("event_offered".into(), StateValue::Boolean(false)),
                            ("front_option".into(), StateValue::Unsigned(2)),
                            ("front_room".into(), StateValue::Unsigned(1)),
                            (
                                "keyhole_animation".into(),
                                StateValue::Text("closed".into()),
                            ),
                            ("keyhole_present".into(), StateValue::Boolean(true)),
                            ("kind".into(), StateValue::Unsigned(1)),
                            ("locked".into(), StateValue::Boolean(true)),
                            ("next_room".into(), StateValue::Unsigned(1)),
                            ("parameters".into(), StateValue::Unsigned(0x6c10_2201)),
                            ("unlock_switch".into(), StateValue::Unsigned(0x0b)),
                        ]),
                    },
                    binding: ComponentBinding::Actor {
                        instance_id: ACTOR_ID.into(),
                    },
                    lifetime: SemanticLifetime::RoomLoad,
                    serialization_owner: SerializationOwner::None,
                    provenance: vec![ComponentProvenance {
                        source_kind: ProvenanceSourceKind::ExtractedFact,
                        source_id: "gz2e01:d_mn05/stage.dzs/door/1".into(),
                        source_sha256: Some(
                            "9d08ac55fce27a6a741a6a502a4a2502146c3ff91abeb7d8c44824a6df8325a4"
                                .parse()
                                .expect("checked keyed-door source digest"),
                        ),
                        transition_id: None,
                    }],
                },
                StateComponent {
                    id: "dungeon.d-mn05-memory".into(),
                    component_kind: ComponentKind::DungeonMemory,
                    payload: ComponentPayload::Structured {
                        fields: BTreeMap::from([
                            ("small_keys".into(), StateValue::Unsigned(1)),
                            ("switch_0b".into(), StateValue::Boolean(false)),
                        ]),
                    },
                    binding: ComponentBinding::Dungeon {
                        dungeon: "forest-temple".into(),
                    },
                    lifetime: SemanticLifetime::StageLoad,
                    serialization_owner: SerializationOwner::StageBank {
                        runtime_file_id: runtime_file_id.clone(),
                        stage: "D_MN05".into(),
                    },
                    provenance: vec![ComponentProvenance {
                        source_kind: ProvenanceSourceKind::TraceObservation,
                        source_id: "gz2e01:dsv-memory/d-mn05".into(),
                        source_sha256: Some(Digest([0x45; 32])),
                        transition_id: None,
                    }],
                },
                StateComponent {
                    id: "session.pending-key-delta".into(),
                    component_kind: ComponentKind::Session,
                    payload: ComponentPayload::Structured {
                        fields: BTreeMap::from([("pending_delta".into(), StateValue::Signed(0))]),
                    },
                    binding: ComponentBinding::Session {
                        session_id: "session-1".into(),
                    },
                    lifetime: SemanticLifetime::Session,
                    serialization_owner: SerializationOwner::None,
                    provenance: vec![ComponentProvenance {
                        source_kind: ProvenanceSourceKind::Initialized,
                        source_id: "gz2e01:dcomifgp-item-key-delta".into(),
                        source_sha256: Some(Digest([0x46; 32])),
                        transition_id: None,
                    }],
                },
            ],
            static_world_objects: Vec::new(),
            spatial_volumes: Vec::new(),
            spatial_connections: Vec::new(),
            spatial_planes: Vec::new(),
            persisted_object_controls: Vec::new(),
            live_world_objects: Vec::new(),
        },
        semantic_observations: Vec::new(),
    }
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

fn opening_start_state(
    runtime_configuration: RuntimeConfiguration,
) -> Result<PlannerExecutionStateDocument, ProjectError> {
    let runtime_file_id = "file-0".to_owned();
    let component = |id: &str,
                     component_kind: ComponentKind,
                     fields: BTreeMap<String, StateValue>,
                     binding: ComponentBinding,
                     lifetime: SemanticLifetime,
                     serialization_owner: SerializationOwner| StateComponent {
        id: id.into(),
        component_kind,
        payload: ComponentPayload::Structured { fields },
        binding,
        lifetime,
        serialization_owner,
        provenance: vec![ComponentProvenance {
            source_kind: ProvenanceSourceKind::ExtractedFact,
            source_id: "demo.opening-reset".into(),
            source_sha256: None,
            transition_id: None,
        }],
    };
    let reset_control = component(
        "reset-control",
        ComponentKind::Session,
        BTreeMap::from([
            ("fader_status".into(), StateValue::Unsigned(1)),
            ("reset_requested".into(), StateValue::Boolean(true)),
            ("return_to_menu".into(), StateValue::Boolean(false)),
        ]),
        ComponentBinding::Session {
            session_id: "process".into(),
        },
        SemanticLifetime::Session,
        SerializationOwner::None,
    );
    let restart = component(
        "restart",
        ComponentKind::Restart,
        BTreeMap::from([("room_param".into(), StateValue::Unsigned(0xc9))]),
        ComponentBinding::RuntimeFile {
            runtime_file_id: runtime_file_id.clone(),
        },
        SemanticLifetime::RuntimeFile,
        SerializationOwner::RuntimeFile {
            runtime_file_id: runtime_file_id.clone(),
        },
    );
    let snapshot = StateSnapshot {
        schema: STATE_SNAPSHOT_SCHEMA.into(),
        id: "snapshot.opening-before-reset".into(),
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
                stage: "R_SP107".into(),
                room: 3,
                layer: 0,
                spawn: 0,
            },
            player: PlayerState {
                form: PlayerForm::Wolf,
                mount: None,
                position: [0.0; 3],
                rotation: [0; 3],
                has_control: Some(true),
                action: "idle".into(),
            },
            components: vec![reset_control, restart],
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
        assert_eq!(list.projects.len(), 4);
        assert!(list.projects.iter().all(|project| project.read_only));
        assert!(
            list.projects
                .iter()
                .any(|project| project.id == "demo-fanadi-return-place")
        );
        let fanadi = store.load("demo-fanadi-return-place").unwrap();
        assert!(fanadi.project.start_state.is_some());
        assert_eq!(fanadi.project.catalog.mechanics.goals.len(), 1);
        let opening = store.load("demo-opening-flow").unwrap();
        assert!(opening.project.start_state.is_some());
        assert_eq!(opening.project.catalog.mechanics.goals.len(), 1);
        let keyed_door = store.load("demo-forest-keyed-door").unwrap();
        assert!(keyed_door.project.start_state.is_some());
        assert_eq!(keyed_door.project.catalog.mechanics.transitions.len(), 9);
        assert_eq!(keyed_door.project.catalog.mechanics.goals.len(), 1);
        let rebind = store.load("demo-hypothetical-local-bank-rebind").unwrap();
        assert_eq!(rebind.project.evidence_mode, RuntimeEvidenceMode::Research);
        assert_eq!(rebind.project.catalog.mechanics.transitions.len(), 2);
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
