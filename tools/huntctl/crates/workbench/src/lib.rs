//! Local, Git-backed route workbench primitives.
//!
//! Authored `.timeline` files and the artifacts they name are authoritative.
//! It projects timeline topology as graph JSON, materializes exact parent chains and named
//! continuations, and offers revision-checked edits for labels, segment subtrees, and the
//! timeline-configured milestone program. Segment input artifacts remain independent Git objects.

// Keep the implementation's domain vocabulary explicit while its public API migrates out of the
// historical root crate. These are dependencies, not callbacks into the huntctl executable.
pub use dusklight_automation_contracts::{artifact, scenario_fixture, tape};
pub use dusklight_control::{
    option_diagnostics, option_execution, tape_chain, tape_dsl, tape_program,
};
pub use dusklight_evidence::content_store;
pub use dusklight_harness_contracts::evaluation as search_evaluator;
pub use dusklight_objectives::milestone_dsl;
pub use dusklight_routes::timeline;
pub use dusklight_search::search;

mod graph_projection;
mod inventory_catalog;
mod project_catalog;
mod server;
mod stage_catalog;

pub use graph_projection::{
    ThumbnailPruneEntry, ThumbnailPruneReport, graph_from_timeline, prune_thumbnails,
};
pub use server::serve;

use graph_projection::*;
use project_catalog::*;

#[cfg(test)]
use server::{HttpResponse, handle_http, origin_allowed, thumbnail_response};

use crate::content_store::{ContentKind, ContentStore};
use crate::milestone_dsl::MilestoneProgram;
use crate::option_diagnostics::{OptionDiagnosticBundle, OptionVisualization};
use crate::search::{Candidate, SearchResults};
use crate::search_evaluator::{AnchoredObjectiveIdentity, BoundaryFingerprint};
use crate::tape::{InputTape, TapeBoot};
use crate::tape_chain::{ChainSegment, SegmentFrames, concatenate};
use crate::timeline::{ArtifactSource, ResolvedLineage, Segment, Timeline, tokenize};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const GRAPH_SCHEMA: &str = "dusklight.route-workbench.graph.v9";
const PROJECT_CATALOG_SCHEMA: &str = "dusklight.route-workbench.workspace.v2";
const PROJECT_WORKSPACE_PATH: &str = "routes";
const DRAFT_SCHEMA: &str = "dusklight.route-workbench.draft.v2";
const DRAFT_MANIFEST: &str = "draft.json";
const DRAFT_FINAL_MANIFEST: &str = "draft.final.json";
const DRAFT_LAUNCH: &str = "launch.json";
const DRAFT_TAPE: &str = "continuation.tape";
const DRAFT_TERMINAL_THUMBNAIL: &str = "terminal.png";
const DRAFT_TRASH_DIRECTORY: &str = "trash";
const GENERATED_SEARCH_TOMBSTONES: &str = "generated-search-tombstones.json";
const GENERATED_SEARCH_TOMBSTONE_SCHEMA: &str =
    "dusklight.route-workbench.generated-search-tombstones.v1";
const DRAFT_DELETE_PREVIEW_SCHEMA: &str = "dusklight.route-workbench.delete-preview.v1";
const DRAFT_DELETE_RESULT_SCHEMA: &str = "dusklight.route-workbench.delete-result.v1";
const DRAFT_RENAME_RESULT_SCHEMA: &str = "dusklight.route-workbench.rename-result.v1";
const SEGMENT_RENAME_RESULT_SCHEMA: &str = "dusklight.route-workbench.segment-rename-result.v1";
const SEGMENT_DELETE_PREVIEW_SCHEMA: &str = "dusklight.route-workbench.segment-delete-preview.v1";
const SIBLING_DELETE_PREVIEW_SCHEMA: &str = "dusklight.route-workbench.sibling-delete-preview.v1";
const SEGMENT_DELETE_RESULT_SCHEMA: &str = "dusklight.route-workbench.segment-delete-result.v1";
const SIBLING_DELETE_RESULT_SCHEMA: &str = "dusklight.route-workbench.sibling-delete-result.v1";
const MILESTONE_PROGRAM_SCHEMA: &str = "dusklight.route-workbench.milestone-program.v1";
const THUMBNAIL_CAPTURE_SCHEMA: &str = "dusklight.route-workbench.thumbnail-capture.v1";
const THUMBNAIL_PRUNE_SCHEMA: &str = "dusklight.route-workbench.thumbnail-prune.v1";
const THUMBNAIL_DIRECTORY: &str = "thumbnails";
const MAX_THUMBNAIL_BYTES: u64 = 2 * 1024 * 1024;
const THUMBNAIL_WIDTH: u32 = 320;
const THUMBNAIL_HEIGHT: u32 = 240;
const MAX_DRAFTS: usize = 10_000;
const MAX_SEARCH_RUNS: usize = 1_000;
const MAX_SEARCH_ARTIFACT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_OPTION_DIAGNOSTIC_BYTES: u64 = 64 * 1024 * 1024;
const MAX_GENERATED_SEGMENTS: usize = 16;
const GENERATED_SEGMENTS_PER_RUN: usize = 4;
const MAX_HTTP_HEADER: usize = 64 * 1024;
const MAX_HTTP_BODY: usize = 1024 * 1024;

