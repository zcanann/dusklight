//! File-backed planner workspaces.
//!
//! A workspace manifest is deliberately small. Mutable semantic assets and
//! presentation layouts live in independent, typed JSON documents so file
//! organization never becomes semantic identity.

use crate::project::PlannerWebProject;
use dusklight_route_planner::PlannerContractError;
use dusklight_route_planner::artifact::Digest;
use dusklight_route_planner::execution::PlannerExecutionStateDocument;
use dusklight_route_planner::graph::PlannerGraph;
use dusklight_route_planner::identity::{ContextSelector, ExactContext};
use dusklight_route_planner::logic::{ContextScope, PredicateExpression};
use dusklight_route_planner::refinement::ComposedPlannerCatalog;
use dusklight_route_planner::route_book::{ROUTE_BOOK_SCHEMA, RouteBook, RouteBookManifest};
use dusklight_route_planner::transition::StateOperation;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub const WORKSPACE_MANIFEST_SCHEMA: &str = "dusklight.route-planner.workspace/v1";
pub const WORKSPACE_ASSET_SCHEMA: &str = "dusklight.route-planner.workspace-asset/v1";
pub const WORKSPACE_FOLDER_SCHEMA: &str = "dusklight.route-planner.workspace-folder/v1";
pub const WORKSPACE_LIST_SCHEMA: &str = "dusklight.route-planner.workspace-list/v1";
pub const WORKSPACE_RECORD_SCHEMA: &str = "dusklight.route-planner.workspace-record/v2";
pub const WORKSPACE_CREATE_SCHEMA: &str = "dusklight.route-planner.workspace-create/v1";
pub const WORKSPACE_ASSET_RECORD_SCHEMA: &str = "dusklight.route-planner.workspace-asset-record/v1";
pub const WORKSPACE_ASSET_SAVE_SCHEMA: &str = "dusklight.route-planner.workspace-asset-save/v1";
pub const WORKSPACE_ASSET_COMMAND_SCHEMA: &str =
    "dusklight.route-planner.workspace-asset-command/v1";
pub const WORKSPACE_ROUTE_GRAPH_SAVE_SCHEMA: &str =
    "dusklight.route-planner.workspace-route-graph-save/v1";
pub const WORKSPACE_ROUTE_GRAPH_EDIT_RECORD_SCHEMA: &str =
    "dusklight.route-planner.workspace-route-graph-edit-record/v1";
pub const WORKSPACE_TRASH_COMMAND_SCHEMA: &str =
    "dusklight.route-planner.workspace-trash-command/v1";
pub const WORKSPACE_FOLDER_COMMAND_SCHEMA: &str =
    "dusklight.route-planner.workspace-folder-command/v1";
pub const WORKSPACE_FOLDER_TRASH_COMMAND_SCHEMA: &str =
    "dusklight.route-planner.workspace-folder-trash-command/v1";
pub const WORKSPACE_LIBRARY_FORK_SCHEMA: &str = "dusklight.route-planner.workspace-library-fork/v1";
pub const WORKSPACE_EXPORT_SCHEMA: &str = "dusklight.route-planner.workspace-export/v2";
pub const BUILTIN_LIBRARY_VERSION: &str = "builtin-v1";
pub const WORKSPACE_FORMAT_VERSION: u32 = 1;
const MANIFEST_FILE: &str = "workspace.json";
const LEGACY_WORKSPACE_MANIFEST_SCHEMA: &str = "dusklight.route-planner.workspace/v0";
const LEGACY_WORKSPACE_EXPORT_SCHEMA: &str = "dusklight.route-planner.workspace-export/v1";
const TRANSACTION_ROOT: &str = ".dusklight/transactions";
const TRASH_ROOT: &str = ".dusklight/trash";
const FOLDER_TRASH_ROOT: &str = ".dusklight/folder-trash";
const FOLDER_STAGING_ROOT: &str = ".dusklight/folder-staging";
const FOLDER_MARKER_FILE: &str = ".dusklight-folder.json";
const FOLDER_TRASH_RECORD_FILE: &str = "trash.json";
const FOLDER_TRASH_PAYLOAD: &str = "payload";
const FOLDER_TRASH_RECORD_SCHEMA: &str = "dusklight.route-planner.workspace-folder-trash/v1";
const TRANSACTION_SCHEMA: &str = "dusklight.route-planner.workspace-transaction/v1";
static NEXT_TEMPORARY_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceAssetKind {
    Scenario,
    RouteGraph,
    ReusableSubgraph,
    CustomNodeDefinition,
    StateSeed,
    QueryGoal,
    RouteBook,
    Layout,
}

impl WorkspaceAssetKind {
    pub const ALL: [Self; 8] = [
        Self::Scenario,
        Self::RouteGraph,
        Self::ReusableSubgraph,
        Self::CustomNodeDefinition,
        Self::StateSeed,
        Self::QueryGoal,
        Self::RouteBook,
        Self::Layout,
    ];

