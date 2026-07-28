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
pub const WORKSPACE_SCENARIO_CREATE_SCHEMA: &str =
    "dusklight.route-planner.workspace-scenario-create/v1";
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

struct BlankScenarioConfiguration<'a> {
    label: &'a str,
    goal_id: &'a str,
}

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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceScenarioCreateRequest {
    pub schema: String,
    pub library_id: String,
    pub namespace: String,
    pub label: String,
    pub goal_id: String,
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

mod assets;
mod folders;
mod helpers;
mod registry;
mod templates;
mod transactions;
mod validation;

use helpers::*;

#[cfg(test)]
mod tests;