fn active_recordings() -> &'static Mutex<BTreeSet<String>> {
    static ACTIVE: OnceLock<Mutex<BTreeSet<String>>> = OnceLock::new();
    ACTIVE.get_or_init(|| Mutex::new(BTreeSet::new()))
}

fn milestone_program_edits() -> &'static Mutex<()> {
    static EDITS: OnceLock<Mutex<()>> = OnceLock::new();
    EDITS.get_or_init(|| Mutex::new(()))
}

fn timeline_edits() -> &'static Mutex<()> {
    static EDITS: OnceLock<Mutex<()>> = OnceLock::new();
    EDITS.get_or_init(|| Mutex::new(()))
}

#[derive(Clone, Debug, Serialize)]
pub struct WorkbenchGraph {
    pub schema: String,
    pub timeline: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<GraphOrigin>,
    pub segments: Vec<GraphSegment>,
    pub goals: Vec<GraphGoal>,
    pub drafts: Vec<GraphDraft>,
    pub projects: GraphProjectCatalog,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub draft_graph_revision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub predicate_program: Option<GraphPredicateProgram>,
}

fn workspace_edits() -> &'static Mutex<()> {
    static EDITS: OnceLock<Mutex<()>> = OnceLock::new();
    EDITS.get_or_init(|| Mutex::new(()))
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct GraphProjectCatalog {
    pub schema: String,
    pub groups: Vec<GraphProjectGroup>,
    pub entries: Vec<GraphProject>,
    pub stages: Vec<GraphStageSummary>,
    pub inventory_items: Vec<GraphInventoryItem>,
    pub inventory_slots: Vec<GraphInventorySlot>,
}