    pub const fn root_name(self) -> &'static str {
        match self {
            Self::Scenario => "scenarios",
            Self::RouteGraph => "route-graphs",
            Self::ReusableSubgraph => "subgraphs",
            Self::CustomNodeDefinition => "custom-nodes",
            Self::StateSeed => "state-seeds",
            Self::QueryGoal => "queries",
            Self::RouteBook => "route-books",
            Self::Layout => "layouts",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MountedLibrary {
    pub id: String,
    pub version: String,
    pub sha256: Digest,
    /// User-facing locator only. Identity is the `(id, version, sha256)` pin.
    pub source: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceManifest {
    pub schema: String,
    pub format_version: u32,
    pub id: String,
    pub label: String,
    pub mounted_libraries: Vec<MountedLibrary>,
    pub exact_context_defaults: Vec<ExactContext>,
    pub asset_roots: BTreeMap<WorkspaceAssetKind, String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceAssetHeader {
    pub id: String,
    pub label: String,
    pub kind: WorkspaceAssetKind,
    pub version: u32,
    #[serde(default)]
    pub origin: Option<WorkspaceAssetOrigin>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceAssetOrigin {
    pub library_id: String,
    pub library_version: String,
    pub library_sha256: Digest,
    pub source_asset_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceAssetReference {
    pub asset_id: String,
    pub kind: WorkspaceAssetKind,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceAsset {
    pub schema: String,
    pub header: WorkspaceAssetHeader,
    #[serde(default)]
    pub references: Vec<WorkspaceAssetReference>,
    pub payload: WorkspaceAssetPayload,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkspaceAssetPayload {
    Scenario(ScenarioAsset),
    RouteGraph {
        graph: PlannerGraph,
    },
    ReusableSubgraph {
        graph: PlannerGraph,
    },
    CustomNodeDefinition(CustomNodeDefinitionAsset),
    StateSeed {
        state: PlannerExecutionStateDocument,
    },
    QueryGoal(QueryGoalAsset),
    RouteBook {
        route_book: RouteBook,
    },
    Layout(LayoutAsset),
}

impl WorkspaceAssetPayload {
    pub const fn kind(&self) -> WorkspaceAssetKind {
        match self {
            Self::Scenario(_) => WorkspaceAssetKind::Scenario,
            Self::RouteGraph { .. } => WorkspaceAssetKind::RouteGraph,
            Self::ReusableSubgraph { .. } => WorkspaceAssetKind::ReusableSubgraph,
            Self::CustomNodeDefinition(_) => WorkspaceAssetKind::CustomNodeDefinition,
            Self::StateSeed { .. } => WorkspaceAssetKind::StateSeed,
            Self::QueryGoal(_) => WorkspaceAssetKind::QueryGoal,
            Self::RouteBook { .. } => WorkspaceAssetKind::RouteBook,
            Self::Layout(_) => WorkspaceAssetKind::Layout,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioAsset {
    pub exact_context: ExactContext,
    pub anchor: ScenarioAnchor,
    pub route_graph_id: String,
    pub state_seed_id: Option<String>,
    pub route_book_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ScenarioAnchor {
    FreshBoot,
    StateSeed { state_seed_id: String },
    AuthenticatedCheckpoint { checkpoint_sha256: Digest },
    EntryContract { predicate: PredicateExpression },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CustomNodeDefinitionAsset {
    pub inputs: Vec<CustomNodePin>,
    pub outputs: Vec<CustomNodePin>,
    pub guard: PredicateExpression,
    pub effects: Vec<StateOperation>,
    pub evidence_status: CustomNodeEvidenceStatus,
    #[serde(default)]
    pub evidence: Vec<CustomNodeEvidenceRecord>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CustomNodePin {
    pub id: String,
    pub label: String,
    pub value_type: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CustomNodeEvidenceRecord {
    pub id: String,
    pub source: String,
    pub note: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CustomNodeEvidenceStatus {
    Hypothetical,
    Research,
    Established,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct QueryGoalAsset {
    pub predicate: PredicateExpression,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LayoutAsset {
    /// Stable identity of the semantic graph this layout presents.
    pub semantic_asset_id: String,
    pub positions: BTreeMap<String, LayoutPoint>,
    #[serde(default)]
    pub viewport: Option<LayoutViewport>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LayoutPoint {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LayoutViewport {
    pub x: f64,
    pub y: f64,
    pub zoom: f64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceAssetListing {
    pub id: String,
    pub label: String,
    pub kind: WorkspaceAssetKind,
    pub relative_path: PathBuf,
    pub revision_sha256: Digest,
}

/// A mutable content-browser folder. Its stable identity is independent of its
/// on-disk directory and of every asset stored below it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceFolder {
    pub schema: String,
    pub id: String,
    pub label: String,
    pub kind: WorkspaceAssetKind,
    pub parent_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceFolderListing {
    pub id: String,
    pub label: String,
    pub kind: WorkspaceAssetKind,
    pub parent_id: Option<String>,
    pub relative_path: PathBuf,
    pub revision_sha256: Digest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceCreateRequest {
    pub schema: String,
    pub id: String,
    pub label: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceSummary {
    pub id: String,
    pub label: String,
    pub asset_count: usize,
    pub dependency_error: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceList {
    pub schema: String,
    pub workspaces: Vec<WorkspaceSummary>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceRecord {
    pub schema: String,
    pub manifest: WorkspaceManifest,
    pub folders: Vec<WorkspaceFolderListing>,
    pub assets: Vec<WorkspaceAssetListing>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceExport {
    pub schema: String,
    pub manifest: WorkspaceManifest,
    #[serde(default)]
    pub folders: Vec<WorkspaceExportFolder>,
    pub assets: Vec<WorkspaceExportAsset>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceExportFolder {
    pub relative_path: PathBuf,
    pub folder: WorkspaceFolder,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceExportAsset {
    pub relative_path: PathBuf,
    pub asset: WorkspaceAsset,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceAssetSaveRequest {
    pub schema: String,
    pub relative_path: PathBuf,
    pub expected_revision_sha256: Option<Digest>,
    pub asset: WorkspaceAsset,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceAssetRecord {
    pub schema: String,
    pub relative_path: PathBuf,
    pub revision_sha256: Digest,
    pub asset: WorkspaceAsset,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceRouteGraphSaveRequest {
    pub schema: String,
    pub expected_graph_revision_sha256: Digest,
    pub route_book_id: String,
    pub expected_route_book_revision_sha256: Digest,
    pub route_book: RouteBook,
    pub layout: Option<WorkspaceRouteGraphLayoutEdit>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceRouteGraphLayoutEdit {
    pub asset_id: String,
    pub expected_revision_sha256: Digest,
    pub positions: BTreeMap<String, LayoutPoint>,
    pub viewport: Option<LayoutViewport>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceRouteGraphEditRecord {
    pub schema: String,
    pub workspace: WorkspaceRecord,
    pub graph: WorkspaceAssetRecord,
    pub route_book: WorkspaceAssetRecord,
    pub layout: Option<WorkspaceAssetRecord>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceTrashListing {
    pub id: String,
    pub label: String,
    pub kind: WorkspaceAssetKind,
    pub original_relative_path: PathBuf,
    pub revision_sha256: Digest,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceFolderTrashListing {
    pub id: String,
    pub label: String,
    pub kind: WorkspaceAssetKind,
    pub original_relative_path: PathBuf,
    pub revision_sha256: Digest,
    pub folder_count: usize,
    pub asset_count: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceAssetCommandRequest {
    pub schema: String,
    pub command: WorkspaceAssetCommand,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkspaceAssetCommand {
    Rename {
        expected_revision_sha256: Digest,
        label: String,
    },
    Move {
        expected_revision_sha256: Digest,
        relative_path: PathBuf,
    },
    Duplicate {
        new_id: String,
        new_label: String,
        relative_path: PathBuf,
    },
    DeleteToTrash {
        expected_revision_sha256: Digest,
        allow_broken_references: bool,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceTrashCommandRequest {
    pub schema: String,
    pub expected_revision_sha256: Digest,
    pub command: WorkspaceTrashCommand,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceLibraryForkRequest {
    pub schema: String,
    pub namespace: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceTrashCommand {
    Restore,
    PermanentlyDelete,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceFolderCommandRequest {
    pub schema: String,
    pub command: WorkspaceFolderCommand,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkspaceFolderCommand {
    Create {
        id: String,
        label: String,
        asset_kind: WorkspaceAssetKind,
        parent_id: Option<String>,
        directory_name: String,
    },
    Rename {
        expected_revision_sha256: Digest,
        label: String,
        directory_name: String,
    },
    Move {
        expected_revision_sha256: Digest,
        parent_id: Option<String>,
    },
    Duplicate {
        new_id: String,
        new_label: String,
        parent_id: Option<String>,
        directory_name: String,
    },
    DeleteToTrash {
        expected_revision_sha256: Digest,
        allow_broken_references: bool,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceFolderTrashCommandRequest {
    pub schema: String,
    pub expected_revision_sha256: Digest,
    pub command: WorkspaceTrashCommand,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct WorkspaceFolderTrashRecord {
    schema: String,
    folder: WorkspaceFolder,
    original_relative_path: PathBuf,
    revision_sha256: Digest,
    folder_count: usize,
    asset_count: usize,
}

#[derive(Debug)]
pub struct WorkspaceRegistry {
    root: PathBuf,
    available_libraries: BTreeMap<(String, String), Digest>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum WorkspaceMutation {
    Put {
        relative_path: PathBuf,
        expected_revision_sha256: Option<Digest>,
        asset: WorkspaceAsset,
    },
    Delete {
        relative_path: PathBuf,
        expected_revision_sha256: Digest,
    },
    Archive {
        trash_relative_path: PathBuf,
        asset: WorkspaceAsset,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LibraryDependencyIssue {
    Missing {
        id: String,
        version: String,
        expected_sha256: Digest,
        source: String,
    },
    Changed {
        id: String,
        version: String,
        expected_sha256: Digest,
        actual_sha256: Digest,
        source: String,
    },
}

#[derive(Debug)]
pub struct WorkspaceError {
    detail: String,
}

impl WorkspaceError {
    fn new(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
        }
    }

    fn io(error: std::io::Error) -> Self {
        Self::new(error.to_string())
    }

    fn json(error: serde_json::Error) -> Self {
        Self::new(error.to_string())
    }
}

impl fmt::Display for WorkspaceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl Error for WorkspaceError {}

impl From<PlannerContractError> for WorkspaceError {
    fn from(value: PlannerContractError) -> Self {
        Self::new(value.to_string())
    }
}

#[derive(Debug)]
pub struct WorkspaceStore {
    root: PathBuf,
    manifest: WorkspaceManifest,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyWorkspaceManifest {
    schema: String,
    id: String,
    label: String,
    mounted_libraries: Vec<MountedLibrary>,
    exact_context_defaults: Vec<ExactContext>,
    asset_roots: BTreeMap<WorkspaceAssetKind, String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WorkspaceTransactionJournal {
    schema: String,
    id: String,
    operations: Vec<JournalOperation>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum JournalOperation {
    Put {
        relative_path: String,
        staged_file: String,
        expected_revision_sha256: Option<Digest>,
        new_revision_sha256: Digest,
    },
    Delete {
        relative_path: String,
        expected_revision_sha256: Digest,
    },
}

impl WorkspaceManifest {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Result<Self, WorkspaceError> {
        let manifest = Self {
            schema: WORKSPACE_MANIFEST_SCHEMA.into(),
            format_version: WORKSPACE_FORMAT_VERSION,
            id: id.into(),
            label: label.into(),
            mounted_libraries: Vec::new(),
            exact_context_defaults: Vec::new(),
            asset_roots: WorkspaceAssetKind::ALL
                .into_iter()
                .map(|kind| (kind, kind.root_name().into()))
                .collect(),
        };
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<(), WorkspaceError> {
        if self.schema != WORKSPACE_MANIFEST_SCHEMA
            || self.format_version != WORKSPACE_FORMAT_VERSION
        {
            return Err(WorkspaceError::new(format!(
                "workspace format is unsupported: schema {}, version {}; migrate it with this application before opening",
                self.schema, self.format_version
            )));
        }
        validate_stable_id("workspace id", &self.id)?;
        validate_label("workspace label", &self.label)?;
        if self.asset_roots.len() != WorkspaceAssetKind::ALL.len() {
            return Err(WorkspaceError::new(
                "workspace manifest must define every fixed asset root exactly once",
            ));
        }
        let mut roots = BTreeSet::new();
        for kind in WorkspaceAssetKind::ALL {
            let root = self
                .asset_roots
                .get(&kind)
                .ok_or_else(|| WorkspaceError::new(format!("missing {kind:?} asset root")))?;
            validate_relative_path("asset root", Path::new(root))?;
            if root != kind.root_name() {
                return Err(WorkspaceError::new(format!(
                    "{kind:?} asset root is fixed at {}; found {root}",
                    kind.root_name()
                )));
            }
            if !roots.insert(root) {
                return Err(WorkspaceError::new("asset roots must be unique"));
            }
        }
        let mut libraries = BTreeSet::new();
        for library in &self.mounted_libraries {
            validate_stable_id("library id", &library.id)?;
            validate_label("library version", &library.version)?;
            validate_label("library source", &library.source)?;
            if library.sha256 == Digest::ZERO {
                return Err(WorkspaceError::new("library digest must be nonzero"));
            }
            if !libraries.insert((&library.id, &library.version)) {
                return Err(WorkspaceError::new(format!(
                    "library {} version {} is mounted more than once",
                    library.id, library.version
                )));
            }
        }
        if !self.exact_context_defaults.iter().all(|context| {
            context.content_sha256 != Digest::ZERO
                && context.runtime_configuration_sha256 != Digest::ZERO
        }) {
            return Err(WorkspaceError::new(
                "exact-context defaults must use nonzero content and runtime digests",
            ));
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, WorkspaceError> {
        self.validate()?;
        canonical_json(self)
    }
}

impl WorkspaceAsset {
    pub fn validate(&self) -> Result<(), WorkspaceError> {
        if self.schema != WORKSPACE_ASSET_SCHEMA {
            return Err(WorkspaceError::new(format!(
                "asset {} uses unsupported schema {}; migrate it with this application before opening",
                self.header.id, self.schema
            )));
        }
        validate_stable_id("asset id", &self.header.id)?;
        validate_label("asset label", &self.header.label)?;
        if self.header.version == 0 {
            return Err(WorkspaceError::new("asset version must be positive"));
        }
        if let Some(origin) = &self.header.origin {
            validate_stable_id("asset origin library id", &origin.library_id)?;
            validate_label("asset origin library version", &origin.library_version)?;
            validate_stable_id("asset origin source asset id", &origin.source_asset_id)?;
            if origin.library_sha256 == Digest::ZERO {
                return Err(WorkspaceError::new(
                    "asset origin library digest must be nonzero",
                ));
            }
        }
        if self.header.kind != self.payload.kind() {
            return Err(WorkspaceError::new(format!(
                "asset {} header kind does not match its typed payload",
                self.header.id
            )));
        }
        let mut previous = None;
        for reference in &self.references {
            validate_stable_id("asset reference", &reference.asset_id)?;
            if previous.is_some_and(|value| value >= reference) {
                return Err(WorkspaceError::new(
                    "asset references must be unique and sorted",
                ));
            }
            previous = Some(reference);
        }
        match &self.payload {
            WorkspaceAssetPayload::Scenario(scenario) => {
                validate_stable_id("scenario route graph id", &scenario.route_graph_id)?;
                if let Some(id) = &scenario.state_seed_id {
                    validate_stable_id("scenario state seed id", id)?;
                }
                if let Some(id) = &scenario.route_book_id {
                    validate_stable_id("scenario route book id", id)?;
                }
                if let ScenarioAnchor::StateSeed { state_seed_id } = &scenario.anchor {
                    validate_stable_id("scenario anchor state seed id", state_seed_id)?;
                    if scenario.state_seed_id.as_ref() != Some(state_seed_id) {
                        return Err(WorkspaceError::new(
                            "state-seed scenario anchor must match scenario state_seed_id",
                        ));
                    }
                }
                if let ScenarioAnchor::AuthenticatedCheckpoint { checkpoint_sha256 } =
                    scenario.anchor
                    && checkpoint_sha256 == Digest::ZERO
                {
                    return Err(WorkspaceError::new(
                        "authenticated checkpoint digest must be nonzero",
                    ));
                }
                if let ScenarioAnchor::EntryContract { predicate } = &scenario.anchor {
                    predicate.validate()?;
                }
            }
            WorkspaceAssetPayload::RouteGraph { graph }
            | WorkspaceAssetPayload::ReusableSubgraph { graph } => graph.validate()?,
            WorkspaceAssetPayload::CustomNodeDefinition(node) => {
                node.guard.validate()?;
                validate_pins("custom node inputs", &node.inputs)?;
                validate_pins("custom node outputs", &node.outputs)?;
                let mut evidence_ids = BTreeSet::new();
                for evidence in &node.evidence {
                    validate_stable_id("custom node evidence id", &evidence.id)?;
                    validate_label("custom node evidence source", &evidence.source)?;
                    validate_label("custom node evidence note", &evidence.note)?;
                    if !evidence_ids.insert(&evidence.id) {
                        return Err(WorkspaceError::new(format!(
                            "custom node evidence contains duplicate {}",
                            evidence.id
                        )));
                    }
                }
                if node.evidence_status == CustomNodeEvidenceStatus::Established
                    && node.evidence.is_empty()
                {
                    return Err(WorkspaceError::new(
                        "an established custom node requires explicit evidence",
                    ));
                }
            }
            WorkspaceAssetPayload::StateSeed { state } => state.validate()?,
            WorkspaceAssetPayload::QueryGoal(goal) => goal.predicate.validate()?,
            WorkspaceAssetPayload::RouteBook { route_book } => route_book.validate()?,
            WorkspaceAssetPayload::Layout(layout) => {
                validate_stable_id("layout semantic asset id", &layout.semantic_asset_id)?;
                if layout
                    .positions
                    .values()
                    .any(|point| !point.x.is_finite() || !point.y.is_finite())
                {
                    return Err(WorkspaceError::new("layout positions must be finite"));
                }
                if let Some(viewport) = layout.viewport
                    && (!viewport.x.is_finite()
                        || !viewport.y.is_finite()
                        || !viewport.zoom.is_finite()
                        || viewport.zoom <= 0.0)
                {
                    return Err(WorkspaceError::new(
                        "layout viewport must be finite with positive zoom",
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, WorkspaceError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn digest(&self) -> Result<Digest, WorkspaceError> {
        let digest = Sha256::digest(self.canonical_bytes()?);
        Ok(Digest(digest.into()))
    }
}

impl WorkspaceFolder {
    pub fn validate(&self) -> Result<(), WorkspaceError> {
        if self.schema != WORKSPACE_FOLDER_SCHEMA {
            return Err(WorkspaceError::new(format!(
                "folder {} uses unsupported schema {}",
                self.id, self.schema
            )));
        }
        validate_stable_id("folder id", &self.id)?;
        validate_label("folder label", &self.label)?;
        if self.parent_id.as_ref() == Some(&self.id) {
            return Err(WorkspaceError::new("folder cannot be its own parent"));
        }
        if let Some(parent_id) = &self.parent_id {
            validate_stable_id("folder parent id", parent_id)?;
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, WorkspaceError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn digest(&self) -> Result<Digest, WorkspaceError> {
        Ok(Digest(Sha256::digest(self.canonical_bytes()?).into()))
    }
}

impl WorkspaceRegistry {
    pub fn open(
        root: impl Into<PathBuf>,
        available_libraries: BTreeMap<(String, String), Digest>,
    ) -> Result<Self, WorkspaceError> {
        let root = root.into();
        fs::create_dir_all(&root).map_err(WorkspaceError::io)?;
        let root = root.canonicalize().map_err(WorkspaceError::io)?;
        Ok(Self {
            root,
            available_libraries,
        })
    }

    pub fn list(&self) -> Result<WorkspaceList, WorkspaceError> {
        let mut workspaces = Vec::new();
        for entry in fs::read_dir(&self.root).map_err(WorkspaceError::io)? {
            let entry = entry.map_err(WorkspaceError::io)?;
            if !entry.file_type().map_err(WorkspaceError::io)?.is_dir() {
                continue;
            }
            let path = entry.path();
            if !path.join(MANIFEST_FILE).is_file() {
                continue;
            }
            let manifest = read_manifest_and_migrate(&path.join(MANIFEST_FILE))?;
            let directory_id = path.file_name().and_then(|name| name.to_str());
            if directory_id != Some(manifest.id.as_str()) {
                return Err(WorkspaceError::new(format!(
                    "workspace folder {} does not match stable identity {}",
                    path.display(),
                    manifest.id
                )));
            }
            let issues = dependency_issues(&manifest, &self.available_libraries);
            let dependency_error = (!issues.is_empty()).then(|| format_dependency_issues(&issues));
            let asset_count = if dependency_error.is_none() {
                WorkspaceStore::open(&path, &self.available_libraries)?
                    .list_assets()?
                    .len()
            } else {
                0
            };
            workspaces.push(WorkspaceSummary {
                id: manifest.id,
                label: manifest.label,
                asset_count,
                dependency_error,
            });
        }
        workspaces.sort_by(|left, right| {
            left.label
                .cmp(&right.label)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(WorkspaceList {
            schema: WORKSPACE_LIST_SCHEMA.into(),
            workspaces,
        })
    }

    pub fn create(
        &self,
        request: WorkspaceCreateRequest,
    ) -> Result<WorkspaceRecord, WorkspaceError> {
        if request.schema != WORKSPACE_CREATE_SCHEMA {
            return Err(WorkspaceError::new(
                "workspace create request schema is unsupported",
            ));
        }
        let manifest = WorkspaceManifest::new(request.id, request.label)?;
        let path = self.workspace_path(&manifest.id)?;
        if path.exists() {
            return Err(WorkspaceError::new(format!(
                "workspace {} already exists",
                manifest.id
            )));
        }
        let store = WorkspaceStore::create(path, manifest)?;
        workspace_record(&store)
    }

    pub fn load(&self, id: &str) -> Result<WorkspaceRecord, WorkspaceError> {
        let path = self.workspace_path(id)?;
        if !path.is_dir() {
            return Err(WorkspaceError::new(format!(
                "workspace {id} does not exist"
            )));
        }
        let store = WorkspaceStore::open(path, &self.available_libraries)?;
        workspace_record(&store)
    }

    pub fn export(&self, id: &str) -> Result<WorkspaceExport, WorkspaceError> {
        let store = self.open_workspace(id)?;
        let folders = store
            .list_folders()?
            .into_iter()
            .map(|listing| {
                Ok(WorkspaceExportFolder {
                    relative_path: listing.relative_path.clone(),
                    folder: store.load_folder(&listing.id)?.0,
                })
            })
            .collect::<Result<Vec<_>, WorkspaceError>>()?;
        let assets = store
            .list_assets()?
            .into_iter()
            .map(|listing| {
                let (asset, relative_path) = store.load_asset(&listing.id)?;
                Ok(WorkspaceExportAsset {
                    relative_path,
                    asset,
                })
            })
            .collect::<Result<Vec<_>, WorkspaceError>>()?;
        Ok(WorkspaceExport {
            schema: WORKSPACE_EXPORT_SCHEMA.into(),
            manifest: store.manifest().clone(),
            folders,
            assets,
        })
    }

    pub fn import(&self, bundle: WorkspaceExport) -> Result<WorkspaceRecord, WorkspaceError> {
        if bundle.schema != WORKSPACE_EXPORT_SCHEMA
            && bundle.schema != LEGACY_WORKSPACE_EXPORT_SCHEMA
        {
            return Err(WorkspaceError::new(
                "workspace export schema is unsupported",
            ));
        }
        bundle.manifest.validate()?;
        let issues = dependency_issues(&bundle.manifest, &self.available_libraries);
        if !issues.is_empty() {
            return Err(WorkspaceError::new(format_dependency_issues(&issues)));
        }
        let root = self.workspace_path(&bundle.manifest.id)?;
        if root.exists() {
            return Err(WorkspaceError::new(format!(
                "workspace {} already exists",
                bundle.manifest.id
            )));
        }
        let mut folder_ids = BTreeSet::new();
        let mut folder_paths = BTreeSet::new();
        for record in &bundle.folders {
            record.folder.validate()?;
            validate_folder_relative_path(
                &bundle.manifest,
                record.folder.kind,
                &record.relative_path,
            )?;
            if !folder_ids.insert(record.folder.id.clone()) {
                return Err(WorkspaceError::new(format!(
                    "workspace import duplicates folder identity {}",
                    record.folder.id
                )));
            }
            if !folder_paths.insert(record.relative_path.clone()) {
                return Err(WorkspaceError::new(format!(
                    "workspace import duplicates folder path {}",
                    record.relative_path.display()
                )));
            }
        }
        validate_export_folder_hierarchy(&bundle.manifest, &bundle.folders)?;
        let mut ids = BTreeSet::new();
        let mut paths = BTreeSet::new();
        for record in &bundle.assets {
            record.asset.validate()?;
            validate_relative_path("workspace import asset path", &record.relative_path)?;
            let expected_root = bundle
                .manifest
                .asset_roots
                .get(&record.asset.header.kind)
                .expect("validated manifest contains every asset root");
            if !record.relative_path.starts_with(expected_root)
                || record
                    .relative_path
                    .extension()
                    .and_then(|value| value.to_str())
                    != Some("json")
            {
                return Err(WorkspaceError::new(format!(
                    "{:?} import asset must be a JSON file below {}",
                    record.asset.header.kind, expected_root
                )));
            }
            if !ids.insert(record.asset.header.id.clone()) {
                return Err(WorkspaceError::new(format!(
                    "workspace import duplicates asset identity {}",
                    record.asset.header.id
                )));
            }
            if !paths.insert(record.relative_path.clone()) {
                return Err(WorkspaceError::new(format!(
                    "workspace import duplicates asset path {}",
                    record.relative_path.display()
                )));
            }
        }
        let store = WorkspaceStore::create(&root, bundle.manifest)?;
        if let Err(error) = store.import_folders(&bundle.folders) {
            let cleanup = fs::remove_dir_all(&root);
            return match cleanup {
                Ok(()) => Err(error),
                Err(cleanup) => Err(WorkspaceError::new(format!(
                    "{error}; failed to remove incomplete imported workspace: {cleanup}"
                ))),
            };
        }
        let mutations = bundle
            .assets
            .into_iter()
            .map(|record| WorkspaceMutation::Put {
                relative_path: record.relative_path,
                expected_revision_sha256: None,
                asset: record.asset,
            })
            .collect::<Vec<_>>();
        if let Err(error) = store.transact(&mutations) {
            let cleanup = fs::remove_dir_all(&root);
            return match cleanup {
                Ok(()) => Err(error),
                Err(cleanup) => Err(WorkspaceError::new(format!(
                    "{error}; failed to remove incomplete imported workspace: {cleanup}"
                ))),
            };
        }
        workspace_record(&store)
    }

    pub fn command_folder(
        &self,
        workspace_id: &str,
        folder_id: &str,
        request: WorkspaceFolderCommandRequest,
    ) -> Result<WorkspaceRecord, WorkspaceError> {
        if request.schema != WORKSPACE_FOLDER_COMMAND_SCHEMA {
            return Err(WorkspaceError::new(
                "workspace folder command schema is unsupported",
            ));
        }
        let store = self.open_workspace(workspace_id)?;
        match request.command {
            WorkspaceFolderCommand::Create {
                id,
                label,
                asset_kind,
                parent_id,
                directory_name,
            } => {
                if id != folder_id {
                    return Err(WorkspaceError::new(
                        "URL folder id does not match create command",
                    ));
                }
                store.create_folder(
                    id,
                    label,
                    asset_kind,
                    parent_id.as_deref(),
                    &directory_name,
                )?;
            }
            WorkspaceFolderCommand::Rename {
                expected_revision_sha256,
                label,
                directory_name,
            } => {
                store.rename_folder(folder_id, label, &directory_name, expected_revision_sha256)?;
            }
            WorkspaceFolderCommand::Move {
                expected_revision_sha256,
                parent_id,
            } => {
                store.move_folder(folder_id, parent_id.as_deref(), expected_revision_sha256)?;
            }
            WorkspaceFolderCommand::Duplicate {
                new_id,
                new_label,
                parent_id,
                directory_name,
            } => {
                store.duplicate_folder(
                    folder_id,
                    new_id,
                    new_label,
                    parent_id.as_deref(),
                    &directory_name,
                )?;
            }
            WorkspaceFolderCommand::DeleteToTrash {
                expected_revision_sha256,
                allow_broken_references,
            } => {
                store.delete_folder_to_trash(
                    folder_id,
                    expected_revision_sha256,
                    allow_broken_references,
                )?;
            }
        }
        workspace_record(&store)
    }

    pub fn list_folder_trash(
        &self,
        workspace_id: &str,
    ) -> Result<Vec<WorkspaceFolderTrashListing>, WorkspaceError> {
        self.open_workspace(workspace_id)?.list_folder_trash()
    }

    pub fn command_folder_trash(
        &self,
        workspace_id: &str,
        folder_id: &str,
        request: WorkspaceFolderTrashCommandRequest,
    ) -> Result<WorkspaceRecord, WorkspaceError> {
        if request.schema != WORKSPACE_FOLDER_TRASH_COMMAND_SCHEMA {
            return Err(WorkspaceError::new(
                "workspace folder trash command schema is unsupported",
            ));
        }
        let store = self.open_workspace(workspace_id)?;
        match request.command {
            WorkspaceTrashCommand::Restore => {
                store.restore_folder_from_trash(folder_id, request.expected_revision_sha256)?;
            }
            WorkspaceTrashCommand::PermanentlyDelete => {
                store.permanently_delete_folder_from_trash(
                    folder_id,
                    request.expected_revision_sha256,
                )?;
            }
        }
        workspace_record(&store)
    }

    pub fn load_asset(
        &self,
        workspace_id: &str,
        asset_id: &str,
    ) -> Result<WorkspaceAssetRecord, WorkspaceError> {
        let store = self.open_workspace(workspace_id)?;
        let (asset, relative_path) = store.load_asset(asset_id)?;
        let revision_sha256 = asset.digest()?;
        Ok(WorkspaceAssetRecord {
            schema: WORKSPACE_ASSET_RECORD_SCHEMA.into(),
            relative_path,
            revision_sha256,
            asset,
        })
    }

    pub fn save_asset(
        &self,
        workspace_id: &str,
        asset_id: &str,
        request: WorkspaceAssetSaveRequest,
    ) -> Result<WorkspaceAssetRecord, WorkspaceError> {
        if request.schema != WORKSPACE_ASSET_SAVE_SCHEMA {
            return Err(WorkspaceError::new(
                "workspace asset save request schema is unsupported",
            ));
        }
        if request.asset.header.id != asset_id {
            return Err(WorkspaceError::new(
                "URL asset id does not match the document",
            ));
        }
        let store = self.open_workspace(workspace_id)?;
        let revision_sha256 = store.save_asset(
            &request.relative_path,
            request.expected_revision_sha256,
            &request.asset,
        )?;
        Ok(WorkspaceAssetRecord {
            schema: WORKSPACE_ASSET_RECORD_SCHEMA.into(),
            relative_path: request.relative_path,
            revision_sha256,
            asset: request.asset,
        })
    }

    pub fn save_route_graph(
        &self,
        workspace_id: &str,
        graph_id: &str,
        request: WorkspaceRouteGraphSaveRequest,
        catalog: &ComposedPlannerCatalog,
    ) -> Result<WorkspaceRouteGraphEditRecord, WorkspaceError> {
        if request.schema != WORKSPACE_ROUTE_GRAPH_SAVE_SCHEMA {
            return Err(WorkspaceError::new(
                "workspace route-graph save request schema is unsupported",
            ));
        }
        catalog.validate()?;
        request.route_book.validate_against_composed(catalog)?;
        let store = self.open_workspace(workspace_id)?;
        let (mut graph_asset, graph_path) = store.load_asset(graph_id)?;
        if graph_asset.header.kind != WorkspaceAssetKind::RouteGraph {
            return Err(WorkspaceError::new(format!(
                "asset {graph_id} is not a route graph"
            )));
        }
        if graph_asset.digest()? != request.expected_graph_revision_sha256 {
            return Err(WorkspaceError::new(
                "route graph revision conflict before save",
            ));
        }
        if !graph_asset.references.iter().any(|reference| {
            reference.asset_id == request.route_book_id
                && reference.kind == WorkspaceAssetKind::RouteBook
        }) {
            return Err(WorkspaceError::new(format!(
                "route graph {graph_id} does not reference route book {}",
                request.route_book_id
            )));
        }
        let (mut route_book_asset, route_book_path) = store.load_asset(&request.route_book_id)?;
        if route_book_asset.header.kind != WorkspaceAssetKind::RouteBook {
            return Err(WorkspaceError::new(format!(
                "asset {} is not a route book",
                request.route_book_id
            )));
        }
        if route_book_asset.digest()? != request.expected_route_book_revision_sha256 {
            return Err(WorkspaceError::new(
                "route book revision conflict before save",
            ));
        }
        graph_asset.payload = WorkspaceAssetPayload::RouteGraph {
            graph: PlannerGraph::project_composed_with_route_book(catalog, &request.route_book)?,
        };
        route_book_asset.payload = WorkspaceAssetPayload::RouteBook {
            route_book: request.route_book,
        };

        let mut mutations = vec![
            WorkspaceMutation::Put {
                relative_path: graph_path.clone(),
                expected_revision_sha256: Some(request.expected_graph_revision_sha256),
                asset: graph_asset.clone(),
            },
            WorkspaceMutation::Put {
                relative_path: route_book_path.clone(),
                expected_revision_sha256: Some(request.expected_route_book_revision_sha256),
                asset: route_book_asset.clone(),
            },
        ];
        let mut saved_layout = None;
        if let Some(edit) = request.layout {
            let (mut asset, path) = store.load_asset(&edit.asset_id)?;
            if asset.digest()? != edit.expected_revision_sha256 {
                return Err(WorkspaceError::new("layout revision conflict before save"));
            }
            let WorkspaceAssetPayload::Layout(layout) = &mut asset.payload else {
                return Err(WorkspaceError::new(format!(
                    "asset {} is not a layout",
                    edit.asset_id
                )));
            };
            if layout.semantic_asset_id != graph_id {
                return Err(WorkspaceError::new(format!(
                    "layout {} presents {}, not route graph {graph_id}",
                    edit.asset_id, layout.semantic_asset_id
                )));
            }
            layout.positions = edit.positions;
            layout.viewport = edit.viewport;
            mutations.push(WorkspaceMutation::Put {
                relative_path: path.clone(),
                expected_revision_sha256: Some(edit.expected_revision_sha256),
                asset: asset.clone(),
            });
            saved_layout = Some((asset, path));
        }
        store.transact(&mutations)?;

        let asset_record = |asset: WorkspaceAsset,
                            relative_path: PathBuf|
         -> Result<WorkspaceAssetRecord, WorkspaceError> {
            Ok(WorkspaceAssetRecord {
                schema: WORKSPACE_ASSET_RECORD_SCHEMA.into(),
                revision_sha256: asset.digest()?,
                relative_path,
                asset,
            })
        };
        Ok(WorkspaceRouteGraphEditRecord {
            schema: WORKSPACE_ROUTE_GRAPH_EDIT_RECORD_SCHEMA.into(),
            workspace: workspace_record(&store)?,
            graph: asset_record(graph_asset, graph_path)?,
            route_book: asset_record(route_book_asset, route_book_path)?,
            layout: saved_layout
                .map(|(asset, path)| asset_record(asset, path))
                .transpose()?,
        })
    }

    pub fn command_asset(
        &self,
        workspace_id: &str,
        asset_id: &str,
        request: WorkspaceAssetCommandRequest,
    ) -> Result<WorkspaceRecord, WorkspaceError> {
        if request.schema != WORKSPACE_ASSET_COMMAND_SCHEMA {
            return Err(WorkspaceError::new(
                "workspace asset command schema is unsupported",
            ));
        }
        let store = self.open_workspace(workspace_id)?;
        match request.command {
            WorkspaceAssetCommand::Rename {
                expected_revision_sha256,
                label,
            } => {
                store.rename_asset(asset_id, label, expected_revision_sha256)?;
            }
            WorkspaceAssetCommand::Move {
                expected_revision_sha256,
                relative_path,
            } => {
                store.move_asset(asset_id, &relative_path, expected_revision_sha256)?;
            }
            WorkspaceAssetCommand::Duplicate {
                new_id,
                new_label,
                relative_path,
            } => {
                store.duplicate_asset(asset_id, new_id, new_label, &relative_path)?;
            }
            WorkspaceAssetCommand::DeleteToTrash {
                expected_revision_sha256,
                allow_broken_references,
            } => {
                store.delete_to_trash(
                    asset_id,
                    expected_revision_sha256,
                    allow_broken_references,
                )?;
            }
        }
        workspace_record(&store)
    }

    pub fn list_trash(
        &self,
        workspace_id: &str,
    ) -> Result<Vec<WorkspaceTrashListing>, WorkspaceError> {
        self.open_workspace(workspace_id)?.list_trash()
    }

    pub fn command_trash(
        &self,
        workspace_id: &str,
        asset_id: &str,
        request: WorkspaceTrashCommandRequest,
    ) -> Result<WorkspaceRecord, WorkspaceError> {
        if request.schema != WORKSPACE_TRASH_COMMAND_SCHEMA {
            return Err(WorkspaceError::new(
                "workspace trash command schema is unsupported",
            ));
        }
        let store = self.open_workspace(workspace_id)?;
        match request.command {
            WorkspaceTrashCommand::Restore => {
                store.restore_from_trash(asset_id, request.expected_revision_sha256)?;
            }
            WorkspaceTrashCommand::PermanentlyDelete => {
                store.permanently_delete_from_trash(asset_id, request.expected_revision_sha256)?;
            }
        }
        workspace_record(&store)
    }

    pub fn create_scenario_from_library(
        &self,
        workspace_id: &str,
        project: &PlannerWebProject,
        library_sha256: Digest,
    ) -> Result<WorkspaceRecord, WorkspaceError> {
        let mut store = self.open_workspace(workspace_id)?;
        store.import_project_template(project, library_sha256)?;
        workspace_record(&store)
    }

    pub fn add_library_reference(
        &self,
        workspace_id: &str,
        project: &PlannerWebProject,
        library_sha256: Digest,
    ) -> Result<WorkspaceRecord, WorkspaceError> {
        project
            .validate()
            .map_err(|error| WorkspaceError::new(error.to_string()))?;
        if library_sha256 == Digest::ZERO {
            return Err(WorkspaceError::new("library digest must be nonzero"));
        }
        let exact_context = project
            .start_state
            .as_ref()
            .ok_or_else(|| WorkspaceError::new("Library has no exact context to reference"))?
            .snapshot
            .environment
            .runtime_configuration
            .exact_context()?;
        let mut store = self.open_workspace(workspace_id)?;
        store.mount_library(
            MountedLibrary {
                id: project.id.clone(),
                version: BUILTIN_LIBRARY_VERSION.into(),
                sha256: library_sha256,
                source: format!("builtin:{}", project.id),
            },
            exact_context,
        )?;
        workspace_record(&store)
    }

    pub fn fork_library(
        &self,
        workspace_id: &str,
        project: &PlannerWebProject,
        library_sha256: Digest,
        request: WorkspaceLibraryForkRequest,
    ) -> Result<WorkspaceRecord, WorkspaceError> {
        if request.schema != WORKSPACE_LIBRARY_FORK_SCHEMA {
            return Err(WorkspaceError::new(
                "workspace Library fork schema is unsupported",
            ));
        }
        let mut store = self.open_workspace(workspace_id)?;
        store.fork_project_template(project, library_sha256, &request.namespace)?;
        workspace_record(&store)
    }

    fn open_workspace(&self, id: &str) -> Result<WorkspaceStore, WorkspaceError> {
        let path = self.workspace_path(id)?;
        if !path.is_dir() {
            return Err(WorkspaceError::new(format!(
                "workspace {id} does not exist"
            )));
        }
        WorkspaceStore::open(path, &self.available_libraries)
    }

    fn workspace_path(&self, id: &str) -> Result<PathBuf, WorkspaceError> {
        validate_stable_id("workspace id", id)?;
        if id.contains('/') || id.contains(':') {
            return Err(WorkspaceError::new(
                "workspace id used as a folder cannot contain '/' or ':'",
            ));
        }
        Ok(self.root.join(id))
    }
}

fn workspace_record(store: &WorkspaceStore) -> Result<WorkspaceRecord, WorkspaceError> {
    Ok(WorkspaceRecord {
        schema: WORKSPACE_RECORD_SCHEMA.into(),
        manifest: store.manifest().clone(),
        folders: store.list_folders()?,
        assets: store.list_assets()?,
    })
}

impl WorkspaceStore {
    pub fn create(
        root: impl Into<PathBuf>,
        manifest: WorkspaceManifest,
    ) -> Result<Self, WorkspaceError> {
        manifest.validate()?;
        let root = root.into();
        if root.exists()
            && fs::read_dir(&root)
                .map_err(WorkspaceError::io)?
                .next()
                .is_some()
        {
            return Err(WorkspaceError::new(format!(
                "workspace folder {} is not empty",
                root.display()
            )));
        }
        fs::create_dir_all(&root).map_err(WorkspaceError::io)?;
        for asset_root in manifest.asset_roots.values() {
            fs::create_dir_all(root.join(asset_root)).map_err(WorkspaceError::io)?;
        }
        write_atomically(&root.join(MANIFEST_FILE), &manifest.canonical_bytes()?)?;
        let root = root.canonicalize().map_err(WorkspaceError::io)?;
        let store = Self { root, manifest };
        store.ensure_transaction_root()?;
        Ok(store)
    }

    pub fn open(
        root: impl Into<PathBuf>,
        available_libraries: &BTreeMap<(String, String), Digest>,
    ) -> Result<Self, WorkspaceError> {
        let root = root.into().canonicalize().map_err(WorkspaceError::io)?;
        let manifest = read_manifest_and_migrate(&root.join(MANIFEST_FILE))?;
        let issues = dependency_issues(&manifest, available_libraries);
        if !issues.is_empty() {
            return Err(WorkspaceError::new(format_dependency_issues(&issues)));
        }
        let store = Self { root, manifest };
        store.ensure_transaction_root()?;
        store.recover_transactions()?;
        store.recover_folder_operations()?;
        Ok(store)
    }

    pub fn manifest(&self) -> &WorkspaceManifest {
        &self.manifest
    }

    pub fn list_assets(&self) -> Result<Vec<WorkspaceAssetListing>, WorkspaceError> {
        let mut listings = Vec::new();
        let mut identities = BTreeMap::new();
        for kind in WorkspaceAssetKind::ALL {
            let root = self.asset_root(kind)?;
            collect_asset_files(&root, &mut |path| {
                let asset = read_asset(path)?;
                if asset.header.kind != kind {
                    return Err(WorkspaceError::new(format!(
                        "{} contains {:?} asset {} under the {:?} root",
                        path.display(),
                        asset.header.kind,
                        asset.header.id,
                        kind
                    )));
                }
                if let Some(first) = identities.insert(asset.header.id.clone(), path.to_path_buf())
                {
                    return Err(WorkspaceError::new(format!(
                        "asset identity {} is duplicated at {} and {}",
                        asset.header.id,
                        first.display(),
                        path.display()
                    )));
                }
                listings.push(WorkspaceAssetListing {
                    id: asset.header.id.clone(),
                    label: asset.header.label.clone(),
                    kind,
                    relative_path: path
                        .strip_prefix(&self.root)
                        .map_err(|_| WorkspaceError::new("asset escaped workspace root"))?
                        .to_path_buf(),
                    revision_sha256: asset.digest()?,
                });
                Ok(())
            })?;
        }
        listings.sort_by(|left, right| {
            left.kind
                .cmp(&right.kind)
                .then_with(|| left.label.cmp(&right.label))
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(listings)
    }

    pub fn list_folders(&self) -> Result<Vec<WorkspaceFolderListing>, WorkspaceError> {
        let mut folders = Vec::new();
        let mut identities = BTreeMap::new();
        let mut paths = BTreeMap::new();
        for kind in WorkspaceAssetKind::ALL {
            let root = self.asset_root(kind)?;
            collect_folder_markers(&root, &mut |directory, marker| {
                let folder = read_folder(marker)?;
                if folder.kind != kind {
                    return Err(WorkspaceError::new(format!(
                        "{} declares {:?} below the {:?} root",
                        marker.display(),
                        folder.kind,
                        kind
                    )));
                }
                let relative_path = directory
                    .strip_prefix(&self.root)
                    .map_err(|_| WorkspaceError::new("folder escaped workspace root"))?
                    .to_path_buf();
                if let Some(first) = identities.insert(folder.id.clone(), relative_path.clone()) {
                    return Err(WorkspaceError::new(format!(
                        "folder identity {} is duplicated at {} and {}",
                        folder.id,
                        first.display(),
                        relative_path.display()
                    )));
                }
                paths.insert(relative_path.clone(), folder.id.clone());
                let revision_sha256 = folder.digest()?;
                folders.push(WorkspaceFolderListing {
                    id: folder.id,
                    label: folder.label,
                    kind,
                    parent_id: folder.parent_id,
                    relative_path,
                    revision_sha256,
                });
                Ok(())
            })?;
        }
        for folder in &folders {
            let expected_parent_path = folder
                .relative_path
                .parent()
                .ok_or_else(|| WorkspaceError::new("folder has no parent directory"))?;
            let asset_root = Path::new(
                self.manifest
                    .asset_roots
                    .get(&folder.kind)
                    .expect("validated manifest has every root"),
            );
            match &folder.parent_id {
                Some(parent_id) => {
                    let actual_parent_id = paths.get(expected_parent_path).ok_or_else(|| {
                        WorkspaceError::new(format!(
                            "folder {} names parent {parent_id}, but {} has no folder marker",
                            folder.id,
                            expected_parent_path.display()
                        ))
                    })?;
                    if actual_parent_id != parent_id {
                        return Err(WorkspaceError::new(format!(
                            "folder {} names parent {parent_id}, but its containing folder is {actual_parent_id}",
                            folder.id
                        )));
                    }
                }
                None if expected_parent_path != asset_root => {
                    return Err(WorkspaceError::new(format!(
                        "folder {} has no parent identity but is not directly below its fixed root",
                        folder.id
                    )));
                }
                None => {}
            }
        }
        folders.sort_by(|left, right| {
            left.kind
                .cmp(&right.kind)
                .then_with(|| left.relative_path.cmp(&right.relative_path))
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(folders)
    }

    pub fn load_folder(&self, id: &str) -> Result<(WorkspaceFolder, PathBuf), WorkspaceError> {
        validate_stable_id("folder id", id)?;
        let listing = self
            .list_folders()?
            .into_iter()
            .find(|listing| listing.id == id)
            .ok_or_else(|| WorkspaceError::new(format!("folder {id} does not exist")))?;
        Ok((
            read_folder(
                &self
                    .root
                    .join(&listing.relative_path)
                    .join(FOLDER_MARKER_FILE),
            )?,
            listing.relative_path,
        ))
    }

    pub fn create_folder(
        &self,
        id: impl Into<String>,
        label: impl Into<String>,
        kind: WorkspaceAssetKind,
        parent_id: Option<&str>,
        directory_name: &str,
    ) -> Result<Digest, WorkspaceError> {
        let folder = WorkspaceFolder {
            schema: WORKSPACE_FOLDER_SCHEMA.into(),
            id: id.into(),
            label: label.into(),
            kind,
            parent_id: parent_id.map(str::to_owned),
        };
        folder.validate()?;
        if self
            .list_folders()?
            .iter()
            .any(|existing| existing.id == folder.id)
        {
            return Err(WorkspaceError::new(format!(
                "folder identity {} already exists",
                folder.id
            )));
        }
        validate_directory_name(directory_name)?;
        let parent = self.folder_parent_path(kind, parent_id)?;
        let destination = parent.join(directory_name);
        if destination.exists() {
            return Err(WorkspaceError::new(format!(
                "folder path {} already exists",
                destination.display()
            )));
        }
        let staging = self.root.join(FOLDER_STAGING_ROOT).join(format!(
            "folder-{}-{}",
            std::process::id(),
            NEXT_TEMPORARY_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&staging).map_err(WorkspaceError::io)?;
        let result = (|| {
            write_new_synced(
                &staging.join(FOLDER_MARKER_FILE),
                &folder.canonical_bytes()?,
            )?;
            fs::rename(&staging, &destination).map_err(WorkspaceError::io)
        })();
        if let Err(error) = result {
            if staging.is_dir() {
                let _ = fs::remove_dir_all(&staging);
            }
            return Err(error);
        }
        folder.digest()
    }

    pub fn rename_folder(
        &self,
        id: &str,
        label: impl Into<String>,
        directory_name: &str,
        expected_revision_sha256: Digest,
    ) -> Result<Digest, WorkspaceError> {
        let (mut folder, path) = self.load_folder(id)?;
        if folder.digest()? != expected_revision_sha256 {
            return Err(WorkspaceError::new(
                "folder revision conflict before rename",
            ));
        }
        folder.label = label.into();
        validate_directory_name(directory_name)?;
        let bytes = folder.canonical_bytes()?;
        let source = self.root.join(path);
        let destination = source
            .parent()
            .ok_or_else(|| WorkspaceError::new("folder path has no parent"))?
            .join(directory_name);
        if source == destination {
            write_atomically(&source.join(FOLDER_MARKER_FILE), &bytes)?;
        } else {
            if destination.exists() {
                return Err(WorkspaceError::new(format!(
                    "folder path {} already exists",
                    destination.display()
                )));
            }
            fs::rename(&source, &destination).map_err(WorkspaceError::io)?;
            if let Err(error) = write_atomically(&destination.join(FOLDER_MARKER_FILE), &bytes) {
                let _ = fs::rename(&destination, &source);
                return Err(error);
            }
        }
        folder.digest()
    }

    pub fn move_folder(
        &self,
        id: &str,
        parent_id: Option<&str>,
        expected_revision_sha256: Digest,
    ) -> Result<Digest, WorkspaceError> {
        let (mut folder, source_relative) = self.load_folder(id)?;
        if folder.digest()? != expected_revision_sha256 {
            return Err(WorkspaceError::new("folder revision conflict before move"));
        }
        if parent_id == Some(id) {
            return Err(WorkspaceError::new("folder cannot be moved into itself"));
        }
        let destination_parent = self.folder_parent_path(folder.kind, parent_id)?;
        let source = self.root.join(&source_relative);
        if destination_parent.starts_with(&source) {
            return Err(WorkspaceError::new(
                "folder cannot be moved into its own descendant",
            ));
        }
        let name = source
            .file_name()
            .ok_or_else(|| WorkspaceError::new("folder path has no directory name"))?;
        let destination = destination_parent.join(name);
        if destination == source {
            return folder.digest();
        }
        if destination.exists() {
            return Err(WorkspaceError::new(format!(
                "folder path {} already exists",
                destination.display()
            )));
        }
        fs::rename(&source, &destination).map_err(WorkspaceError::io)?;
        folder.parent_id = parent_id.map(str::to_owned);
        if let Err(error) = write_atomically(
            &destination.join(FOLDER_MARKER_FILE),
            &folder.canonical_bytes()?,
        ) {
            let _ = fs::rename(&destination, &source);
            return Err(error);
        }
        folder.digest()
    }

    pub fn duplicate_folder(
        &self,
        id: &str,
        new_id: impl Into<String>,
        new_label: impl Into<String>,
        parent_id: Option<&str>,
        directory_name: &str,
    ) -> Result<Digest, WorkspaceError> {
        let (source_folder, source_relative) = self.load_folder(id)?;
        let new_id = new_id.into();
        let new_label = new_label.into();
        validate_stable_id("duplicated folder id", &new_id)?;
        validate_label("duplicated folder label", &new_label)?;
        validate_directory_name(directory_name)?;
        if self
            .list_folders()?
            .iter()
            .any(|folder| folder.id == new_id)
        {
            return Err(WorkspaceError::new(format!(
                "folder identity {new_id} already exists"
            )));
        }
        let destination_parent = self.folder_parent_path(source_folder.kind, parent_id)?;
        let destination = destination_parent.join(directory_name);
        if destination.exists() {
            return Err(WorkspaceError::new(format!(
                "folder path {} already exists",
                destination.display()
            )));
        }
        validate_folder_subtree_files(&self.root.join(&source_relative))?;

        let source_folders = self
            .list_folders()?
            .into_iter()
            .filter(|folder| folder.relative_path.starts_with(&source_relative))
            .collect::<Vec<_>>();
        let source_assets = self
            .list_assets()?
            .into_iter()
            .filter(|asset| asset.relative_path.starts_with(&source_relative))
            .collect::<Vec<_>>();
        let existing_folder_ids = self
            .list_folders()?
            .into_iter()
            .map(|folder| folder.id)
            .collect::<BTreeSet<_>>();
        let existing_asset_ids = self
            .list_assets()?
            .into_iter()
            .map(|asset| asset.id)
            .collect::<BTreeSet<_>>();
        let folder_id_map = clone_identity_map(
            &source_folders
                .iter()
                .map(|folder| &folder.id)
                .collect::<Vec<_>>(),
            &new_id,
            "folder",
            &existing_folder_ids,
            Some(id),
        )?;
        let asset_id_map = clone_identity_map(
            &source_assets
                .iter()
                .map(|asset| &asset.id)
                .collect::<Vec<_>>(),
            &new_id,
            "asset",
            &existing_asset_ids,
            None,
        )?;
        let staging_root = self.root.join(FOLDER_STAGING_ROOT);
        fs::create_dir_all(&staging_root).map_err(WorkspaceError::io)?;
        let staging = staging_root.join(format!(
            "folder-{}-{}",
            std::process::id(),
            NEXT_TEMPORARY_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let mut cloned_assets = Vec::new();
        for listing in &source_assets {
            let (mut asset, _) = self.load_asset(&listing.id)?;
            remap_cloned_asset(&mut asset, &asset_id_map);
            cloned_assets.push((listing, asset));
        }
        let mut available = self
            .list_assets()?
            .into_iter()
            .map(|asset| (asset.id, asset.kind))
            .collect::<BTreeMap<_, _>>();
        available.extend(
            cloned_assets
                .iter()
                .map(|(_, asset)| (asset.header.id.clone(), asset.header.kind)),
        );
        for (_, asset) in &cloned_assets {
            validate_asset_references(asset, &available)?;
        }
        fs::create_dir(&staging).map_err(WorkspaceError::io)?;

        let build = (|| {
            for listing in &source_folders {
                let (mut folder, _) = self.load_folder(&listing.id)?;
                folder.id = folder_id_map
                    .get(&folder.id)
                    .expect("every cloned folder has an identity")
                    .clone();
                folder.label = if listing.id == id {
                    new_label.clone()
                } else {
                    folder.label
                };
                folder.parent_id = if listing.id == id {
                    parent_id.map(str::to_owned)
                } else {
                    folder
                        .parent_id
                        .as_ref()
                        .and_then(|parent| folder_id_map.get(parent))
                        .cloned()
                };
                let relative = listing
                    .relative_path
                    .strip_prefix(&source_relative)
                    .map_err(|_| WorkspaceError::new("cloned folder escaped source"))?;
                let target = staging.join(relative);
                fs::create_dir_all(&target).map_err(WorkspaceError::io)?;
                write_new_synced(&target.join(FOLDER_MARKER_FILE), &folder.canonical_bytes()?)?;
            }
            for (listing, asset) in &cloned_assets {
                let relative = listing
                    .relative_path
                    .strip_prefix(&source_relative)
                    .map_err(|_| WorkspaceError::new("cloned asset escaped source"))?;
                let target = staging.join(relative);
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent).map_err(WorkspaceError::io)?;
                }
                write_new_synced(&target, &asset.canonical_bytes()?)?;
            }
            fs::rename(&staging, &destination).map_err(WorkspaceError::io)
        })();
        if let Err(error) = build {
            if staging.is_dir() {
                let _ = fs::remove_dir_all(&staging);
            }
            return Err(error);
        }
        let (folder, _) = self.load_folder(&new_id)?;
        folder.digest()
    }

    pub fn delete_folder_to_trash(
        &self,
        id: &str,
        expected_revision_sha256: Digest,
        allow_broken_references: bool,
    ) -> Result<(), WorkspaceError> {
        let (folder, source_relative) = self.load_folder(id)?;
        if folder.digest()? != expected_revision_sha256 {
            return Err(WorkspaceError::new(
                "folder revision conflict before delete",
            ));
        }
        let subtree_assets = self
            .list_assets()?
            .into_iter()
            .filter(|asset| asset.relative_path.starts_with(&source_relative))
            .collect::<Vec<_>>();
        let subtree_ids = subtree_assets
            .iter()
            .map(|asset| asset.id.as_str())
            .collect::<BTreeSet<_>>();
        let mut external_inbound = BTreeSet::new();
        for listing in self.list_assets()? {
            if subtree_ids.contains(listing.id.as_str()) {
                continue;
            }
            let asset = read_asset(&self.root.join(&listing.relative_path))?;
            if asset
                .references
                .iter()
                .any(|reference| subtree_ids.contains(reference.asset_id.as_str()))
            {
                external_inbound.insert(listing.id);
            }
        }
        if !allow_broken_references && !external_inbound.is_empty() {
            return Err(WorkspaceError::new(format!(
                "folder {id} contains assets referenced by {}; confirm deletion to preserve these as broken stable-ID references",
                external_inbound.into_iter().collect::<Vec<_>>().join(", ")
            )));
        }
        let folder_count = self
            .list_folders()?
            .into_iter()
            .filter(|child| child.relative_path.starts_with(&source_relative))
            .count();
        let group = self.folder_trash_group(id)?;
        if group.exists() {
            return Err(WorkspaceError::new(format!(
                "folder {id} already has a grouped Trash entry"
            )));
        }
        fs::create_dir(&group).map_err(WorkspaceError::io)?;
        let record = WorkspaceFolderTrashRecord {
            schema: FOLDER_TRASH_RECORD_SCHEMA.into(),
            folder,
            original_relative_path: source_relative.clone(),
            revision_sha256: expected_revision_sha256,
            folder_count,
            asset_count: subtree_assets.len(),
        };
        if let Err(error) = write_new_synced(
            &group.join(FOLDER_TRASH_RECORD_FILE),
            &canonical_json(&record)?,
        ) {
            let _ = fs::remove_dir(&group);
            return Err(error);
        }
        if let Err(error) = fs::rename(
            self.root.join(&source_relative),
            group.join(FOLDER_TRASH_PAYLOAD),
        ) {
            let _ = fs::remove_file(group.join(FOLDER_TRASH_RECORD_FILE));
            let _ = fs::remove_dir(&group);
            return Err(WorkspaceError::io(error));
        }
        Ok(())
    }

    pub fn list_folder_trash(&self) -> Result<Vec<WorkspaceFolderTrashListing>, WorkspaceError> {
        let root = self.root.join(FOLDER_TRASH_ROOT);
        let mut trash = Vec::new();
        for entry in fs::read_dir(&root).map_err(WorkspaceError::io)? {
            let entry = entry.map_err(WorkspaceError::io)?;
            if !entry.file_type().map_err(WorkspaceError::io)?.is_dir() {
                return Err(WorkspaceError::new(format!(
                    "unexpected file in grouped folder Trash: {}",
                    entry.path().display()
                )));
            }
            let record = self.read_folder_trash_record(&entry.path())?;
            if entry.file_name().to_string_lossy() != folder_trash_directory_name(&record.folder.id)
                || !entry.path().join(FOLDER_TRASH_PAYLOAD).is_dir()
            {
                return Err(WorkspaceError::new(format!(
                    "grouped folder Trash entry {} is invalid",
                    entry.path().display()
                )));
            }
            trash.push(WorkspaceFolderTrashListing {
                id: record.folder.id,
                label: record.folder.label,
                kind: record.folder.kind,
                original_relative_path: record.original_relative_path,
                revision_sha256: record.revision_sha256,
                folder_count: record.folder_count,
                asset_count: record.asset_count,
            });
        }
        trash.sort_by(|left, right| {
            left.kind
                .cmp(&right.kind)
                .then_with(|| left.label.cmp(&right.label))
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(trash)
    }

    pub fn restore_folder_from_trash(
        &self,
        id: &str,
        expected_revision_sha256: Digest,
    ) -> Result<(), WorkspaceError> {
        let group = self.folder_trash_group(id)?;
        let record = self.read_folder_trash_record(&group)?;
        if record.folder.id != id || record.revision_sha256 != expected_revision_sha256 {
            return Err(WorkspaceError::new(
                "folder Trash revision conflict before restore",
            ));
        }
        let destination = self.folder_restore_destination(&record)?;
        if destination.exists() {
            return Err(WorkspaceError::new(format!(
                "cannot restore folder {id}; {} already exists",
                destination
                    .strip_prefix(&self.root)
                    .unwrap_or(&destination)
                    .display()
            )));
        }
        fs::rename(group.join(FOLDER_TRASH_PAYLOAD), &destination).map_err(WorkspaceError::io)?;
        fs::remove_file(group.join(FOLDER_TRASH_RECORD_FILE)).map_err(WorkspaceError::io)?;
        fs::remove_dir(group).map_err(WorkspaceError::io)
    }

    pub fn permanently_delete_folder_from_trash(
        &self,
        id: &str,
        expected_revision_sha256: Digest,
    ) -> Result<(), WorkspaceError> {
        let group = self.folder_trash_group(id)?;
        let record = self.read_folder_trash_record(&group)?;
        if record.folder.id != id || record.revision_sha256 != expected_revision_sha256 {
            return Err(WorkspaceError::new(
                "folder Trash revision conflict before permanent delete",
            ));
        }
        let trash_root = self.root.join(FOLDER_TRASH_ROOT);
        if group.parent() != Some(trash_root.as_path()) || !group.is_dir() {
            return Err(WorkspaceError::new(
                "refusing to remove an invalid grouped folder Trash path",
            ));
        }
        fs::remove_dir_all(group).map_err(WorkspaceError::io)
    }

    fn folder_parent_path(
        &self,
        kind: WorkspaceAssetKind,
        parent_id: Option<&str>,
    ) -> Result<PathBuf, WorkspaceError> {
        match parent_id {
            Some(id) => {
                let (parent, path) = self.load_folder(id)?;
                if parent.kind != kind {
                    return Err(WorkspaceError::new(format!(
                        "{kind:?} folder cannot be placed below {:?} folder {id}",
                        parent.kind
                    )));
                }
                Ok(self.root.join(path))
            }
            None => self.asset_root(kind),
        }
    }

    fn folder_trash_group(&self, id: &str) -> Result<PathBuf, WorkspaceError> {
        validate_stable_id("folder id", id)?;
        Ok(self
            .root
            .join(FOLDER_TRASH_ROOT)
            .join(folder_trash_directory_name(id)))
    }

    fn read_folder_trash_record(
        &self,
        group: &Path,
    ) -> Result<WorkspaceFolderTrashRecord, WorkspaceError> {
        let record = read_folder_trash_record(group)?;
        validate_folder_relative_path(
            &self.manifest,
            record.folder.kind,
            &record.original_relative_path,
        )?;
        Ok(record)
    }

    fn folder_restore_destination(
        &self,
        record: &WorkspaceFolderTrashRecord,
    ) -> Result<PathBuf, WorkspaceError> {
        let directory_name = record
            .original_relative_path
            .file_name()
            .ok_or_else(|| WorkspaceError::new("trashed folder path has no directory name"))?;
        Ok(self
            .folder_parent_path(record.folder.kind, record.folder.parent_id.as_deref())?
            .join(directory_name))
    }

    fn import_folders(&self, folders: &[WorkspaceExportFolder]) -> Result<(), WorkspaceError> {
        let mut sorted = folders.to_vec();
        sorted.sort_by_key(|folder| folder.relative_path.components().count());
        for record in sorted {
            let path = self.root.join(&record.relative_path);
            fs::create_dir_all(&path).map_err(WorkspaceError::io)?;
            write_new_synced(
                &path.join(FOLDER_MARKER_FILE),
                &record.folder.canonical_bytes()?,
            )?;
        }
        self.list_folders().map(|_| ())
    }

    pub fn load_asset(&self, id: &str) -> Result<(WorkspaceAsset, PathBuf), WorkspaceError> {
        validate_stable_id("asset id", id)?;
        let listing = self
            .list_assets()?
            .into_iter()
            .find(|listing| listing.id == id)
            .ok_or_else(|| WorkspaceError::new(format!("asset {id} does not exist")))?;
        let path = self.root.join(&listing.relative_path);
        Ok((read_asset(&path)?, listing.relative_path))
    }

    pub fn save_asset(
        &self,
        relative_path: &Path,
        expected_revision_sha256: Option<Digest>,
        asset: &WorkspaceAsset,
    ) -> Result<Digest, WorkspaceError> {
        asset.validate()?;
        self.validate_asset_path(relative_path, asset.header.kind)?;
        if self
            .list_assets()?
            .iter()
            .any(|listing| listing.id == asset.header.id && listing.relative_path != relative_path)
        {
            return Err(WorkspaceError::new(format!(
                "asset identity {} already exists at another path",
                asset.header.id
            )));
        }
        let mut available = self
            .list_assets()?
            .into_iter()
            .map(|listing| (listing.id, listing.kind))
            .collect::<BTreeMap<_, _>>();
        available.insert(asset.header.id.clone(), asset.header.kind);
        validate_asset_references(asset, &available)?;
        let path = self.root.join(relative_path);
        let current_revision = path
            .is_file()
            .then(|| read_asset(&path).and_then(|current| current.digest()))
            .transpose()?;
        if current_revision != expected_revision_sha256 {
            return Err(WorkspaceError::new(format!(
                "asset revision conflict: expected {}, current {}",
                display_digest(expected_revision_sha256),
                display_digest(current_revision)
            )));
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(WorkspaceError::io)?;
        }
        write_atomically(&path, &asset.canonical_bytes()?)?;
        asset.digest()
    }

    /// Durably commits a set of asset writes and deletes as one recoverable unit.
    ///
    /// A crash can interrupt the visible filesystem updates, but the prepared
    /// journal is replayed on the next open before assets are returned.
    pub fn transact(&self, mutations: &[WorkspaceMutation]) -> Result<(), WorkspaceError> {
        if mutations.is_empty() {
            return Err(WorkspaceError::new(
                "workspace transaction must contain at least one mutation",
            ));
        }
        self.validate_mutations(mutations)?;
        let id = format!(
            "transaction-{}-{}",
            std::process::id(),
            NEXT_TEMPORARY_ID.fetch_add(1, Ordering::Relaxed)
        );
        let transaction_root = self.root.join(TRANSACTION_ROOT).join(&id);
        fs::create_dir(&transaction_root).map_err(WorkspaceError::io)?;
        let mut operations = Vec::with_capacity(mutations.len());
        for (index, mutation) in mutations.iter().enumerate() {
            match mutation {
                WorkspaceMutation::Put {
                    relative_path,
                    expected_revision_sha256,
                    asset,
                } => {
                    let staged_file = format!("asset-{index:04}.json");
                    let bytes = asset.canonical_bytes()?;
                    write_new_synced(&transaction_root.join(&staged_file), &bytes)?;
                    operations.push(JournalOperation::Put {
                        relative_path: path_to_slashes(relative_path)?,
                        staged_file,
                        expected_revision_sha256: *expected_revision_sha256,
                        new_revision_sha256: asset.digest()?,
                    });
                }
                WorkspaceMutation::Delete {
                    relative_path,
                    expected_revision_sha256,
                } => operations.push(JournalOperation::Delete {
                    relative_path: path_to_slashes(relative_path)?,
                    expected_revision_sha256: *expected_revision_sha256,
                }),
                WorkspaceMutation::Archive {
                    trash_relative_path,
                    asset,
                } => {
                    let staged_file = format!("asset-{index:04}.json");
                    let bytes = asset.canonical_bytes()?;
                    write_new_synced(&transaction_root.join(&staged_file), &bytes)?;
                    operations.push(JournalOperation::Put {
                        relative_path: path_to_slashes(trash_relative_path)?,
                        staged_file,
                        expected_revision_sha256: None,
                        new_revision_sha256: asset.digest()?,
                    });
                }
            }
        }
        let journal = WorkspaceTransactionJournal {
            schema: TRANSACTION_SCHEMA.into(),
            id,
            operations,
        };
        write_atomically(
            &transaction_root.join("transaction.json"),
            &canonical_json(&journal)?,
        )?;
        self.apply_transaction(&transaction_root, &journal)?;
        remove_transaction_directory(&self.root, &transaction_root)
    }

    pub fn move_asset(
        &self,
        id: &str,
        destination: &Path,
        expected_revision_sha256: Digest,
    ) -> Result<(), WorkspaceError> {
        let (asset, source) = self.load_asset(id)?;
        if source == destination {
            return Ok(());
        }
        self.transact(&[
            WorkspaceMutation::Put {
                relative_path: destination.to_path_buf(),
                expected_revision_sha256: None,
                asset,
            },
            WorkspaceMutation::Delete {
                relative_path: source,
                expected_revision_sha256,
            },
        ])
    }

    pub fn rename_asset(
        &self,
        id: &str,
        label: impl Into<String>,
        expected_revision_sha256: Digest,
    ) -> Result<Digest, WorkspaceError> {
        let (mut asset, relative_path) = self.load_asset(id)?;
        if asset.digest()? != expected_revision_sha256 {
            return Err(WorkspaceError::new("asset revision conflict before rename"));
        }
        asset.header.label = label.into();
        self.save_asset(&relative_path, Some(expected_revision_sha256), &asset)
    }

    pub fn duplicate_asset(
        &self,
        source_id: &str,
        new_id: impl Into<String>,
        new_label: impl Into<String>,
        destination: &Path,
    ) -> Result<Digest, WorkspaceError> {
        let (mut asset, _) = self.load_asset(source_id)?;
        asset.header.id = new_id.into();
        asset.header.label = new_label.into();
        asset.header.version = 1;
        self.save_asset(destination, None, &asset)
    }

    pub fn inbound_references(
        &self,
        id: &str,
    ) -> Result<Vec<WorkspaceAssetListing>, WorkspaceError> {
        validate_stable_id("asset id", id)?;
        let mut inbound = Vec::new();
        for listing in self.list_assets()? {
            let asset = read_asset(&self.root.join(&listing.relative_path))?;
            if asset
                .references
                .iter()
                .any(|reference| reference.asset_id == id)
            {
                inbound.push(listing);
            }
        }
        Ok(inbound)
    }

    pub fn delete_to_trash(
        &self,
        id: &str,
        expected_revision_sha256: Digest,
        allow_broken_references: bool,
    ) -> Result<(), WorkspaceError> {
        let inbound = self.inbound_references(id)?;
        if !allow_broken_references && !inbound.is_empty() {
            return Err(WorkspaceError::new(format!(
                "asset {id} is referenced by {}; confirm deletion to preserve these as broken stable-ID references",
                inbound
                    .iter()
                    .map(|listing| listing.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
        let (asset, source) = self.load_asset(id)?;
        if asset.digest()? != expected_revision_sha256 {
            return Err(WorkspaceError::new("asset revision conflict before delete"));
        }
        let trash_relative_path = Path::new(TRASH_ROOT).join(&source);
        self.transact(&[
            WorkspaceMutation::Archive {
                trash_relative_path,
                asset,
            },
            WorkspaceMutation::Delete {
                relative_path: source,
                expected_revision_sha256,
            },
        ])
    }

    pub fn list_trash(&self) -> Result<Vec<WorkspaceTrashListing>, WorkspaceError> {
        let root = self.root.join(TRASH_ROOT);
        let mut trash = Vec::new();
        collect_asset_files(&root, &mut |path| {
            let asset = read_asset(path)?;
            let relative = path
                .strip_prefix(&root)
                .map_err(|_| WorkspaceError::new("trash asset escaped trash root"))?
                .to_path_buf();
            trash.push(WorkspaceTrashListing {
                id: asset.header.id.clone(),
                label: asset.header.label.clone(),
                kind: asset.header.kind,
                original_relative_path: relative,
                revision_sha256: asset.digest()?,
            });
            Ok(())
        })?;
        trash.sort_by(|left, right| {
            left.kind
                .cmp(&right.kind)
                .then_with(|| left.label.cmp(&right.label))
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(trash)
    }

    pub fn restore_from_trash(
        &self,
        id: &str,
        expected_revision_sha256: Digest,
    ) -> Result<(), WorkspaceError> {
        let listing = self
            .list_trash()?
            .into_iter()
            .find(|listing| listing.id == id)
            .ok_or_else(|| WorkspaceError::new(format!("trashed asset {id} does not exist")))?;
        if listing.revision_sha256 != expected_revision_sha256 {
            return Err(WorkspaceError::new(
                "trash revision conflict before restore",
            ));
        }
        let trash_relative_path = Path::new(TRASH_ROOT).join(&listing.original_relative_path);
        let asset = read_asset(&self.root.join(&trash_relative_path))?;
        self.transact(&[
            WorkspaceMutation::Put {
                relative_path: listing.original_relative_path,
                expected_revision_sha256: None,
                asset,
            },
            WorkspaceMutation::Delete {
                relative_path: trash_relative_path,
                expected_revision_sha256,
            },
        ])
    }

    pub fn permanently_delete_from_trash(
        &self,
        id: &str,
        expected_revision_sha256: Digest,
    ) -> Result<(), WorkspaceError> {
        let listing = self
            .list_trash()?
            .into_iter()
            .find(|listing| listing.id == id)
            .ok_or_else(|| WorkspaceError::new(format!("trashed asset {id} does not exist")))?;
        if listing.revision_sha256 != expected_revision_sha256 {
            return Err(WorkspaceError::new(
                "trash revision conflict before permanent delete",
            ));
        }
        let path = self
            .root
            .join(TRASH_ROOT)
            .join(listing.original_relative_path);
        fs::remove_file(path).map_err(WorkspaceError::io)
    }

    fn import_project_template(
        &mut self,
        project: &PlannerWebProject,
        library_sha256: Digest,
    ) -> Result<(), WorkspaceError> {
        let fragment = stable_fragment(&project.id);
        self.import_project_template_as(project, library_sha256, &fragment)
    }

    fn fork_project_template(
        &mut self,
        project: &PlannerWebProject,
        library_sha256: Digest,
        namespace: &str,
    ) -> Result<(), WorkspaceError> {
        validate_stable_id("Library fork namespace", namespace)?;
        let fragment = stable_fragment(namespace);
        if fragment.is_empty() {
            return Err(WorkspaceError::new(
                "Library fork namespace must contain letters or digits",
            ));
        }
        self.import_project_template_as(project, library_sha256, &fragment)
    }

    fn import_project_template_as(
        &mut self,
        project: &PlannerWebProject,
        library_sha256: Digest,
        fragment: &str,
    ) -> Result<(), WorkspaceError> {
        project
            .validate()
            .map_err(|error| WorkspaceError::new(error.to_string()))?;
        if library_sha256 == Digest::ZERO {
            return Err(WorkspaceError::new("library digest must be nonzero"));
        }
        let state = project
            .start_state
            .clone()
            .ok_or_else(|| WorkspaceError::new("Library template has no grounded state seed"))?;
        let exact_context = state
            .snapshot
            .environment
            .runtime_configuration
            .exact_context()?;
        let scenario_id = format!("scenario.{fragment}");
        let graph_id = format!("route-graph.{fragment}");
        let state_id = format!("state-seed.{fragment}");
        let route_book_id = format!("route-book.{fragment}");
        let layout_id = format!("layout.{fragment}");
        let origin = |source_asset_id: String| {
            Some(WorkspaceAssetOrigin {
                library_id: project.id.clone(),
                library_version: BUILTIN_LIBRARY_VERSION.into(),
                library_sha256,
                source_asset_id,
            })
        };
        let route_book = match &project.route_book {
            Some(route_book) => route_book.clone(),
            None => {
                let scope = ContextScope {
                    selectors: vec![ContextSelector::Exact {
                        context: exact_context.clone(),
                    }],
                };
                let goal_id = project
                    .catalog
                    .mechanics
                    .goals
                    .first()
                    .ok_or_else(|| {
                        WorkspaceError::new(
                            "Library template needs a goal before authoring a route",
                        )
                    })?
                    .id
                    .clone();
                RouteBook {
                    schema: ROUTE_BOOK_SCHEMA.into(),
                    manifest: RouteBookManifest {
                        id: route_book_id.clone(),
                        version: "1.0.0".into(),
                        label: format!("{} authored route", project.label),
                        author: "Route Planner".into(),
                        source: "Workspace-authored exact transition sequence".into(),
                        scope: scope.clone(),
                        refinement_stack_sha256: Some(project.catalog.refinement_stack.digest()?),
                    },
                    goal_ids: vec![goal_id],
                    constraints: Vec::new(),
                    directives: Vec::new(),
                    steps: Vec::new(),
                    methods: Vec::new(),
                    regions: Vec::new(),
                    annotations: Vec::new(),
                }
            }
        };
        route_book.validate_against_composed(&project.catalog)?;
        let graph = PlannerGraph::project_composed_with_route_book(&project.catalog, &route_book)?;
        let state_asset = WorkspaceAsset {
            schema: WORKSPACE_ASSET_SCHEMA.into(),
            header: WorkspaceAssetHeader {
                id: state_id.clone(),
                label: format!("{} start state", project.label),
                kind: WorkspaceAssetKind::StateSeed,
                version: 1,
                origin: origin(format!("{}:start-state", project.id)),
            },
            references: Vec::new(),
            payload: WorkspaceAssetPayload::StateSeed { state },
        };
        let graph_references = vec![WorkspaceAssetReference {
            asset_id: route_book_id.clone(),
            kind: WorkspaceAssetKind::RouteBook,
        }];
        let graph_asset = WorkspaceAsset {
            schema: WORKSPACE_ASSET_SCHEMA.into(),
            header: WorkspaceAssetHeader {
                id: graph_id.clone(),
                label: project.label.clone(),
                kind: WorkspaceAssetKind::RouteGraph,
                version: 1,
                origin: origin(format!("{}:graph", project.id)),
            },
            references: graph_references,
            payload: WorkspaceAssetPayload::RouteGraph { graph },
        };
        let layout_asset = WorkspaceAsset {
            schema: WORKSPACE_ASSET_SCHEMA.into(),
            header: WorkspaceAssetHeader {
                id: layout_id.clone(),
                label: format!("{} layout", project.label),
                kind: WorkspaceAssetKind::Layout,
                version: 1,
                origin: origin(format!("{}:presentation", project.id)),
            },
            references: vec![WorkspaceAssetReference {
                asset_id: graph_id.clone(),
                kind: WorkspaceAssetKind::RouteGraph,
            }],
            payload: WorkspaceAssetPayload::Layout(LayoutAsset {
                semantic_asset_id: graph_id.clone(),
                positions: project
                    .presentation
                    .positions
                    .iter()
                    .map(|(id, position)| {
                        (
                            id.clone(),
                            LayoutPoint {
                                x: position.x,
                                y: position.y,
                            },
                        )
                    })
                    .collect(),
                viewport: None,
            }),
        };
        let mut scenario_references = vec![
            WorkspaceAssetReference {
                asset_id: graph_id.clone(),
                kind: WorkspaceAssetKind::RouteGraph,
            },
            WorkspaceAssetReference {
                asset_id: state_id.clone(),
                kind: WorkspaceAssetKind::StateSeed,
            },
        ];
        scenario_references.push(WorkspaceAssetReference {
            asset_id: route_book_id.clone(),
            kind: WorkspaceAssetKind::RouteBook,
        });
        scenario_references.sort();
        let scenario_asset = WorkspaceAsset {
            schema: WORKSPACE_ASSET_SCHEMA.into(),
            header: WorkspaceAssetHeader {
                id: scenario_id,
                label: project.label.clone(),
                kind: WorkspaceAssetKind::Scenario,
                version: 1,
                origin: origin(format!("{}:scenario", project.id)),
            },
            references: scenario_references,
            payload: WorkspaceAssetPayload::Scenario(ScenarioAsset {
                exact_context: exact_context.clone(),
                anchor: ScenarioAnchor::StateSeed {
                    state_seed_id: state_id.clone(),
                },
                route_graph_id: graph_id,
                state_seed_id: Some(state_id),
                route_book_id: Some(route_book_id.clone()),
            }),
        };
        let mut mutations = vec![
            WorkspaceMutation::Put {
                relative_path: Path::new("scenarios").join(format!("{fragment}.json")),
                expected_revision_sha256: None,
                asset: scenario_asset,
            },
            WorkspaceMutation::Put {
                relative_path: Path::new("route-graphs").join(format!("{fragment}.json")),
                expected_revision_sha256: None,
                asset: graph_asset,
            },
            WorkspaceMutation::Put {
                relative_path: Path::new("state-seeds").join(format!("{fragment}.json")),
                expected_revision_sha256: None,
                asset: state_asset,
            },
            WorkspaceMutation::Put {
                relative_path: Path::new("layouts").join(format!("{fragment}.json")),
                expected_revision_sha256: None,
                asset: layout_asset,
            },
        ];
        mutations.push(WorkspaceMutation::Put {
            relative_path: Path::new("route-books").join(format!("{fragment}.json")),
            expected_revision_sha256: None,
            asset: WorkspaceAsset {
                schema: WORKSPACE_ASSET_SCHEMA.into(),
                header: WorkspaceAssetHeader {
                    id: route_book_id,
                    label: route_book.manifest.label.clone(),
                    kind: WorkspaceAssetKind::RouteBook,
                    version: 1,
                    origin: origin(format!("{}:route-book", project.id)),
                },
                references: Vec::new(),
                payload: WorkspaceAssetPayload::RouteBook { route_book },
            },
        });
        self.transact(&mutations)?;
        self.mount_library(
            MountedLibrary {
                id: project.id.clone(),
                version: BUILTIN_LIBRARY_VERSION.into(),
                sha256: library_sha256,
                source: format!("builtin:{}", project.id),
            },
            exact_context,
        )
    }

    fn mount_library(
        &mut self,
        library: MountedLibrary,
        exact_context: ExactContext,
    ) -> Result<(), WorkspaceError> {
        match self
            .manifest
            .mounted_libraries
            .iter()
            .find(|mounted| mounted.id == library.id && mounted.version == library.version)
        {
            Some(mounted) if mounted.sha256 == library.sha256 => {}
            Some(mounted) => {
                return Err(WorkspaceError::new(format!(
                    "mounted Library {} changed from {} to {}; explicit upgrade is required",
                    library.id, mounted.sha256, library.sha256
                )));
            }
            None => self.manifest.mounted_libraries.push(library),
        }
        self.manifest.mounted_libraries.sort_by(|left, right| {
            left.id
                .cmp(&right.id)
                .then_with(|| left.version.cmp(&right.version))
        });
        if !self
            .manifest
            .exact_context_defaults
            .contains(&exact_context)
        {
            self.manifest.exact_context_defaults.push(exact_context);
            self.manifest.exact_context_defaults.sort();
        }
        self.manifest.validate()?;
        write_atomically(
            &self.root.join(MANIFEST_FILE),
            &self.manifest.canonical_bytes()?,
        )
    }

    fn ensure_transaction_root(&self) -> Result<(), WorkspaceError> {
        fs::create_dir_all(self.root.join(TRANSACTION_ROOT)).map_err(WorkspaceError::io)?;
        fs::create_dir_all(self.root.join(TRASH_ROOT)).map_err(WorkspaceError::io)?;
        fs::create_dir_all(self.root.join(FOLDER_TRASH_ROOT)).map_err(WorkspaceError::io)?;
        fs::create_dir_all(self.root.join(FOLDER_STAGING_ROOT)).map_err(WorkspaceError::io)
    }

    fn recover_transactions(&self) -> Result<(), WorkspaceError> {
        let root = self.root.join(TRANSACTION_ROOT);
        let mut transactions = fs::read_dir(&root)
            .map_err(WorkspaceError::io)?
            .map(|entry| entry.map(|entry| entry.path()).map_err(WorkspaceError::io))
            .collect::<Result<Vec<_>, _>>()?;
        transactions.sort();
        for transaction_root in transactions {
            if !transaction_root.is_dir() {
                return Err(WorkspaceError::new(format!(
                    "unexpected file in transaction journal: {}",
                    transaction_root.display()
                )));
            }
            let bytes =
                fs::read(transaction_root.join("transaction.json")).map_err(WorkspaceError::io)?;
            let journal: WorkspaceTransactionJournal =
                serde_json::from_slice(&bytes).map_err(WorkspaceError::json)?;
            if journal.schema != TRANSACTION_SCHEMA
                || transaction_root.file_name().and_then(|name| name.to_str())
                    != Some(journal.id.as_str())
            {
                return Err(WorkspaceError::new(format!(
                    "workspace transaction journal {} is invalid",
                    transaction_root.display()
                )));
            }
            self.apply_transaction(&transaction_root, &journal)?;
            remove_transaction_directory(&self.root, &transaction_root)?;
        }
        Ok(())
    }

    fn recover_folder_operations(&self) -> Result<(), WorkspaceError> {
        let trash_root = self.root.join(FOLDER_TRASH_ROOT);
        for entry in fs::read_dir(&trash_root).map_err(WorkspaceError::io)? {
            let entry = entry.map_err(WorkspaceError::io)?;
            if !entry.file_type().map_err(WorkspaceError::io)?.is_dir()
                || !entry.file_name().to_string_lossy().starts_with("folder-")
            {
                return Err(WorkspaceError::new(format!(
                    "unexpected entry in grouped folder Trash: {}",
                    entry.path().display()
                )));
            }
            let group = entry.path();
            let record = self.read_folder_trash_record(&group)?;
            let payload = group.join(FOLDER_TRASH_PAYLOAD);
            let live_exists = self
                .list_folders()?
                .iter()
                .any(|folder| folder.id == record.folder.id);
            match (payload.is_dir(), live_exists) {
                (true, false) => {}
                (false, true) => {
                    fs::remove_file(group.join(FOLDER_TRASH_RECORD_FILE))
                        .map_err(WorkspaceError::io)?;
                    fs::remove_dir(&group).map_err(WorkspaceError::io)?;
                }
                (true, true) => {
                    return Err(WorkspaceError::new(format!(
                        "grouped folder Trash recovery found both live and trashed copies of {}",
                        record.folder.id
                    )));
                }
                (false, false) => {
                    return Err(WorkspaceError::new(format!(
                        "grouped folder Trash recovery lost both copies of {}",
                        record.folder.id
                    )));
                }
            }
        }

        let staging_root = self.root.join(FOLDER_STAGING_ROOT);
        for entry in fs::read_dir(&staging_root).map_err(WorkspaceError::io)? {
            let entry = entry.map_err(WorkspaceError::io)?;
            if !entry.file_type().map_err(WorkspaceError::io)?.is_dir()
                || !entry.file_name().to_string_lossy().starts_with("folder-")
            {
                return Err(WorkspaceError::new(format!(
                    "unexpected entry in folder staging: {}",
                    entry.path().display()
                )));
            }
            let staging = entry.path();
            if staging.parent() != Some(staging_root.as_path()) {
                return Err(WorkspaceError::new(
                    "refusing to remove invalid folder staging path",
                ));
            }
            fs::remove_dir_all(staging).map_err(WorkspaceError::io)?;
        }
        Ok(())
    }

    fn validate_mutations(&self, mutations: &[WorkspaceMutation]) -> Result<(), WorkspaceError> {
        let listings = self.list_assets()?;
        let mut paths = BTreeSet::new();
        let deleted_paths = mutations
            .iter()
            .filter_map(|mutation| match mutation {
                WorkspaceMutation::Delete { relative_path, .. } => Some(relative_path),
                WorkspaceMutation::Put { .. } | WorkspaceMutation::Archive { .. } => None,
            })
            .collect::<BTreeSet<_>>();
        let mut available = listings
            .iter()
            .filter(|listing| !deleted_paths.contains(&listing.relative_path))
            .map(|listing| (listing.id.clone(), listing.kind))
            .collect::<BTreeMap<_, _>>();
        let mut put_ids = BTreeSet::new();
        for mutation in mutations {
            let (relative_path, expected_revision) = match mutation {
                WorkspaceMutation::Put {
                    relative_path,
                    expected_revision_sha256,
                    asset,
                } => {
                    asset.validate()?;
                    self.validate_asset_path(relative_path, asset.header.kind)?;
                    if !put_ids.insert(&asset.header.id) {
                        return Err(WorkspaceError::new(format!(
                            "transaction writes asset identity {} more than once",
                            asset.header.id
                        )));
                    }
                    available.insert(asset.header.id.clone(), asset.header.kind);
                    if let Some(existing) = listings
                        .iter()
                        .find(|listing| listing.id == asset.header.id)
                        && existing.relative_path != *relative_path
                        && !deleted_paths.contains(&existing.relative_path)
                    {
                        return Err(WorkspaceError::new(format!(
                            "asset identity {} already exists at another path",
                            asset.header.id
                        )));
                    }
                    (relative_path, *expected_revision_sha256)
                }
                WorkspaceMutation::Delete {
                    relative_path,
                    expected_revision_sha256,
                } => {
                    validate_relative_path("asset path", relative_path)?;
                    (relative_path, Some(*expected_revision_sha256))
                }
                WorkspaceMutation::Archive {
                    trash_relative_path,
                    asset,
                } => {
                    asset.validate()?;
                    validate_trash_path(trash_relative_path)?;
                    (trash_relative_path, None)
                }
            };
            if !paths.insert(relative_path) {
                return Err(WorkspaceError::new(format!(
                    "transaction mutates {} more than once",
                    relative_path.display()
                )));
            }
            let current_revision = self.revision_at(relative_path)?;
            if current_revision != expected_revision {
                return Err(WorkspaceError::new(format!(
                    "asset revision conflict at {}: expected {}, current {}",
                    relative_path.display(),
                    display_digest(expected_revision),
                    display_digest(current_revision)
                )));
            }
        }
        for asset in mutations.iter().filter_map(|mutation| match mutation {
            WorkspaceMutation::Put { asset, .. } => Some(asset),
            WorkspaceMutation::Delete { .. } | WorkspaceMutation::Archive { .. } => None,
        }) {
            validate_asset_references(asset, &available)?;
        }
        Ok(())
    }

    fn validate_asset_path(
        &self,
        relative_path: &Path,
        kind: WorkspaceAssetKind,
    ) -> Result<(), WorkspaceError> {
        validate_relative_path("asset path", relative_path)?;
        if relative_path.extension().and_then(|value| value.to_str()) != Some("json") {
            return Err(WorkspaceError::new("asset path must end in .json"));
        }
        let expected_root = self
            .manifest
            .asset_roots
            .get(&kind)
            .expect("validated manifest has every root");
        if !relative_path.starts_with(expected_root) {
            return Err(WorkspaceError::new(format!(
                "{kind:?} asset must be stored under {expected_root}"
            )));
        }
        Ok(())
    }

    fn revision_at(&self, relative_path: &Path) -> Result<Option<Digest>, WorkspaceError> {
        let path = self.root.join(relative_path);
        path.is_file()
            .then(|| read_asset(&path).and_then(|asset| asset.digest()))
            .transpose()
    }

    fn apply_transaction(
        &self,
        transaction_root: &Path,
        journal: &WorkspaceTransactionJournal,
    ) -> Result<(), WorkspaceError> {
        for operation in &journal.operations {
            match operation {
                JournalOperation::Put {
                    relative_path,
                    staged_file,
                    expected_revision_sha256,
                    new_revision_sha256,
                } => {
                    let relative_path = Path::new(relative_path);
                    validate_relative_path("transaction target", relative_path)?;
                    validate_relative_path("transaction staged file", Path::new(staged_file))?;
                    let current = self.revision_at(relative_path)?;
                    if current == Some(*new_revision_sha256) {
                        continue;
                    }
                    if current != *expected_revision_sha256 {
                        return Err(WorkspaceError::new(format!(
                            "cannot recover transaction {}: {} expected {}, current {}",
                            journal.id,
                            relative_path.display(),
                            display_digest(*expected_revision_sha256),
                            display_digest(current)
                        )));
                    }
                    let bytes =
                        fs::read(transaction_root.join(staged_file)).map_err(WorkspaceError::io)?;
                    let staged_asset: WorkspaceAsset =
                        serde_json::from_slice(&bytes).map_err(WorkspaceError::json)?;
                    if staged_asset.digest()? != *new_revision_sha256 {
                        return Err(WorkspaceError::new(format!(
                            "transaction {} staged asset digest changed",
                            journal.id
                        )));
                    }
                    let target = self.root.join(relative_path);
                    if let Some(parent) = target.parent() {
                        fs::create_dir_all(parent).map_err(WorkspaceError::io)?;
                    }
                    write_atomically(&target, &bytes)?;
                }
                JournalOperation::Delete {
                    relative_path,
                    expected_revision_sha256,
                } => {
                    let relative_path = Path::new(relative_path);
                    validate_relative_path("transaction target", relative_path)?;
                    let Some(current) = self.revision_at(relative_path)? else {
                        continue;
                    };
                    if current != *expected_revision_sha256 {
                        return Err(WorkspaceError::new(format!(
                            "cannot recover transaction {}: {} expected {}, current {}",
                            journal.id,
                            relative_path.display(),
                            expected_revision_sha256,
                            current
                        )));
                    }
                    fs::remove_file(self.root.join(relative_path)).map_err(WorkspaceError::io)?;
                }
            }
        }
        Ok(())
    }

    fn asset_root(&self, kind: WorkspaceAssetKind) -> Result<PathBuf, WorkspaceError> {
        let relative = self
            .manifest
            .asset_roots
            .get(&kind)
            .ok_or_else(|| WorkspaceError::new(format!("missing {kind:?} asset root")))?;
        Ok(self.root.join(relative))
    }
}

fn validate_asset_references(
    asset: &WorkspaceAsset,
    available: &BTreeMap<String, WorkspaceAssetKind>,
) -> Result<(), WorkspaceError> {
    for reference in &asset.references {
        let Some(actual_kind) = available.get(&reference.asset_id) else {
            return Err(WorkspaceError::new(format!(
                "{} references missing {:?} asset {}",
                asset.header.id, reference.kind, reference.asset_id
            )));
        };
        if *actual_kind != reference.kind {
            return Err(WorkspaceError::new(format!(
                "{} references {} as {:?}, but it is {:?}",
                asset.header.id, reference.asset_id, reference.kind, actual_kind
            )));
        }
    }
    Ok(())
}

pub fn dependency_issues(
    manifest: &WorkspaceManifest,
    available: &BTreeMap<(String, String), Digest>,
) -> Vec<LibraryDependencyIssue> {
    manifest
        .mounted_libraries
        .iter()
        .filter_map(|pin| {
            let key = (pin.id.clone(), pin.version.clone());
            match available.get(&key) {
                None => Some(LibraryDependencyIssue::Missing {
                    id: pin.id.clone(),
                    version: pin.version.clone(),
                    expected_sha256: pin.sha256,
                    source: pin.source.clone(),
                }),
                Some(actual) if *actual != pin.sha256 => Some(LibraryDependencyIssue::Changed {
                    id: pin.id.clone(),
                    version: pin.version.clone(),
                    expected_sha256: pin.sha256,
                    actual_sha256: *actual,
                    source: pin.source.clone(),
                }),
                Some(_) => None,
            }
        })
        .collect()
}

fn format_dependency_issues(issues: &[LibraryDependencyIssue]) -> String {
    let details = issues
        .iter()
        .map(|issue| match issue {
            LibraryDependencyIssue::Missing {
                id,
                version,
                expected_sha256,
                source,
            } => format!(
                "{id} {version} is missing (expected {expected_sha256}; restore from {source})"
            ),
            LibraryDependencyIssue::Changed {
                id,
                version,
                expected_sha256,
                actual_sha256,
                source,
            } => format!(
                "{id} {version} changed (expected {expected_sha256}, found {actual_sha256}; restore the pinned library from {source} or explicitly upgrade the workspace)"
            ),
        })
        .collect::<Vec<_>>()
        .join("; ");
    format!("workspace library dependencies are not satisfied: {details}")
}

fn read_manifest_and_migrate(path: &Path) -> Result<WorkspaceManifest, WorkspaceError> {
    let bytes = fs::read(path).map_err(WorkspaceError::io)?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(WorkspaceError::json)?;
    let schema = value
        .get("schema")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| WorkspaceError::new("workspace manifest has no schema"))?;
    let manifest = match schema {
        WORKSPACE_MANIFEST_SCHEMA => serde_json::from_value(value).map_err(WorkspaceError::json)?,
        LEGACY_WORKSPACE_MANIFEST_SCHEMA => {
            let legacy: LegacyWorkspaceManifest =
                serde_json::from_value(value).map_err(WorkspaceError::json)?;
            if legacy.schema != LEGACY_WORKSPACE_MANIFEST_SCHEMA {
                return Err(WorkspaceError::new("legacy workspace schema is invalid"));
            }
            let migrated = WorkspaceManifest {
                schema: WORKSPACE_MANIFEST_SCHEMA.into(),
                format_version: WORKSPACE_FORMAT_VERSION,
                id: legacy.id,
                label: legacy.label,
                mounted_libraries: legacy.mounted_libraries,
                exact_context_defaults: legacy.exact_context_defaults,
                asset_roots: legacy.asset_roots,
            };
            migrated.validate()?;
            write_atomically(path, &migrated.canonical_bytes()?)?;
            migrated
        }
        _ => {
            return Err(WorkspaceError::new(format!(
                "workspace schema {schema} is unsupported; migrate it with a compatible application version"
            )));
        }
    };
    manifest.validate()?;
    Ok(manifest)
}

fn read_asset(path: &Path) -> Result<WorkspaceAsset, WorkspaceError> {
    let bytes = fs::read(path).map_err(WorkspaceError::io)?;
    let asset: WorkspaceAsset = serde_json::from_slice(&bytes).map_err(WorkspaceError::json)?;
    asset.validate()?;
    Ok(asset)
}

fn read_folder(path: &Path) -> Result<WorkspaceFolder, WorkspaceError> {
    let bytes = fs::read(path).map_err(WorkspaceError::io)?;
    let folder: WorkspaceFolder = serde_json::from_slice(&bytes).map_err(WorkspaceError::json)?;
    folder.validate()?;
    Ok(folder)
}

fn read_folder_trash_record(group: &Path) -> Result<WorkspaceFolderTrashRecord, WorkspaceError> {
    let bytes = fs::read(group.join(FOLDER_TRASH_RECORD_FILE)).map_err(WorkspaceError::io)?;
    let record: WorkspaceFolderTrashRecord =
        serde_json::from_slice(&bytes).map_err(WorkspaceError::json)?;
    if record.schema != FOLDER_TRASH_RECORD_SCHEMA
        || record.folder.digest()? != record.revision_sha256
        || record.folder_count == 0
    {
        return Err(WorkspaceError::new(format!(
            "grouped folder Trash record {} is invalid",
            group.display()
        )));
    }
    validate_relative_path(
        "grouped folder Trash original path",
        &record.original_relative_path,
    )?;
    Ok(record)
}

fn collect_folder_markers(
    root: &Path,
    visit: &mut impl FnMut(&Path, &Path) -> Result<(), WorkspaceError>,
) -> Result<(), WorkspaceError> {
    if !root.is_dir() {
        return Err(WorkspaceError::new(format!(
            "asset root {} does not exist",
            root.display()
        )));
    }
    let marker = root.join(FOLDER_MARKER_FILE);
    if marker.is_file() {
        visit(root, &marker)?;
    }
    for entry in fs::read_dir(root).map_err(WorkspaceError::io)? {
        let entry = entry.map_err(WorkspaceError::io)?;
        if entry.file_type().map_err(WorkspaceError::io)?.is_dir() {
            collect_folder_markers(&entry.path(), visit)?;
        }
    }
    Ok(())
}

fn validate_folder_subtree_files(root: &Path) -> Result<(), WorkspaceError> {
    for entry in fs::read_dir(root).map_err(WorkspaceError::io)? {
        let entry = entry.map_err(WorkspaceError::io)?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(WorkspaceError::io)?;
        if file_type.is_dir() {
            validate_folder_subtree_files(&path)?;
        } else if !file_type.is_file()
            || (path.file_name().and_then(|name| name.to_str()) != Some(FOLDER_MARKER_FILE)
                && path.extension().and_then(|extension| extension.to_str()) != Some("json"))
        {
            return Err(WorkspaceError::new(format!(
                "folder subtree contains unsupported file {}; only typed workspace JSON files can be duplicated",
                path.display()
            )));
        }
    }
    Ok(())
}

fn validate_folder_relative_path(
    manifest: &WorkspaceManifest,
    kind: WorkspaceAssetKind,
    relative_path: &Path,
) -> Result<(), WorkspaceError> {
    validate_relative_path("workspace folder path", relative_path)?;
    let root = manifest
        .asset_roots
        .get(&kind)
        .expect("validated manifest has every asset root");
    if relative_path == Path::new(root) || !relative_path.starts_with(root) {
        return Err(WorkspaceError::new(format!(
            "{kind:?} folder must be below the fixed {root} root"
        )));
    }
    Ok(())
}

fn validate_export_folder_hierarchy(
    manifest: &WorkspaceManifest,
    folders: &[WorkspaceExportFolder],
) -> Result<(), WorkspaceError> {
    let by_id = folders
        .iter()
        .map(|folder| (folder.folder.id.as_str(), folder))
        .collect::<BTreeMap<_, _>>();
    for record in folders {
        let expected_parent = record
            .relative_path
            .parent()
            .ok_or_else(|| WorkspaceError::new("import folder has no parent path"))?;
        match &record.folder.parent_id {
            Some(parent_id) => {
                let parent = by_id.get(parent_id.as_str()).ok_or_else(|| {
                    WorkspaceError::new(format!(
                        "import folder {} references missing parent {parent_id}",
                        record.folder.id
                    ))
                })?;
                if parent.folder.kind != record.folder.kind
                    || parent.relative_path != expected_parent
                {
                    return Err(WorkspaceError::new(format!(
                        "import folder {} parent identity does not match its path",
                        record.folder.id
                    )));
                }
            }
            None => {
                let root = manifest
                    .asset_roots
                    .get(&record.folder.kind)
                    .expect("validated manifest has every asset root");
                if expected_parent != Path::new(root) {
                    return Err(WorkspaceError::new(format!(
                        "import folder {} without a parent must be directly below {root}",
                        record.folder.id
                    )));
                }
            }
        }
    }
    Ok(())
}

fn validate_directory_name(value: &str) -> Result<(), WorkspaceError> {
    let path = Path::new(value);
    if value.is_empty()
        || value.len() > 128
        || path.components().count() != 1
        || !matches!(path.components().next(), Some(Component::Normal(_)))
        || value.chars().any(char::is_control)
        || matches!(value, "." | "..")
    {
        return Err(WorkspaceError::new(
            "folder directory name must be one safe printable path segment",
        ));
    }
    Ok(())
}

fn clone_identity_map(
    source_ids: &[&String],
    namespace: &str,
    category: &str,
    existing: &BTreeSet<String>,
    root_id: Option<&str>,
) -> Result<BTreeMap<String, String>, WorkspaceError> {
    let mut mapping = BTreeMap::new();
    let mut allocated = existing.clone();
    let mut index = 0_usize;
    for source in source_ids {
        let candidate = if root_id == Some(source.as_str()) {
            namespace.to_owned()
        } else {
            loop {
                let candidate = format!("{namespace}.{category}-{index:03}");
                index += 1;
                if !allocated.contains(&candidate) {
                    break candidate;
                }
            }
        };
        validate_stable_id("duplicated stable identity", &candidate)?;
        if !allocated.insert(candidate.clone()) {
            return Err(WorkspaceError::new(format!(
                "duplicated stable identity {candidate} collides"
            )));
        }
        mapping.insert((*source).clone(), candidate);
    }
    Ok(mapping)
}

fn remap_cloned_asset(asset: &mut WorkspaceAsset, mapping: &BTreeMap<String, String>) {
    asset.header.id = mapping
        .get(&asset.header.id)
        .expect("every cloned asset has a new identity")
        .clone();
    asset.header.version = 1;
    for reference in &mut asset.references {
        if let Some(remapped) = mapping.get(&reference.asset_id) {
            reference.asset_id = remapped.clone();
        }
    }
    match &mut asset.payload {
        WorkspaceAssetPayload::Scenario(scenario) => {
            if let Some(remapped) = mapping.get(&scenario.route_graph_id) {
                scenario.route_graph_id = remapped.clone();
            }
            if let Some(id) = &mut scenario.state_seed_id
                && let Some(remapped) = mapping.get(id)
            {
                *id = remapped.clone();
            }
            if let Some(id) = &mut scenario.route_book_id
                && let Some(remapped) = mapping.get(id)
            {
                *id = remapped.clone();
            }
            if let ScenarioAnchor::StateSeed { state_seed_id } = &mut scenario.anchor
                && let Some(remapped) = mapping.get(state_seed_id)
            {
                *state_seed_id = remapped.clone();
            }
        }
        WorkspaceAssetPayload::Layout(layout) => {
            if let Some(remapped) = mapping.get(&layout.semantic_asset_id) {
                layout.semantic_asset_id = remapped.clone();
            }
        }
        WorkspaceAssetPayload::RouteGraph { .. }
        | WorkspaceAssetPayload::ReusableSubgraph { .. }
        | WorkspaceAssetPayload::CustomNodeDefinition(_)
        | WorkspaceAssetPayload::StateSeed { .. }
        | WorkspaceAssetPayload::QueryGoal(_)
        | WorkspaceAssetPayload::RouteBook { .. } => {}
    }
}

fn collect_asset_files(
    root: &Path,
    visit: &mut impl FnMut(&Path) -> Result<(), WorkspaceError>,
) -> Result<(), WorkspaceError> {
    if !root.is_dir() {
        return Err(WorkspaceError::new(format!(
            "asset root {} does not exist",
            root.display()
        )));
    }
    for entry in fs::read_dir(root).map_err(WorkspaceError::io)? {
        let entry = entry.map_err(WorkspaceError::io)?;
        let path = entry.path();
        if entry.file_type().map_err(WorkspaceError::io)?.is_dir() {
            collect_asset_files(&path, visit)?;
        } else if path.file_name().and_then(|value| value.to_str()) != Some(FOLDER_MARKER_FILE)
            && path.extension().and_then(|value| value.to_str()) == Some("json")
        {
            visit(&path)?;
        }
    }
    Ok(())
}

fn validate_pins(field: &str, pins: &[CustomNodePin]) -> Result<(), WorkspaceError> {
    let mut ids = BTreeSet::new();
    for pin in pins {
        validate_stable_id(field, &pin.id)?;
        validate_label(field, &pin.label)?;
        validate_stable_id(field, &pin.value_type)?;
        if !ids.insert(&pin.id) {
            return Err(WorkspaceError::new(format!(
                "{field} contains duplicate {}",
                pin.id
            )));
        }
    }
    Ok(())
}

fn validate_stable_id(field: &str, value: &str) -> Result<(), WorkspaceError> {
    if value.is_empty()
        || value.len() > 128
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'.' | b'_' | b'-' | b'/' | b':')
        })
    {
        return Err(WorkspaceError::new(format!(
            "{field} must use 1-128 lowercase ASCII letters, digits, '.', '_', '-', '/', or ':'"
        )));
    }
    Ok(())
}

fn stable_fragment(value: &str) -> String {
    let fragment = value
        .chars()
        .map(|character| {
            if character.is_ascii_lowercase() || character.is_ascii_digit() {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    fragment.trim_matches('-').to_owned()
}

fn folder_trash_directory_name(id: &str) -> String {
    format!("folder-{}", Digest(Sha256::digest(id.as_bytes()).into()))
}

fn validate_label(field: &str, value: &str) -> Result<(), WorkspaceError> {
    if value.trim().is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        return Err(WorkspaceError::new(format!(
            "{field} must be nonempty printable text of at most 256 characters"
        )));
    }
    Ok(())
}

fn validate_relative_path(field: &str, path: &Path) -> Result<(), WorkspaceError> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            !matches!(component, Component::Normal(_))
                || component
                    .as_os_str()
                    .to_str()
                    .is_none_or(|value| value.is_empty() || value.chars().any(char::is_control))
        })
    {
        return Err(WorkspaceError::new(format!(
            "{field} must be a nonempty relative path without traversal"
        )));
    }
    Ok(())
}

fn validate_trash_path(path: &Path) -> Result<(), WorkspaceError> {
    validate_relative_path("trash path", path)?;
    if !path.starts_with(TRASH_ROOT)
        || path == Path::new(TRASH_ROOT)
        || path.extension().and_then(|value| value.to_str()) != Some("json")
    {
        return Err(WorkspaceError::new(
            "trash asset path must be a JSON file below the workspace trash root",
        ));
    }
    Ok(())
}

fn canonical_json<T: Serialize>(value: &T) -> Result<Vec<u8>, WorkspaceError> {
    let mut bytes = serde_json::to_vec(value).map_err(WorkspaceError::json)?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn path_to_slashes(path: &Path) -> Result<String, WorkspaceError> {
    validate_relative_path("asset path", path)?;
    Ok(path
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/"))
}

fn write_new_synced(path: &Path, bytes: &[u8]) -> Result<(), WorkspaceError> {
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(WorkspaceError::io)?;
    output.write_all(bytes).map_err(WorkspaceError::io)?;
    output.sync_all().map_err(WorkspaceError::io)
}

fn remove_transaction_directory(root: &Path, transaction: &Path) -> Result<(), WorkspaceError> {
    let expected_parent = root.join(TRANSACTION_ROOT);
    if transaction.parent() != Some(expected_parent.as_path())
        || transaction.file_name().is_none()
        || !transaction.is_dir()
    {
        return Err(WorkspaceError::new(
            "refusing to remove an invalid transaction directory",
        ));
    }
    fs::remove_dir_all(transaction).map_err(WorkspaceError::io)
}

fn display_digest(value: Option<Digest>) -> String {
    value
        .map(|digest| digest.to_string())
        .unwrap_or_else(|| "none".into())
}

fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), WorkspaceError> {
    let temporary = path.with_extension(format!(
        "tmp-{}-{}",
        std::process::id(),
        NEXT_TEMPORARY_ID.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(WorkspaceError::io)?;
        output.write_all(bytes).map_err(WorkspaceError::io)?;
        output.sync_all().map_err(WorkspaceError::io)?;
        drop(output);
        replace_file(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> Result<(), WorkspaceError> {
    fs::rename(source, destination).map_err(WorkspaceError::io)
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> Result<(), WorkspaceError> {
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
        Err(WorkspaceError::io(std::io::Error::last_os_error()))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dusklight_route_planner::graph::PLANNER_GRAPH_SCHEMA;

    fn temporary_directory(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "dusklight-route-planner-{label}-{}-{}",
            std::process::id(),
            NEXT_TEMPORARY_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn graph_asset(id: &str, label: &str) -> WorkspaceAsset {
        WorkspaceAsset {
            schema: WORKSPACE_ASSET_SCHEMA.into(),
            header: WorkspaceAssetHeader {
                id: id.into(),
                label: label.into(),
                kind: WorkspaceAssetKind::RouteGraph,
                version: 1,
                origin: None,
            },
            references: Vec::new(),
            payload: WorkspaceAssetPayload::RouteGraph {
                graph: PlannerGraph {
                    schema: PLANNER_GRAPH_SCHEMA.into(),
                    fact_catalog_sha256: Digest([1; 32]),
                    mechanics_catalog_sha256: Digest([2; 32]),
                    refinement_stack_sha256: None,
                    route_book_sha256: None,
                    nodes: Vec::new(),
                    edges: Vec::new(),
                    regions: Vec::new(),
                },
            },
        }
    }

    #[test]
    fn manifest_is_small_and_defines_fixed_typed_roots() {
        let mut manifest = WorkspaceManifest::new("workspace.test", "Test workspace").unwrap();
        let value = serde_json::to_value(&manifest).unwrap();
        assert_eq!(manifest.asset_roots.len(), WorkspaceAssetKind::ALL.len());
        for kind in WorkspaceAssetKind::ALL {
            assert_eq!(manifest.asset_roots[&kind], kind.root_name());
        }
        for forbidden in ["catalog", "graph", "snapshot", "layout", "route_book"] {
            assert!(value.get(forbidden).is_none());
        }
        assert!(manifest.canonical_bytes().unwrap().ends_with(b"\n"));
        manifest
            .asset_roots
            .insert(WorkspaceAssetKind::RouteGraph, "user-selected-root".into());
        assert!(
            manifest
                .validate()
                .unwrap_err()
                .to_string()
                .contains("asset root is fixed at route-graphs")
        );
    }

    #[test]
    fn semantic_and_layout_identity_are_independent() {
        let semantic = graph_asset("graph.ordon", "Ordon route");
        let semantic_digest = semantic.digest().unwrap();
        let mut layout = WorkspaceAsset {
            schema: WORKSPACE_ASSET_SCHEMA.into(),
            header: WorkspaceAssetHeader {
                id: "layout.ordon".into(),
                label: "Ordon layout".into(),
                kind: WorkspaceAssetKind::Layout,
                version: 1,
                origin: None,
            },
            references: vec![WorkspaceAssetReference {
                asset_id: semantic.header.id.clone(),
                kind: WorkspaceAssetKind::RouteGraph,
            }],
            payload: WorkspaceAssetPayload::Layout(LayoutAsset {
                semantic_asset_id: semantic.header.id.clone(),
                positions: BTreeMap::from([(
                    "node.start".into(),
                    LayoutPoint { x: 10.0, y: 20.0 },
                )]),
                viewport: None,
            }),
        };
        layout.validate().unwrap();
        let first_layout_digest = layout.digest().unwrap();
        let WorkspaceAssetPayload::Layout(layout_payload) = &mut layout.payload else {
            unreachable!()
        };
        layout_payload.positions.get_mut("node.start").unwrap().x = 500.0;
        assert_ne!(first_layout_digest, layout.digest().unwrap());
        assert_eq!(semantic_digest, semantic.digest().unwrap());
    }

    #[test]
    fn store_keeps_identity_when_asset_path_changes() {
        let root = temporary_directory("stable-identity");
        let manifest = WorkspaceManifest::new("workspace.test", "Test workspace").unwrap();
        let store = WorkspaceStore::create(&root, manifest).unwrap();
        let asset = graph_asset("graph.ordon", "Ordon route");
        let first = Path::new("route-graphs/ordon/route.json");
        store.save_asset(first, None, &asset).unwrap();
        let moved = Path::new("route-graphs/routes/ordon.json");
        fs::create_dir_all(root.join("route-graphs/routes")).unwrap();
        fs::rename(root.join(first), root.join(moved)).unwrap();
        let (loaded, relative_path) = store.load_asset("graph.ordon").unwrap();
        assert_eq!(loaded.header.id, "graph.ordon");
        assert_eq!(relative_path, moved);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn open_reports_missing_and_changed_library_pins_actionably() {
        let root = temporary_directory("library-pins");
        let mut manifest = WorkspaceManifest::new("workspace.test", "Test workspace").unwrap();
        manifest.mounted_libraries.push(MountedLibrary {
            id: "library.gz2e01".into(),
            version: "1.0.0".into(),
            sha256: Digest([7; 32]),
            source: "libraries/gz2e01.json".into(),
        });
        WorkspaceStore::create(&root, manifest).unwrap();
        let missing = WorkspaceStore::open(&root, &BTreeMap::new()).unwrap_err();
        assert!(missing.to_string().contains("is missing"));
        assert!(missing.to_string().contains("libraries/gz2e01.json"));
        let changed = WorkspaceStore::open(
            &root,
            &BTreeMap::from([(("library.gz2e01".into(), "1.0.0".into()), Digest([8; 32]))]),
        )
        .unwrap_err();
        assert!(changed.to_string().contains("changed"));
        assert!(changed.to_string().contains("explicitly upgrade"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn save_is_canonical_revision_checked_and_path_confined() {
        let root = temporary_directory("asset-save");
        let manifest = WorkspaceManifest::new("workspace.test", "Test workspace").unwrap();
        let store = WorkspaceStore::create(&root, manifest).unwrap();
        let mut asset = graph_asset("graph.ordon", "Ordon route");
        let path = Path::new("route-graphs/ordon/routes/ordon.json");
        let first_revision = store.save_asset(path, None, &asset).unwrap();
        assert!(root.join("route-graphs/ordon/routes").is_dir());
        assert_eq!(
            fs::read(root.join(path)).unwrap(),
            asset.canonical_bytes().unwrap()
        );
        asset.header.label = "Renamed route".into();
        assert!(
            store
                .save_asset(path, None, &asset)
                .unwrap_err()
                .to_string()
                .contains("revision conflict")
        );
        let second_revision = store
            .save_asset(path, Some(first_revision), &asset)
            .unwrap();
        assert_ne!(first_revision, second_revision);
        assert!(
            store
                .save_asset(Path::new("../escape.json"), None, &asset)
                .unwrap_err()
                .to_string()
                .contains("without traversal")
        );
        let wrong_root = Path::new("custom-nodes/ordon.json");
        assert!(
            store
                .save_asset(wrong_root, None, &asset)
                .unwrap_err()
                .to_string()
                .contains("must be stored under route-graphs")
        );
        assert!(!root.join(wrong_root).exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unsupported_schemas_require_migration_instead_of_best_effort_loading() {
        let root = temporary_directory("migration");
        let manifest = WorkspaceManifest::new("workspace.test", "Test workspace").unwrap();
        WorkspaceStore::create(&root, manifest).unwrap();
        let path = root.join(MANIFEST_FILE);
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        value["schema"] = "dusklight.route-planner.workspace/v99".into();
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let error = WorkspaceStore::open(&root, &BTreeMap::new()).unwrap_err();
        assert!(error.to_string().contains("migrate"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn known_manifest_schema_migrates_automatically_and_canonically() {
        let root = temporary_directory("known-migration");
        let manifest = WorkspaceManifest::new("workspace.test", "Test workspace").unwrap();
        WorkspaceStore::create(&root, manifest).unwrap();
        let path = root.join(MANIFEST_FILE);
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        value["schema"] = LEGACY_WORKSPACE_MANIFEST_SCHEMA.into();
        value.as_object_mut().unwrap().remove("format_version");
        fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();

        let opened = WorkspaceStore::open(&root, &BTreeMap::new()).unwrap();
        assert_eq!(opened.manifest().schema, WORKSPACE_MANIFEST_SCHEMA);
        assert_eq!(opened.manifest().format_version, WORKSPACE_FORMAT_VERSION);
        assert_eq!(
            fs::read(&path).unwrap(),
            opened.manifest().canonical_bytes().unwrap()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn move_is_one_recoverable_transaction_and_preserves_identity() {
        let root = temporary_directory("transactional-move");
        let manifest = WorkspaceManifest::new("workspace.test", "Test workspace").unwrap();
        let store = WorkspaceStore::create(&root, manifest).unwrap();
        let asset = graph_asset("graph.ordon", "Ordon route");
        let source = Path::new("route-graphs/ordon.json");
        let revision = store.save_asset(source, None, &asset).unwrap();
        let destination = Path::new("route-graphs/routes/ordon.json");
        store
            .move_asset("graph.ordon", destination, revision)
            .unwrap();

        assert!(!root.join(source).exists());
        assert!(root.join(destination).is_file());
        let (loaded, path) = store.load_asset("graph.ordon").unwrap();
        assert_eq!(loaded.header.id, "graph.ordon");
        assert_eq!(path, destination);
        assert_eq!(
            fs::read_dir(root.join(TRANSACTION_ROOT)).unwrap().count(),
            0
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn open_finishes_a_transaction_interrupted_between_operations() {
        let root = temporary_directory("transaction-recovery");
        let manifest = WorkspaceManifest::new("workspace.test", "Test workspace").unwrap();
        let store = WorkspaceStore::create(&root, manifest).unwrap();
        let asset = graph_asset("graph.ordon", "Ordon route");
        let source = Path::new("route-graphs/ordon.json");
        let source_revision = store.save_asset(source, None, &asset).unwrap();
        let destination = Path::new("route-graphs/routes/ordon.json");
        fs::create_dir_all(root.join("route-graphs/routes")).unwrap();

        let id = "transaction-interrupted";
        let transaction_root = root.join(TRANSACTION_ROOT).join(id);
        fs::create_dir(&transaction_root).unwrap();
        let staged_file = "asset-0000.json";
        let bytes = asset.canonical_bytes().unwrap();
        fs::write(transaction_root.join(staged_file), &bytes).unwrap();
        let journal = WorkspaceTransactionJournal {
            schema: TRANSACTION_SCHEMA.into(),
            id: id.into(),
            operations: vec![
                JournalOperation::Put {
                    relative_path: path_to_slashes(destination).unwrap(),
                    staged_file: staged_file.into(),
                    expected_revision_sha256: None,
                    new_revision_sha256: asset.digest().unwrap(),
                },
                JournalOperation::Delete {
                    relative_path: path_to_slashes(source).unwrap(),
                    expected_revision_sha256: source_revision,
                },
            ],
        };
        fs::write(
            transaction_root.join("transaction.json"),
            canonical_json(&journal).unwrap(),
        )
        .unwrap();
        // Simulate a crash after the first replacement but before source delete.
        fs::write(root.join(destination), &bytes).unwrap();
        drop(store);

        let recovered = WorkspaceStore::open(&root, &BTreeMap::new()).unwrap();
        assert!(!root.join(source).exists());
        assert!(root.join(destination).is_file());
        assert_eq!(recovered.load_asset("graph.ordon").unwrap().1, destination);
        assert!(!transaction_root.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn asset_crud_preserves_ids_references_and_recoverable_trash() {
        let root = temporary_directory("asset-crud");
        let manifest = WorkspaceManifest::new("workspace.test", "Test workspace").unwrap();
        let store = WorkspaceStore::create(&root, manifest).unwrap();
        let mut graph = graph_asset("graph.ordon", "Ordon route");
        let graph_path = Path::new("route-graphs/ordon.json");
        let first_revision = store.save_asset(graph_path, None, &graph).unwrap();

        let layout = WorkspaceAsset {
            schema: WORKSPACE_ASSET_SCHEMA.into(),
            header: WorkspaceAssetHeader {
                id: "layout.ordon".into(),
                label: "Ordon layout".into(),
                kind: WorkspaceAssetKind::Layout,
                version: 1,
                origin: None,
            },
            references: vec![WorkspaceAssetReference {
                asset_id: graph.header.id.clone(),
                kind: WorkspaceAssetKind::RouteGraph,
            }],
            payload: WorkspaceAssetPayload::Layout(LayoutAsset {
                semantic_asset_id: graph.header.id.clone(),
                positions: BTreeMap::new(),
                viewport: None,
            }),
        };
        let mut missing_reference = layout.clone();
        missing_reference.header.id = "layout.missing-reference".into();
        missing_reference.references[0].asset_id = "graph.absent".into();
        let missing_path = Path::new("layouts/missing-reference.json");
        assert!(
            store
                .save_asset(missing_path, None, &missing_reference)
                .unwrap_err()
                .to_string()
                .contains("references missing RouteGraph asset graph.absent")
        );
        assert!(!root.join(missing_path).exists());
        let mut wrong_kind = layout.clone();
        wrong_kind.header.id = "layout.wrong-kind".into();
        wrong_kind.references[0].kind = WorkspaceAssetKind::StateSeed;
        let wrong_kind_path = Path::new("layouts/wrong-kind.json");
        assert!(
            store
                .save_asset(wrong_kind_path, None, &wrong_kind)
                .unwrap_err()
                .to_string()
                .contains("as StateSeed, but it is RouteGraph")
        );
        assert!(!root.join(wrong_kind_path).exists());
        store
            .save_asset(Path::new("layouts/ordon.json"), None, &layout)
            .unwrap();
        assert_eq!(
            store.inbound_references("graph.ordon").unwrap()[0].id,
            "layout.ordon"
        );
        let error = store
            .delete_to_trash("graph.ordon", first_revision, false)
            .unwrap_err();
        assert!(error.to_string().contains("layout.ordon"));

        store
            .delete_to_trash("graph.ordon", first_revision, true)
            .unwrap();
        assert!(store.load_asset("graph.ordon").is_err());
        let trash = store.list_trash().unwrap();
        assert_eq!(trash[0].original_relative_path, graph_path);
        assert_eq!(
            store.inbound_references("graph.ordon").unwrap()[0].id,
            "layout.ordon"
        );
        store
            .restore_from_trash("graph.ordon", trash[0].revision_sha256)
            .unwrap();

        let restored_revision = store.load_asset("graph.ordon").unwrap().0.digest().unwrap();
        let renamed_revision = store
            .rename_asset("graph.ordon", "Renamed route", restored_revision)
            .unwrap();
        graph.header.id = "graph.ordon-copy".into();
        graph.header.label = "Ordon route copy".into();
        store
            .duplicate_asset(
                "graph.ordon",
                "graph.ordon-copy",
                "Ordon route copy",
                Path::new("route-graphs/ordon-copy.json"),
            )
            .unwrap();
        assert_eq!(store.list_assets().unwrap().len(), 3);
        store
            .move_asset(
                "graph.ordon",
                Path::new("route-graphs/routes/renamed.json"),
                renamed_revision,
            )
            .unwrap();
        assert_eq!(
            store.load_asset("graph.ordon").unwrap().1,
            Path::new("route-graphs/routes/renamed.json")
        );

        let moved_revision = store.load_asset("graph.ordon").unwrap().0.digest().unwrap();
        store
            .delete_to_trash("graph.ordon", moved_revision, true)
            .unwrap();
        let trash = store.list_trash().unwrap();
        let trashed = trash
            .iter()
            .find(|listing| listing.id == "graph.ordon")
            .unwrap();
        store
            .permanently_delete_from_trash("graph.ordon", trashed.revision_sha256)
            .unwrap();
        assert!(
            store
                .list_trash()
                .unwrap()
                .iter()
                .all(|listing| listing.id != "graph.ordon")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn folder_crud_preserves_identity_remaps_clones_and_trashes_the_subtree() {
        let root = temporary_directory("folder-crud");
        let manifest = WorkspaceManifest::new("workspace.test", "Test workspace").unwrap();
        let store = WorkspaceStore::create(&root, manifest).unwrap();
        let parent_revision = store
            .create_folder(
                "folder.routes",
                "Routes",
                WorkspaceAssetKind::RouteGraph,
                None,
                "routes",
            )
            .unwrap();
        store
            .create_folder(
                "folder.routes.ordon",
                "Ordon",
                WorkspaceAssetKind::RouteGraph,
                Some("folder.routes"),
                "ordon",
            )
            .unwrap();

        let child = graph_asset("graph.child", "Child graph");
        store
            .save_asset(
                Path::new("route-graphs/routes/ordon/child.json"),
                None,
                &child,
            )
            .unwrap();
        let mut parent = graph_asset("graph.parent", "Parent graph");
        parent.references.push(WorkspaceAssetReference {
            asset_id: child.header.id.clone(),
            kind: WorkspaceAssetKind::RouteGraph,
        });
        store
            .save_asset(Path::new("route-graphs/routes/parent.json"), None, &parent)
            .unwrap();
        let layout = WorkspaceAsset {
            schema: WORKSPACE_ASSET_SCHEMA.into(),
            header: WorkspaceAssetHeader {
                id: "layout.external".into(),
                label: "External layout".into(),
                kind: WorkspaceAssetKind::Layout,
                version: 1,
                origin: None,
            },
            references: vec![WorkspaceAssetReference {
                asset_id: parent.header.id.clone(),
                kind: WorkspaceAssetKind::RouteGraph,
            }],
            payload: WorkspaceAssetPayload::Layout(LayoutAsset {
                semantic_asset_id: parent.header.id.clone(),
                positions: BTreeMap::new(),
                viewport: None,
            }),
        };
        store
            .save_asset(Path::new("layouts/external.json"), None, &layout)
            .unwrap();

        let renamed_revision = store
            .rename_folder(
                "folder.routes",
                "Main routes",
                "main-routes",
                parent_revision,
            )
            .unwrap();
        assert_ne!(renamed_revision, parent_revision);
        assert_eq!(
            store.load_folder("folder.routes").unwrap().1,
            Path::new("route-graphs/main-routes")
        );

        store
            .duplicate_folder(
                "folder.routes",
                "folder.routes-copy",
                "Routes copy",
                None,
                "routes-copy",
            )
            .unwrap();
        let cloned_assets = store
            .list_assets()
            .unwrap()
            .into_iter()
            .filter(|asset| asset.relative_path.starts_with("route-graphs/routes-copy"))
            .collect::<Vec<_>>();
        assert_eq!(cloned_assets.len(), 2);
        let cloned_parent = cloned_assets
            .iter()
            .find(|asset| asset.label == "Parent graph")
            .unwrap();
        let cloned_child = cloned_assets
            .iter()
            .find(|asset| asset.label == "Child graph")
            .unwrap();
        let cloned_parent_asset = store.load_asset(&cloned_parent.id).unwrap().0;
        assert_eq!(cloned_parent_asset.references[0].asset_id, cloned_child.id);
        assert_ne!(cloned_parent.id, parent.header.id);

        let error = store
            .delete_folder_to_trash("folder.routes", renamed_revision, false)
            .unwrap_err();
        assert!(error.to_string().contains("layout.external"));
        store
            .delete_folder_to_trash("folder.routes", renamed_revision, true)
            .unwrap();
        assert!(store.load_folder("folder.routes").is_err());
        assert!(store.load_asset("graph.parent").is_err());
        let trash = store.list_folder_trash().unwrap();
        assert_eq!(trash.len(), 1);
        assert_eq!(trash[0].folder_count, 2);
        assert_eq!(trash[0].asset_count, 2);
        store
            .restore_folder_from_trash("folder.routes", trash[0].revision_sha256)
            .unwrap();
        assert_eq!(
            store.load_asset("graph.child").unwrap().1,
            Path::new("route-graphs/main-routes/ordon/child.json")
        );

        let restored_revision = store
            .load_folder("folder.routes")
            .unwrap()
            .0
            .digest()
            .unwrap();
        store
            .delete_folder_to_trash("folder.routes", restored_revision, true)
            .unwrap();
        let trash = store.list_folder_trash().unwrap();
        store
            .permanently_delete_folder_from_trash("folder.routes", trash[0].revision_sha256)
            .unwrap();
        assert!(store.list_folder_trash().unwrap().is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn folder_move_rejects_cross_type_and_descendant_parents() {
        let root = temporary_directory("folder-move-validation");
        let manifest = WorkspaceManifest::new("workspace.test", "Test workspace").unwrap();
        let store = WorkspaceStore::create(&root, manifest).unwrap();
        let parent_revision = store
            .create_folder(
                "folder.parent",
                "Parent",
                WorkspaceAssetKind::RouteGraph,
                None,
                "parent",
            )
            .unwrap();
        store
            .create_folder(
                "folder.child",
                "Child",
                WorkspaceAssetKind::RouteGraph,
                Some("folder.parent"),
                "child",
            )
            .unwrap();
        store
            .create_folder(
                "folder.layouts",
                "Layouts",
                WorkspaceAssetKind::Layout,
                None,
                "layouts",
            )
            .unwrap();

        assert!(
            store
                .move_folder("folder.parent", Some("folder.child"), parent_revision)
                .unwrap_err()
                .to_string()
                .contains("descendant")
        );
        assert!(
            store
                .move_folder("folder.parent", Some("folder.layouts"), parent_revision)
                .unwrap_err()
                .to_string()
                .contains("cannot be placed")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn trashed_child_restores_below_its_stable_parent_after_parent_move() {
        let root = temporary_directory("folder-restore-parent-move");
        let manifest = WorkspaceManifest::new("workspace.test", "Test workspace").unwrap();
        let store = WorkspaceStore::create(&root, manifest).unwrap();
        let parent_revision = store
            .create_folder(
                "folder.parent",
                "Parent",
                WorkspaceAssetKind::RouteGraph,
                None,
                "parent",
            )
            .unwrap();
        let child_revision = store
            .create_folder(
                "folder.child",
                "Child",
                WorkspaceAssetKind::RouteGraph,
                Some("folder.parent"),
                "child",
            )
            .unwrap();
        store
            .create_folder(
                "folder.archive",
                "Archive",
                WorkspaceAssetKind::RouteGraph,
                None,
                "archive",
            )
            .unwrap();
        store
            .delete_folder_to_trash("folder.child", child_revision, false)
            .unwrap();
        store
            .move_folder("folder.parent", Some("folder.archive"), parent_revision)
            .unwrap();
        let trash = store.list_folder_trash().unwrap();
        store
            .restore_folder_from_trash("folder.child", trash[0].revision_sha256)
            .unwrap();

        assert_eq!(
            store.load_folder("folder.child").unwrap().1,
            Path::new("route-graphs/archive/parent/child")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn open_recovers_completed_folder_restore_and_abandoned_clone_staging() {
        let root = temporary_directory("folder-operation-recovery");
        let manifest = WorkspaceManifest::new("workspace.test", "Test workspace").unwrap();
        let store = WorkspaceStore::create(&root, manifest).unwrap();
        let revision = store
            .create_folder(
                "folder.routes",
                "Routes",
                WorkspaceAssetKind::RouteGraph,
                None,
                "routes",
            )
            .unwrap();
        store
            .delete_folder_to_trash("folder.routes", revision, false)
            .unwrap();
        let group = store.folder_trash_group("folder.routes").unwrap();
        fs::rename(
            group.join(FOLDER_TRASH_PAYLOAD),
            root.join("route-graphs/routes"),
        )
        .unwrap();
        let abandoned = root.join(FOLDER_STAGING_ROOT).join("folder-abandoned");
        fs::create_dir(&abandoned).unwrap();
        fs::write(abandoned.join(FOLDER_MARKER_FILE), b"incomplete").unwrap();
        drop(store);

        let recovered = WorkspaceStore::open(&root, &BTreeMap::new()).unwrap();
        assert!(recovered.load_folder("folder.routes").is_ok());
        assert!(recovered.list_folder_trash().unwrap().is_empty());
        assert!(!abandoned.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn registry_export_import_round_trips_independent_assets() {
        let root = temporary_directory("workspace-export-import");
        let registry = WorkspaceRegistry::open(root.join("workspaces"), BTreeMap::new()).unwrap();
        registry
            .create(WorkspaceCreateRequest {
                schema: WORKSPACE_CREATE_SCHEMA.into(),
                id: "source".into(),
                label: "Source workspace".into(),
            })
            .unwrap();
        registry
            .command_folder(
                "source",
                "folder.routes",
                WorkspaceFolderCommandRequest {
                    schema: WORKSPACE_FOLDER_COMMAND_SCHEMA.into(),
                    command: WorkspaceFolderCommand::Create {
                        id: "folder.routes".into(),
                        label: "Routes".into(),
                        asset_kind: WorkspaceAssetKind::RouteGraph,
                        parent_id: None,
                        directory_name: "routes".into(),
                    },
                },
            )
            .unwrap();
        let asset = graph_asset("graph.ordon", "Ordon route");
        registry
            .save_asset(
                "source",
                "graph.ordon",
                WorkspaceAssetSaveRequest {
                    schema: WORKSPACE_ASSET_SAVE_SCHEMA.into(),
                    relative_path: "route-graphs/routes/ordon.json".into(),
                    expected_revision_sha256: None,
                    asset: asset.clone(),
                },
            )
            .unwrap();

        let mut bundle = registry.export("source").unwrap();
        assert_eq!(bundle.schema, WORKSPACE_EXPORT_SCHEMA);
        assert_eq!(bundle.folders.len(), 1);
        assert_eq!(bundle.folders[0].folder.id, "folder.routes");
        assert_eq!(bundle.assets.len(), 1);
        assert_eq!(
            bundle.assets[0].relative_path,
            Path::new("route-graphs/routes/ordon.json")
        );
        assert_eq!(bundle.assets[0].asset, asset);

        bundle.manifest.id = "imported".into();
        bundle.manifest.label = "Imported workspace".into();
        let imported = registry.import(bundle.clone()).unwrap();
        assert_eq!(imported.manifest.id, "imported");
        assert_eq!(imported.folders.len(), 1);
        assert_eq!(imported.folders[0].id, "folder.routes");
        assert_eq!(imported.assets.len(), 1);
        assert_eq!(imported.assets[0].id, "graph.ordon");
        assert_eq!(
            imported.assets[0].relative_path,
            Path::new("route-graphs/routes/ordon.json")
        );
        assert_eq!(
            registry
                .load_asset("imported", "graph.ordon")
                .unwrap()
                .asset,
            asset
        );
        assert!(
            registry
                .import(bundle)
                .unwrap_err()
                .to_string()
                .contains("already exists")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn registry_import_rejects_duplicate_paths_before_creating_workspace() {
        let root = temporary_directory("workspace-import-collision");
        let registry = WorkspaceRegistry::open(root.join("workspaces"), BTreeMap::new()).unwrap();
        let manifest = WorkspaceManifest::new("collision", "Collision").unwrap();
        let bundle = WorkspaceExport {
            schema: WORKSPACE_EXPORT_SCHEMA.into(),
            manifest,
            folders: Vec::new(),
            assets: vec![
                WorkspaceExportAsset {
                    relative_path: "route-graphs/same.json".into(),
                    asset: graph_asset("graph.first", "First"),
                },
                WorkspaceExportAsset {
                    relative_path: "route-graphs/same.json".into(),
                    asset: graph_asset("graph.second", "Second"),
                },
            ],
        };

        let error = registry.import(bundle).unwrap_err();
        assert!(error.to_string().contains("duplicates asset path"));
        assert!(!root.join("workspaces/collision").exists());
        fs::remove_dir_all(root).unwrap();
    }
}