#[derive(Clone, Debug, Serialize)]
pub struct GraphInventoryItem {
    pub id: u16,
    pub name: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct GraphInventorySlot {
    pub id: u16,
    pub default_item: u16,
    pub quantity: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct GraphStageSummary {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub friendly_name: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct GraphStageBootOptions {
    pub stage: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub friendly_name: Option<String>,
    pub inventory_indexed: bool,
    pub rooms: Vec<GraphStageRoomBootOptions>,
}

#[derive(Clone, Debug, Serialize)]
pub struct GraphStageRoomBootOptions {
    pub id: i8,
    pub spawn_points: Vec<i16>,
    pub layers: Vec<i8>,
}

#[derive(Clone, Debug, Serialize)]
pub struct GraphProjectGroup {
    pub id: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct GraphProject {
    pub id: String,
    pub label: String,
    pub group: String,
    pub kind: String,
    pub active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub boot_override: Option<GraphBootOverride>,
    pub artifact: GraphArtifact,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub boot: Option<TapeBoot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub materialization_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<String>,
    pub playable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixture_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GraphBootOverride {
    pub enabled: bool,
    pub boot: TapeBoot,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserBootOverrideUpdateRequest {
    pub project: String,
    pub enabled: bool,
    pub boot: TapeBoot,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserStageBootOptionsRequest {
    pub stage: String,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceNodeKind {
    Folder,
    Project,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserWorkspaceFolderCreateRequest {
    pub parent: String,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserWorkspaceTapeCreateRequest {
    pub parent: String,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserWorkspaceTapeCloneRequest {
    pub source: String,
    pub destination: String,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserWorkspaceMoveRequest {
    pub id: String,
    pub kind: WorkspaceNodeKind,
    pub destination: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserWorkspaceDeleteRequest {
    pub id: String,
    pub kind: WorkspaceNodeKind,
}

#[derive(Clone, Debug, Serialize)]
pub struct WorkspaceMutationResult {
    pub operation: String,
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trash: Option<PathBuf>,
}

#[derive(Clone, Debug, Serialize)]
pub struct GraphPredicateProgram {
    pub schema: String,
    pub source: String,
    pub revision_sha256: String,
    pub program_sha256: String,
    pub definitions: Vec<GraphPredicate>,
}

#[derive(Clone, Debug, Serialize)]
pub struct GraphPredicate {
    pub name: String,
    pub phase: milestone_dsl::EvaluationPhase,
    pub stable_ticks: u16,
    pub expression: milestone_dsl::Expression,
    pub then: Vec<milestone_dsl::Expression>,
    pub within_ticks: Option<u16>,
    pub definition_sha256: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct GraphDraft {
    pub id: String,
    pub label: String,
    pub parent: DraftParent,
    pub created_unix_ms: u64,
    pub status: DraftStatus,
    pub frame_count: Option<u64>,
    pub playable: bool,
    pub endpoint_kind: String,
    pub verification: String,
    pub tape_sha256: Option<String>,
    pub result_tape_sha256: Option<String>,
    pub tape_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DraftParent {
    Milestone {
        id: String,
        program_sha256: String,
        definition_sha256: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        boundary_fingerprint: Option<String>,
    },
    Segment {
        id: String,
        terminal_milestone: String,
        boundary_fingerprint: String,
    },
    Draft {
        id: String,
        parent_tape_sha256: String,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DraftStatus {
    Preparing,
    Recording,
    Orphaned,
    Ready,
    ZeroFrames,
    NeverReachedHandoff,
    CapacityExhausted,
    WriteFailure,
    StartBoundaryMismatch,
    ProcessFailure,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeRecordStatus {
    schema: String,
    status: String,
    tape: String,
    frame_count: u64,
    frame_capacity: u64,
    handoff_reached: bool,
    capacity_exhausted: bool,
    error: Option<String>,
    process_success: bool,
    session_token: Option<String>,
    start_milestone: Option<String>,
    start_fingerprint: Option<String>,
    #[serde(default)]
    expected_start_fingerprint: Option<String>,
    #[serde(default)]
    start_boundary_kind: Option<String>,
    #[serde(default)]
    start_boundary_index: Option<u64>,
    #[serde(default)]
    start_program_digest: Option<String>,
    #[serde(default)]
    start_definition_digest: Option<String>,
    start_tape_frame: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DraftManifest {
    schema: String,
    id: String,
    label: String,
    parent: DraftParent,
    parent_tape_sha256: String,
    created_unix_ms: u64,
    session_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    expected_start_milestone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expected_start_fingerprint: Option<String>,
    tape: String,
    status: DraftStatus,
    endpoint_kind: String,
    verification: String,
    start_boundary_verified: bool,
    #[serde(default)]
    accelerated_parent_replay: bool,
    parent_frames: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    tape_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tape_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result_tape_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    frames: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DraftLaunch {
    schema: String,
    id: String,
    pid: u32,
    session_token: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct GraphOrigin {
    pub id: String,
    pub predicate: String,
    pub predicate_program: GraphPredicateProgram,
    pub recordable_from_boot: bool,
    pub configurations: Vec<TapeBoot>,
}

#[derive(Clone, Debug, Serialize)]
pub struct GraphSegment {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    pub profile: String,
    pub artifact: GraphArtifact,
    pub start_fingerprint: String,
    pub boundary_fingerprint: String,
    pub goal_proofs: Vec<GraphGoalProof>,
    pub predicate_proof: String,
    pub first_hit_tick: Option<u64>,
    pub frame_count: Option<u64>,
    pub start_tick: u64,
    pub end_tick: Option<u64>,
    pub ticks: Option<u64>,
    pub playable: bool,
    pub recordable: bool,
    pub record_anchors: Vec<GraphRecordAnchor>,
    pub option_visualization: Vec<OptionVisualization>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub option_diagnostic_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generated: Option<GraphGeneratedSegment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct GraphGeneratedSegment {
    pub kind: String,
    pub status: String,
    pub uncommitted: bool,
    pub run: String,
    pub generation: u32,
    pub candidate_id: String,
    pub candidate: String,
    pub tape: String,
    pub objective_sha256: String,
    pub source_predicate: String,
    pub goal_predicate: String,
    pub proof_attempts: u32,
}

#[derive(Debug, Deserialize)]
struct GeneratedAnchoredResults {
    schema: String,
    objective: AnchoredObjectiveIdentity,
    results: SearchResults,
}

#[derive(Debug, Deserialize)]
struct GeneratedAttempt {
    candidate_id: String,
    tape: PathBuf,
    exit_code: Option<i32>,
    infrastructure_error: Option<String>,
    first_hit_tick: Option<u64>,
    goal_reached: bool,
    boundary_fingerprints: BTreeMap<String, BoundaryFingerprint>,
}

struct GeneratedProjection {
    segment: GraphSegment,
    full_tape: PathBuf,
}

#[derive(Clone, Debug, Serialize)]
pub struct GraphGoal {
    pub id: String,
    pub segment: String,
    pub predicate: String,
    pub predicate_program: GraphPredicateProgram,
}

#[derive(Clone, Debug, Serialize)]
pub struct GraphGoalProof {
    pub goal: String,
    pub predicate: String,
    pub program_sha256: String,
    pub definition_sha256: String,
    pub status: String,
    pub first_hit_tick: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct GraphRecordAnchor {
    pub goal: String,
    pub predicate: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct GraphArtifact {
    pub kind: String,
    pub value: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayRequest {
    #[serde(default)]
    pub lineage: Option<String>,
    #[serde(default)]
    pub standalone_segment: Option<String>,
    #[serde(default)]
    pub through_segment: Option<String>,
    #[serde(default)]
    pub segment: Option<String>,
    #[serde(default)]
    pub frame: Option<u64>,
    #[serde(default = "default_takeover")]
    pub takeover: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserPlayRequest {
    pub selection: BrowserSelection,
    pub stop: BrowserStop,
    #[serde(default = "default_takeover")]
    pub handoff: bool,
    #[serde(default)]
    pub mode: PlaybackMode,
    #[serde(
        default = "default_speed_percent",
        deserialize_with = "deserialize_playback_speed_percent"
    )]
    pub speed_percent: u16,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackMode {
    #[default]
    Playback,
    ResumeAccelerated,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackOrigin {
    #[default]
    Boot,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BrowserSelection {
    Draft { id: String },
    Segment { id: String },
    Project { id: String },
}

const DEFAULT_RECORD_INPUT_COUNTDOWN_SECONDS: u8 = 3;
const MAX_RECORD_INPUT_COUNTDOWN_SECONDS: u8 = 10;
const RECORDING_SPEED_PERCENTAGES: &[u16] = &[1, 5, 10, 20, 30, 40, 50, 60, 70, 80, 90, 100];
const PLAYBACK_SPEED_PERCENTAGES: &[u16] = &[
    0, 1, 5, 10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 125, 150, 175, 200, 300, 400,
];

fn default_speed_percent() -> u16 {
    100
}

fn deserialize_speed_percent<'de, D>(
    deserializer: D,
    allowed: &[u16],
    description: &str,
) -> Result<u16, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let percent = u16::deserialize(deserializer)?;
    if !allowed.contains(&percent) {
        return Err(serde::de::Error::custom(format!(
            "{description} speed percentage {percent} is not supported"
        )));
    }
    Ok(percent)
}

fn deserialize_playback_speed_percent<'de, D>(deserializer: D) -> Result<u16, D::Error>
where
    D: serde::Deserializer<'de>,
{
    deserialize_speed_percent(deserializer, PLAYBACK_SPEED_PERCENTAGES, "playback")
}

fn deserialize_recording_speed_percent<'de, D>(deserializer: D) -> Result<u16, D::Error>
where
    D: serde::Deserializer<'de>,
{
    deserialize_speed_percent(deserializer, RECORDING_SPEED_PERCENTAGES, "recording")
}

fn default_record_input_countdown_seconds() -> u8 {
    DEFAULT_RECORD_INPUT_COUNTDOWN_SECONDS
}

fn deserialize_record_input_countdown_seconds<'de, D>(deserializer: D) -> Result<u8, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let seconds = u8::deserialize(deserializer)?;
    if seconds > MAX_RECORD_INPUT_COUNTDOWN_SECONDS {
        return Err(serde::de::Error::custom(format!(
            "recording handoff countdown must be between 0 and {MAX_RECORD_INPUT_COUNTDOWN_SECONDS} seconds"
        )));
    }
    Ok(seconds)
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserRecordRequest {
    pub parent: BrowserRecordParent,
    #[serde(default)]
    pub label: String,
    #[serde(
        default = "default_record_input_countdown_seconds",
        deserialize_with = "deserialize_record_input_countdown_seconds"
    )]
    pub countdown_seconds: u8,
    #[serde(
        default = "default_speed_percent",
        deserialize_with = "deserialize_recording_speed_percent"
    )]
    pub speed_percent: u16,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserThumbnailCaptureRequest {
    pub selection: BrowserSelection,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserDraftDeletePreviewRequest {
    pub id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserDraftDeleteApplyRequest {
    pub id: String,
    pub confirmation_token: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserDraftRenameRequest {
    pub id: String,
    pub label: String,
    pub expected_graph_revision: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserSegmentRenameRequest {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub expected_name: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserSegmentDeletePreviewRequest {
    pub id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserSegmentDeleteApplyRequest {
    pub id: String,
    pub confirmation_token: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserSiblingDeletePreviewRequest {
    pub keep_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserSiblingDeleteApplyRequest {
    pub keep_id: String,
    pub confirmation_token: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserMilestoneProgramUpdateRequest {
    #[serde(default)]
    pub owner: String,
    pub expected_revision_sha256: String,
    pub source: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct DraftDeleteImpact {
    pub id: String,
    pub label: String,
    pub status: DraftStatus,
}

#[derive(Clone, Debug, Serialize)]
pub struct DraftDeletePreview {
    pub schema: String,
    pub id: String,
    pub graph_revision: String,
    pub drafts: Vec<DraftDeleteImpact>,
    pub confirmation_token: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct DraftDeleteResult {
    pub schema: String,
    pub id: String,
    pub graph_revision: String,
    pub drafts: Vec<String>,
    pub trash_transaction: PathBuf,
}

#[derive(Clone, Debug, Serialize)]
pub struct DraftRenameResult {
    pub schema: String,
    pub id: String,
    pub label: String,
    pub graph_revision: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct SegmentRenameResult {
    pub schema: String,
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct SegmentDeleteImpact {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct SegmentDeletePreview {
    pub schema: String,
    pub id: String,
    pub segments: Vec<SegmentDeleteImpact>,
    pub goals: Vec<String>,
    pub proofs: usize,
    pub lineages: Vec<String>,
    pub drafts: Vec<DraftDeleteImpact>,
    pub confirmation_token: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct SegmentDeleteResult {
    pub schema: String,
    pub id: String,
    pub segments: Vec<String>,
    pub drafts: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trash_transaction: Option<PathBuf>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SiblingDeletePreview {
    pub schema: String,
    pub keep_id: String,
    pub sibling_roots: Vec<SegmentDeleteImpact>,
    pub draft_roots: Vec<DraftDeleteImpact>,
    pub generated: Vec<GeneratedDeleteImpact>,
    pub segments: Vec<SegmentDeleteImpact>,
    pub goals: Vec<String>,
    pub proofs: usize,
    pub lineages: Vec<String>,
    pub drafts: Vec<DraftDeleteImpact>,
    pub confirmation_token: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct SiblingDeleteResult {
    pub schema: String,
    pub keep_id: String,
    pub sibling_roots: Vec<String>,
    pub draft_roots: Vec<String>,
    pub generated_candidates: Vec<String>,
    pub segments: Vec<String>,
    pub drafts: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trash_transaction: Option<PathBuf>,
}

#[derive(Clone, Debug, Serialize)]
pub struct GeneratedDeleteImpact {
    pub id: String,
    pub name: String,
    pub candidate_id: String,
    pub run: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct GeneratedSearchTombstones {
    schema: String,
    candidate_ids: BTreeSet<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BrowserRecordParent {
    Origin { id: String },
    Segment { id: String, terminal_goal: String },
    Draft { id: String },
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BrowserStop {
    Segment { segment: String },
    Tick { tick: u64 },
}

fn default_takeover() -> bool {
    true
}

#[derive(Clone, Debug, Serialize)]
pub struct PlayResponse {
    pub pid: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lineage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segment: Option<String>,
    pub tape: PathBuf,
    pub session_state_root: PathBuf,
    pub session_id: String,
    pub frames: u64,
    pub input_tape_end: String,
    pub origin: PlaybackOrigin,
    pub speed_percent: u16,
    pub fast: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fast_forward_frames: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<String>,
}

#[derive(Clone, Copy, Debug)]
struct PlaybackSettings {
    speed_percent: u16,
    fast: bool,
}

#[derive(Debug)]
struct MaterializedLaunchOptions {
    takeover: bool,
    origin: PlaybackOrigin,
    fast_forward_frames: Option<u64>,
    thumbnail: Option<PlaybackThumbnailCapture>,
    playback: PlaybackSettings,
}

#[derive(Clone, Copy, Debug)]
struct PlaybackCliOptions<'a> {
    seed_stage: Option<&'a str>,
    fast_forward_frames: Option<u64>,
    playback: PlaybackSettings,
}

#[derive(Clone, Copy, Debug)]
struct SegmentPlaybackOptions {
    handoff: bool,
    playback: PlaybackSettings,
}

#[derive(Clone, Debug, Serialize)]
pub struct RecordResponse {
    pub pid: u32,
    pub draft_id: String,
    pub manifest: PathBuf,
    pub tape: PathBuf,
    pub frames_before_recording: u64,
    pub speed_percent: u16,
}

#[derive(Clone, Debug, Serialize)]
pub struct ThumbnailCaptureResponse {
    pub schema: String,
    pub pid: u32,
    pub key: String,
    pub thumbnail: String,
    pub frames: u64,
}

#[derive(Clone, Debug)]
pub enum MaterializeTarget {
    FullLineage,
    ThroughSegment(String),
    ThroughSegmentFrame { segment: String, frame: u64 },
    ThroughStepCount(usize),
}

#[derive(Clone, Debug)]
pub struct MaterializedLineage {
    pub lineage: String,
    pub tape: InputTape,
    pub steps: Vec<MaterializedStep>,
}

#[derive(Clone, Debug)]
struct MaterializedPlayback {
    lineage: Option<String>,
    segment: Option<String>,
    tape: InputTape,
    seed_stage: Option<&'static str>,
    native_oracle: NativePlaybackOracle,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum NativePlaybackOracle {
    #[default]
    None,
    EyeShredder,
}

fn legacy_seed_stage(
    tape: &InputTape,
    profile: crate::search::SegmentProfile,
) -> Option<&'static str> {
    (tape.boot == TapeBoot::Process && profile == crate::search::SegmentProfile::Fsp103ToFsp104)
        .then_some("F_SP103,1,1,3")
}

#[derive(Clone, Debug)]
struct PlaybackThumbnailCapture {
    path: PathBuf,
    url: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct MaterializedStep {
    pub segment: String,
    pub source_start_frame: u64,
    pub source_end_frame: u64,
    pub chain_start_frame: u64,
    pub chain_end_frame: u64,
}

#[derive(Clone, Debug)]
pub struct WorkbenchConfig {
    pub timeline_path: PathBuf,
    pub repository_root: PathBuf,
    pub working_directory: PathBuf,
    pub game: PathBuf,
    pub dvd: PathBuf,
    /// Parent directory only. Cold playback always gets a fresh child.
    pub state_root: PathBuf,
}

#[derive(Debug)]
pub struct WorkbenchError(String);

impl WorkbenchError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for WorkbenchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for WorkbenchError {}

mod milestone_program;
pub use milestone_program::*;
mod draft_store;
use draft_store::*;
mod playback;
pub use playback::*;
#[cfg(test)]
mod tests;
