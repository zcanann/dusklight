//! Local, Git-backed route workbench primitives.
//!
//! Authored `.timeline` files and the artifacts they name are authoritative.
//! It projects timeline topology as graph JSON, materializes exact parent chains and named
//! continuations, and offers revision-checked edits for labels, segment subtrees, and the
//! timeline-configured milestone program. Segment input artifacts remain independent Git objects.

// Keep the implementation's domain vocabulary explicit while its public API migrates out of the
// historical root crate. These are dependencies, not callbacks into the huntctl executable.
pub use dusklight_automation_contracts::{artifact, tape};
pub use dusklight_control::{
    option_diagnostics, option_execution, tape_chain, tape_dsl, tape_program,
};
pub use dusklight_evidence::content_store;
pub use dusklight_harness_contracts::evaluation as search_evaluator;
pub use dusklight_objectives::milestone_dsl;
pub use dusklight_routes::timeline;
pub use dusklight_search::search;

mod server;

pub use server::serve;

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

const GRAPH_SCHEMA: &str = "dusklight.route-workbench.graph.v8";
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub draft_graph_revision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub predicate_program: Option<GraphPredicateProgram>,
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
    pub recordable_from_boot: bool,
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
    pub origin: PlaybackOrigin,
    #[serde(
        default = "default_speed_percent",
        deserialize_with = "deserialize_playback_speed_percent"
    )]
    pub speed_percent: u16,
    #[serde(default)]
    pub fast: bool,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackOrigin {
    #[default]
    Boot,
    Parent,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BrowserSelection {
    Draft { id: String },
    Segment { id: String },
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
    origin: PlaybackOrigin,
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

fn configured_artifact_root(config: &WorkbenchConfig) -> Result<PathBuf, WorkbenchError> {
    let repository = fs::canonicalize(&config.repository_root).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot resolve repository root {}: {error}",
            config.repository_root.display()
        ))
    })?;
    let timeline = fs::canonicalize(&config.timeline_path).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot resolve timeline {}: {error}",
            config.timeline_path.display()
        ))
    })?;
    if !timeline.starts_with(&repository) {
        return Err(WorkbenchError::new(format!(
            "timeline {} is outside repository {}",
            timeline.display(),
            repository.display()
        )));
    }
    timeline
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| WorkbenchError::new("timeline has no parent directory"))
}

/// Parse the checked-in timeline on every request so edits in the Git working
/// tree are visible without synchronizing a secondary store.
pub fn load_authoritative_timeline(path: &Path) -> Result<Timeline, WorkbenchError> {
    let source = fs::read_to_string(path)
        .map_err(|error| WorkbenchError::new(format!("cannot read {}: {error}", path.display())))?;
    Timeline::parse(&source).map_err(|error| WorkbenchError::new(error.to_string()))
}

fn source_revision(source: &[u8]) -> String {
    format!("{:x}", Sha256::digest(source))
}

fn validate_milestone_program_source(
    timeline: &Timeline,
    source: &str,
) -> Result<(MilestoneProgram, milestone_dsl::CompiledMilestones), WorkbenchError> {
    let program = milestone_dsl::parse(source)
        .map_err(|error| WorkbenchError::new(format!("invalid milestone program: {error}")))?;
    let authored = program
        .definitions
        .iter()
        .map(|definition| definition.name.as_str())
        .collect::<BTreeSet<_>>();
    let mut referenced = timeline
        .goals
        .values()
        .map(|goal| goal.predicate.as_str())
        .collect::<BTreeSet<_>>();
    if let Some(origin) = &timeline.origin {
        referenced.insert(origin.predicate.as_str());
    }
    if let Some(missing) = referenced.difference(&authored).next() {
        return Err(WorkbenchError::new(format!(
            "timeline references predicate {missing:?}, but the predicate program does not define it"
        )));
    }
    let compiled = milestone_dsl::compile(&program).map_err(|error| {
        WorkbenchError::new(format!("cannot compile milestone program: {error}"))
    })?;
    Ok((program, compiled))
}

fn validated_milestone_program_path(
    timeline: &Timeline,
    root: &Path,
) -> Result<Option<PathBuf>, WorkbenchError> {
    let Some(relative) = &timeline.predicate_program else {
        return Ok(None);
    };
    if relative.as_os_str().is_empty()
        || relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(WorkbenchError::new(
            "configured milestone program is not a contained relative path",
        ));
    }
    let root = fs::canonicalize(root).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot resolve milestone program root {}: {error}",
            root.display()
        ))
    })?;
    let mut candidate = root.clone();
    for component in relative.components() {
        candidate.push(component.as_os_str());
        let metadata = fs::symlink_metadata(&candidate).map_err(|error| {
            WorkbenchError::new(format!(
                "cannot inspect configured milestone program path {}: {error}",
                candidate.display()
            ))
        })?;
        if metadata.file_type().is_symlink() {
            return Err(WorkbenchError::new(format!(
                "configured milestone program path {} contains a symbolic link",
                candidate.display()
            )));
        }
    }
    let metadata = fs::symlink_metadata(&candidate).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot inspect configured milestone program {}: {error}",
            candidate.display()
        ))
    })?;
    if !metadata.is_file() {
        return Err(WorkbenchError::new(format!(
            "configured milestone program {} is not a regular file",
            candidate.display()
        )));
    }
    let resolved = fs::canonicalize(&candidate).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot resolve configured milestone program {}: {error}",
            candidate.display()
        ))
    })?;
    if !resolved.starts_with(&root) {
        return Err(WorkbenchError::new(format!(
            "configured milestone program {} escapes root {}",
            resolved.display(),
            root.display()
        )));
    }
    Ok(Some(resolved))
}

fn milestone_program_projection(
    timeline: &Timeline,
    root: &Path,
) -> Result<Option<GraphPredicateProgram>, WorkbenchError> {
    let Some(path) = validated_milestone_program_path(timeline, root)? else {
        return Ok(None);
    };
    let bytes = fs::read(&path).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot read configured milestone program {}: {error}",
            path.display()
        ))
    })?;
    let source = String::from_utf8(bytes.clone()).map_err(|_| {
        WorkbenchError::new(format!(
            "configured milestone program {} is not UTF-8",
            path.display()
        ))
    })?;
    let (program, compiled) = validate_milestone_program_source(timeline, &source)?;
    let definition_digests = compiled
        .definitions
        .into_iter()
        .map(|definition| {
            let digest = definition
                .sha256
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            (definition.name, digest)
        })
        .collect::<BTreeMap<_, _>>();
    let program_sha256 = compiled
        .program_sha256
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(Some(GraphPredicateProgram {
        schema: MILESTONE_PROGRAM_SCHEMA.into(),
        source,
        revision_sha256: source_revision(&bytes),
        program_sha256,
        definitions: program
            .definitions
            .into_iter()
            .map(|definition| GraphPredicate {
                definition_sha256: definition_digests[&definition.name].clone(),
                name: definition.name,
                phase: definition.phase,
                stable_ticks: definition.stable_ticks,
                expression: definition.when,
                then: definition.then,
                within_ticks: definition.within_ticks,
            })
            .collect(),
    }))
}

fn is_exact_boot_boundary_predicate(definition: &GraphPredicate) -> bool {
    use milestone_dsl::{Comparison, EvaluationPhase, Expression, Field, Value};

    fn collect<'a>(
        expression: &'a Expression,
        leaves: &mut Vec<(Field, Comparison, &'a Value)>,
    ) -> bool {
        match expression {
            Expression::And(left, right) => collect(left, leaves) && collect(right, leaves),
            Expression::Compare {
                field,
                operator,
                value,
            } => {
                leaves.push((*field, *operator, value));
                true
            }
            Expression::Not(_) | Expression::Or(_, _) | Expression::Query { .. } => false,
        }
    }

    if definition.phase != EvaluationPhase::PreInput
        || definition.stable_ticks != 1
        || !definition.then.is_empty()
        || definition.within_ticks.is_some()
    {
        return false;
    }
    let mut leaves = Vec::new();
    if !collect(&definition.expression, &mut leaves) || leaves.len() != 2 {
        return false;
    }
    let boot_kind = leaves.iter().any(|(field, operator, value)| {
        *field == Field::BoundaryKind
            && *operator == Comparison::Equal
            && matches!(value, Value::Symbol(symbol) if symbol == "boot")
    });
    let boundary_zero = leaves.iter().any(|(field, operator, value)| {
        *field == Field::BoundaryIndex
            && *operator == Comparison::Equal
            && matches!(value, Value::U64(0))
    });
    boot_kind && boundary_zero
}

#[derive(Debug)]
enum MilestoneProgramUpdateError {
    Stale { expected: String, actual: String },
    Invalid(WorkbenchError),
}

impl fmt::Display for MilestoneProgramUpdateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stale { expected, actual } => write!(
                formatter,
                "milestone program edit is stale: expected revision {expected}, current revision is {actual}"
            ),
            Self::Invalid(error) => error.fmt(formatter),
        }
    }
}

impl From<WorkbenchError> for MilestoneProgramUpdateError {
    fn from(error: WorkbenchError) -> Self {
        Self::Invalid(error)
    }
}

struct RemoveFileOnDrop(Option<PathBuf>);

impl Drop for RemoveFileOnDrop {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = fs::remove_file(path);
        }
    }
}

fn rollback_milestone_program(backup: &Path, target: &Path) -> Result<(), WorkbenchError> {
    if target.exists() {
        return Err(WorkbenchError::new(format!(
            "cannot restore milestone program backup {} because {} now exists",
            backup.display(),
            target.display()
        )));
    }
    fs::rename(backup, target).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot restore milestone program backup {} to {}: {error}",
            backup.display(),
            target.display()
        ))
    })
}

fn update_milestone_program(
    timeline: &Timeline,
    root: &Path,
    request: &BrowserMilestoneProgramUpdateRequest,
) -> Result<GraphPredicateProgram, MilestoneProgramUpdateError> {
    let _edit = milestone_program_edits()
        .lock()
        .map_err(|_| WorkbenchError::new("milestone program edit lock is poisoned"))?;
    let path = validated_milestone_program_path(timeline, root)?
        .ok_or_else(|| WorkbenchError::new("timeline has no configured milestone program"))?;
    let current = fs::read(&path).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot read configured milestone program {}: {error}",
            path.display()
        ))
    })?;
    let current_revision = source_revision(&current);
    if request.expected_revision_sha256 != current_revision {
        return Err(MilestoneProgramUpdateError::Stale {
            expected: request.expected_revision_sha256.clone(),
            actual: current_revision,
        });
    }
    validate_milestone_program_source(timeline, &request.source)?;

    let parent = path
        .parent()
        .ok_or_else(|| WorkbenchError::new("milestone program has no parent directory"))?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| WorkbenchError::new("milestone program filename is not UTF-8"))?;
    let nonce = random_session_token()?;
    let temporary = parent.join(format!(".{name}.{nonce}.tmp"));
    let backup = parent.join(format!(".{name}.{nonce}.rollback"));
    let mut temporary_file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| {
            WorkbenchError::new(format!(
                "cannot create adjacent milestone program temporary file {}: {error}",
                temporary.display()
            ))
        })?;
    let mut temporary_cleanup = RemoveFileOnDrop(Some(temporary.clone()));
    if let Err(error) = temporary_file
        .write_all(request.source.as_bytes())
        .and_then(|()| temporary_file.sync_all())
    {
        return Err(WorkbenchError::new(format!(
            "cannot flush milestone program temporary file {}: {error}",
            temporary.display()
        ))
        .into());
    }
    drop(temporary_file);

    let revalidated = validated_milestone_program_path(timeline, root)?
        .ok_or_else(|| WorkbenchError::new("timeline lost its configured milestone program"))?;
    if revalidated != path {
        return Err(WorkbenchError::new(
            "configured milestone program path changed while preparing the edit",
        )
        .into());
    }
    let before_replace = fs::read(&path).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot re-read configured milestone program {}: {error}",
            path.display()
        ))
    })?;
    let before_replace_revision = source_revision(&before_replace);
    if request.expected_revision_sha256 != before_replace_revision {
        return Err(MilestoneProgramUpdateError::Stale {
            expected: request.expected_revision_sha256.clone(),
            actual: before_replace_revision,
        });
    }

    if let Err(error) = fs::rename(&path, &backup) {
        return Err(WorkbenchError::new(format!(
            "cannot stage milestone program rollback backup {}: {error}",
            backup.display()
        ))
        .into());
    }
    let moved_revision = fs::symlink_metadata(&backup)
        .map_err(|error| {
            WorkbenchError::new(format!(
                "cannot inspect milestone program rollback backup {}: {error}",
                backup.display()
            ))
        })
        .and_then(|metadata| {
            if metadata.is_file() && !metadata.file_type().is_symlink() {
                Ok(())
            } else {
                Err(WorkbenchError::new(format!(
                    "milestone program changed to a non-regular file during replacement: {}",
                    backup.display()
                )))
            }
        })
        .and_then(|()| fs::read(&backup).map_err(|error| WorkbenchError::new(error.to_string())))
        .map(|bytes| source_revision(&bytes))
        .map_err(|error| {
            WorkbenchError::new(format!(
                "cannot verify milestone program rollback backup {}: {error}",
                backup.display()
            ))
        });
    match moved_revision {
        Ok(actual) if actual == request.expected_revision_sha256 => {}
        result => {
            let rollback = rollback_milestone_program(&backup, &path);
            rollback?;
            return match result {
                Ok(actual) => Err(MilestoneProgramUpdateError::Stale {
                    expected: request.expected_revision_sha256.clone(),
                    actual,
                }),
                Err(error) => Err(error.into()),
            };
        }
    }
    if let Err(error) = fs::rename(&temporary, &path) {
        let rollback = rollback_milestone_program(&backup, &path);
        rollback?;
        return Err(WorkbenchError::new(format!(
            "cannot replace milestone program {}: {error}",
            path.display()
        ))
        .into());
    }
    temporary_cleanup.0 = None;
    let _ = fs::remove_file(&backup);

    milestone_program_projection(timeline, root)?
        .ok_or_else(|| WorkbenchError::new("timeline lost its configured milestone program"))
        .map_err(Into::into)
}

/// Build the JSON-ready graph projection used by the visual selector.
/// Missing or unsupported artifacts remain visible with `playable: false`.
pub fn graph_from_timeline(
    timeline: &Timeline,
    repository_root: &Path,
) -> Result<WorkbenchGraph, WorkbenchError> {
    timeline
        .inspect()
        .map_err(|error| WorkbenchError::new(error.to_string()))?;
    let predicate_program = milestone_program_projection(timeline, repository_root)?;
    let predicate_digests = predicate_program
        .as_ref()
        .map(|program| {
            program
                .definitions
                .iter()
                .map(|definition| {
                    (
                        definition.name.as_str(),
                        definition.definition_sha256.as_str(),
                    )
                })
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    let origin = timeline.origin.as_ref().map(|origin| GraphOrigin {
        id: origin.id.clone(),
        predicate: origin.predicate.clone(),
        recordable_from_boot: predicate_program
            .as_ref()
            .and_then(|program| {
                program
                    .definitions
                    .iter()
                    .find(|definition| definition.name == origin.predicate)
            })
            .is_some_and(is_exact_boot_boundary_predicate),
    });
    let goals = timeline
        .goals
        .values()
        .map(|goal| GraphGoal {
            id: goal.id.clone(),
            segment: goal.segment.clone(),
            predicate: goal.predicate.clone(),
        })
        .collect();

    let segments = timeline
        .segments
        .values()
        .map(|segment| {
            let loaded = load_segment_tape(segment, repository_root);
            let relevant_goals = timeline
                .goals
                .values()
                .filter(|goal| {
                    goal.segment == segment.id
                        || timeline
                            .proofs
                            .iter()
                            .any(|proof| proof.segment == segment.id && proof.goal == goal.id)
                })
                .collect::<Vec<_>>();
            let goal_proofs = relevant_goals
                .iter()
                .map(|goal| {
                    let proof = timeline
                        .proofs
                        .iter()
                        .find(|proof| proof.segment == segment.id && proof.goal == goal.id);
                    let status = match (predicate_program.as_ref(), proof) {
                        (None, _) => "not_required",
                        (Some(_), None) => "missing",
                        (Some(program), Some(proof))
                            if proof.predicate_program_sha256 == program.program_sha256
                                && predicate_digests.get(goal.predicate.as_str()).is_some_and(
                                    |digest| *digest == proof.predicate_definition_sha256,
                                ) =>
                        {
                            "verified"
                        }
                        (Some(_), Some(_)) => "stale",
                    };
                    GraphGoalProof {
                        goal: goal.id.clone(),
                        predicate: goal.predicate.clone(),
                        program_sha256: proof
                            .map(|proof| proof.predicate_program_sha256.clone())
                            .unwrap_or_default(),
                        definition_sha256: proof
                            .map(|proof| proof.predicate_definition_sha256.clone())
                            .unwrap_or_default(),
                        status: status.into(),
                        first_hit_tick: proof.and_then(|proof| proof.first_hit_tick),
                    }
                })
                .collect::<Vec<_>>();
            let predicate_proof = if goal_proofs.is_empty() {
                "not_required"
            } else if goal_proofs.iter().all(|proof| proof.status == "verified") {
                "verified"
            } else if goal_proofs.iter().any(|proof| proof.status == "stale") {
                "stale"
            } else {
                "missing"
            };
            let first_hit_tick = goal_proofs
                .iter()
                .filter(|proof| proof.status == "verified")
                .filter_map(|proof| proof.first_hit_tick)
                .min();
            let record_anchors = goal_proofs
                .iter()
                .filter(|proof| proof.status == "verified")
                .map(|proof| GraphRecordAnchor {
                    goal: proof.goal.clone(),
                    predicate: proof.predicate.clone(),
                })
                .collect::<Vec<_>>();
            let playable = loaded.is_ok()
                && artifact_is_canonical_payload(&segment.artifact)
                && fingerprints_are_exact(segment)
                && materialize_segment_chain(timeline, repository_root, &segment.id).is_ok();
            let (option_visualization, option_diagnostic_error) = loaded
                .as_ref()
                .ok()
                .map(|tape| load_option_visualization(segment, repository_root, tape))
                .unwrap_or_else(|| Ok(Vec::new()))
                .map(|visualization| (visualization, None))
                .unwrap_or_else(|error| (Vec::new(), Some(error.to_string())));
            GraphSegment {
                id: segment.id.clone(),
                name: segment.name.clone(),
                parent: segment.parent.clone(),
                profile: segment.profile.as_str().into(),
                artifact: graph_artifact(&segment.artifact),
                start_fingerprint: segment.start_fingerprint.clone(),
                boundary_fingerprint: segment.end_fingerprint.clone(),
                goal_proofs,
                predicate_proof: predicate_proof.into(),
                first_hit_tick,
                frame_count: loaded.as_ref().ok().map(|tape| tape.frames.len() as u64),
                start_tick: 0,
                end_tick: loaded
                    .as_ref()
                    .ok()
                    .and_then(|tape| (tape.frames.len() as u64).checked_sub(1)),
                ticks: first_hit_tick,
                playable,
                recordable: loaded.is_ok() && !record_anchors.is_empty(),
                record_anchors,
                option_visualization,
                option_diagnostic_error,
                generated: None,
                thumbnail: None,
                error: loaded.err().map(|error| error.to_string()),
            }
        })
        .collect();
    Ok(WorkbenchGraph {
        schema: GRAPH_SCHEMA.into(),
        timeline: timeline.name.clone(),
        origin,
        segments,
        goals,
        drafts: Vec::new(),
        draft_graph_revision: None,
        predicate_program,
    })
}

fn native_fingerprint(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn graph_with_drafts(
    timeline: &Timeline,
    repository_root: &Path,
    state_root: &Path,
) -> Result<WorkbenchGraph, WorkbenchError> {
    let mut graph = graph_from_timeline(timeline, repository_root)?;
    let manifests = scan_draft_manifests(state_root)?;
    graph.draft_graph_revision = Some(draft_graph_revision(&manifests)?);
    graph.drafts = graph_drafts_from_manifests(timeline, repository_root, state_root, manifests)?;
    Ok(graph)
}

fn bounded_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > MAX_SEARCH_ARTIFACT_BYTES
    {
        return None;
    }
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

fn median_tick(ticks: &[u64]) -> Option<u64> {
    let mut ticks = ticks.to_vec();
    ticks.sort_unstable();
    ticks.get(ticks.len() / 2).copied()
}

fn generated_search_projections(
    timeline: &Timeline,
    search_root: &Path,
) -> Vec<GeneratedProjection> {
    let Ok(root_metadata) = fs::symlink_metadata(search_root) else {
        return Vec::new();
    };
    if !root_metadata.is_dir() || root_metadata.file_type().is_symlink() {
        return Vec::new();
    }
    let Ok(canonical_search_root) = fs::canonicalize(search_root) else {
        return Vec::new();
    };
    let authored_boundaries = timeline
        .segments
        .values()
        .map(|segment| segment.end_fingerprint.as_str())
        .collect::<BTreeSet<_>>();
    let mut runs = fs::read_dir(search_root)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            if !metadata.is_dir() || entry.file_type().ok()?.is_symlink() {
                return None;
            }
            let modified = metadata.modified().ok()?;
            Some((modified, entry.path()))
        })
        .collect::<Vec<_>>();
    runs.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
    runs.truncate(MAX_SEARCH_RUNS);

    let mut projections = Vec::new();
    let mut seen_boundaries = BTreeSet::new();
    for (_, run) in runs {
        let mut completed_generations = fs::read_dir(&run)
            .ok()
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let name = entry.file_name();
                let generation = name.to_str()?.strip_prefix('g')?.parse::<u32>().ok()?;
                entry
                    .path()
                    .join("results.json")
                    .is_file()
                    .then_some((generation, entry.path()))
            })
            .collect::<Vec<_>>();
        completed_generations.sort_by_key(|item| std::cmp::Reverse(item.0));
        let Some((generation, generation_root)) = completed_generations.into_iter().next() else {
            continue;
        };
        let Some(results) =
            bounded_json::<GeneratedAnchoredResults>(&generation_root.join("results.json"))
        else {
            continue;
        };
        if results.schema != "dusklight-anchored-search-results/v2"
            || results.results.segment != results.objective.segment
            || results.objective.digest.len() != 64
            || !results
                .objective
                .digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            continue;
        }
        let objective = &results.objective;
        let Some(parent) = timeline
            .segments
            .values()
            .find(|segment| segment.end_fingerprint == objective.source_boundary_fingerprint)
        else {
            continue;
        };
        let goal = timeline.goals.values().find(|goal| {
            goal.predicate == objective.goal_milestone
                && (goal.segment == parent.id
                    || timeline.segments.get(&goal.segment).is_some_and(|segment| {
                        segment.parent.as_deref() == Some(parent.id.as_str())
                    }))
        });
        let display_base = goal
            .and_then(|goal| timeline.segments.get(&goal.segment))
            .and_then(|segment| segment.name.clone())
            .unwrap_or_else(|| objective.goal_milestone.replace('_', " "));
        let mut ranked = results
            .results
            .candidates
            .iter()
            .filter(|(_, result)| {
                result.milestone_depth == 2
                    && result.attempts >= 2
                    && result.successes == result.attempts
                    && result.first_hit_ticks.len() == result.attempts as usize
                    && result
                        .first_hit_ticks
                        .windows(2)
                        .all(|pair| pair[0] == pair[1])
            })
            .map(|(id, result)| (id, result, median_tick(&result.first_hit_ticks).unwrap()))
            .collect::<Vec<_>>();
        ranked.sort_by(|left, right| left.2.cmp(&right.2).then_with(|| left.0.cmp(right.0)));
        ranked.truncate(GENERATED_SEGMENTS_PER_RUN);
        for (candidate_id, result, tick) in ranked {
            if candidate_id.len() != 64
                || !candidate_id.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                continue;
            }
            let candidate_path = generation_root.join(format!("{candidate_id}.candidate.json"));
            let suffix_path = generation_root.join(format!("{candidate_id}.tape"));
            let attempt_root = generation_root
                .join("evaluations")
                .join("candidates")
                .join(candidate_id);
            let mut attempts = Vec::new();
            for attempt in 1..=result.attempts {
                let path = attempt_root
                    .join(format!("attempt-{attempt:03}"))
                    .join("attempt.json");
                let Some(evidence) = bounded_json::<GeneratedAttempt>(&path) else {
                    attempts.clear();
                    break;
                };
                attempts.push(evidence);
            }
            let Some(first) = attempts.first() else {
                continue;
            };
            let Some(output_fingerprint) = first
                .boundary_fingerprints
                .get(&objective.goal_milestone)
                .map(|fingerprint| fingerprint.digest.clone())
            else {
                continue;
            };
            if authored_boundaries.contains(output_fingerprint.as_str())
                || !native_fingerprint(&output_fingerprint)
                || !seen_boundaries.insert((
                    objective.source_boundary_fingerprint.clone(),
                    output_fingerprint.clone(),
                ))
                || attempts.iter().any(|attempt| {
                    attempt.candidate_id != *candidate_id
                        || attempt.exit_code != Some(0)
                        || attempt.infrastructure_error.is_some()
                        || !attempt.goal_reached
                        || attempt.first_hit_tick != Some(tick)
                        || attempt
                            .boundary_fingerprints
                            .get(&objective.goal_milestone)
                            .is_none_or(|fingerprint| fingerprint.digest != output_fingerprint)
                })
            {
                continue;
            }
            let (Ok(candidate_path), Ok(suffix_path), Ok(full_tape)) = (
                fs::canonicalize(&candidate_path),
                fs::canonicalize(&suffix_path),
                fs::canonicalize(&first.tape),
            ) else {
                continue;
            };
            if !candidate_path.starts_with(&canonical_search_root)
                || !suffix_path.starts_with(&canonical_search_root)
                || !full_tape.starts_with(&canonical_search_root)
                || fs::symlink_metadata(&candidate_path)
                    .ok()
                    .is_none_or(|metadata| {
                        !metadata.is_file()
                            || metadata.file_type().is_symlink()
                            || metadata.len() > MAX_SEARCH_ARTIFACT_BYTES
                    })
                || fs::symlink_metadata(&suffix_path)
                    .ok()
                    .is_none_or(|metadata| {
                        !metadata.is_file()
                            || metadata.file_type().is_symlink()
                            || metadata.len() > MAX_SEARCH_ARTIFACT_BYTES
                    })
                || fs::symlink_metadata(&full_tape)
                    .ok()
                    .is_none_or(|metadata| {
                        !metadata.is_file()
                            || metadata.file_type().is_symlink()
                            || metadata.len() > MAX_SEARCH_ARTIFACT_BYTES
                    })
            {
                continue;
            }
            let Ok(suffix_bytes) = fs::read(&suffix_path) else {
                continue;
            };
            if suffix_bytes.len() as u64 > MAX_SEARCH_ARTIFACT_BYTES {
                continue;
            }
            let Ok(suffix) = InputTape::decode(&suffix_bytes) else {
                continue;
            };
            let short_objective = &objective.digest[..16];
            let short_candidate = &candidate_id[..16];
            let id = format!("search-{short_objective}-{short_candidate}");
            let goal_proofs = goal
                .map(|goal| {
                    vec![GraphGoalProof {
                        goal: goal.id.clone(),
                        predicate: goal.predicate.clone(),
                        program_sha256: objective.milestone_program_sha256.clone(),
                        definition_sha256: objective.goal_definition_sha256.clone(),
                        status: "verified".into(),
                        first_hit_tick: Some(tick),
                    }]
                })
                .unwrap_or_default();
            projections.push(GeneratedProjection {
                segment: GraphSegment {
                    id,
                    name: Some(format!("{display_base} · {tick}f · {}", &candidate_id[..6])),
                    parent: Some(parent.id.clone()),
                    profile: objective.segment.as_str().into(),
                    artifact: GraphArtifact {
                        kind: "tape".into(),
                        value: suffix_path.display().to_string(),
                    },
                    start_fingerprint: objective.source_boundary_fingerprint.clone(),
                    boundary_fingerprint: output_fingerprint,
                    goal_proofs,
                    predicate_proof: "verified".into(),
                    first_hit_tick: Some(tick),
                    frame_count: Some(suffix.tape.frames.len() as u64),
                    start_tick: 0,
                    end_tick: (suffix.tape.frames.len() as u64).checked_sub(1),
                    ticks: Some(tick),
                    playable: true,
                    recordable: false,
                    record_anchors: Vec::new(),
                    option_visualization: Vec::new(),
                    option_diagnostic_error: None,
                    generated: Some(GraphGeneratedSegment {
                        kind: "search_candidate".into(),
                        status: "proved".into(),
                        uncommitted: true,
                        run: run.display().to_string(),
                        generation,
                        candidate_id: candidate_id.clone(),
                        candidate: candidate_path.display().to_string(),
                        tape: suffix_path.display().to_string(),
                        objective_sha256: objective.digest.clone(),
                        source_predicate: objective.source_milestone.clone(),
                        goal_predicate: objective.goal_milestone.clone(),
                        proof_attempts: result.attempts,
                    }),
                    thumbnail: None,
                    error: None,
                },
                full_tape,
            });
            if projections.len() >= MAX_GENERATED_SEGMENTS {
                return projections;
            }
        }
    }
    projections
}

fn generated_search_tombstone_path(state_root: &Path) -> PathBuf {
    state_root.join(GENERATED_SEARCH_TOMBSTONES)
}

fn load_generated_search_tombstones(
    state_root: &Path,
) -> Result<GeneratedSearchTombstones, WorkbenchError> {
    let path = generated_search_tombstone_path(state_root);
    let Ok(metadata) = fs::symlink_metadata(&path) else {
        return Ok(GeneratedSearchTombstones {
            schema: GENERATED_SEARCH_TOMBSTONE_SCHEMA.into(),
            candidate_ids: BTreeSet::new(),
        });
    };
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() > MAX_SEARCH_ARTIFACT_BYTES
    {
        return Err(WorkbenchError::new(
            "generated search tombstones are not a bounded physical file",
        ));
    }
    let tombstones: GeneratedSearchTombstones =
        serde_json::from_slice(&fs::read(&path).map_err(|error| {
            WorkbenchError::new(format!("cannot read generated search tombstones: {error}"))
        })?)
        .map_err(|error| WorkbenchError::new(format!("invalid search tombstones: {error}")))?;
    if tombstones.schema != GENERATED_SEARCH_TOMBSTONE_SCHEMA
        || tombstones.candidate_ids.iter().any(|id| {
            id.len() != 64
                || !id
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        })
    {
        return Err(WorkbenchError::new(
            "generated search tombstones have an invalid schema or candidate ID",
        ));
    }
    Ok(tombstones)
}

fn visible_generated_search_projections(
    timeline: &Timeline,
    search_root: &Path,
    state_root: &Path,
) -> Result<Vec<GeneratedProjection>, WorkbenchError> {
    let hidden = load_generated_search_tombstones(state_root)?.candidate_ids;
    Ok(generated_search_projections(timeline, search_root)
        .into_iter()
        .filter(|projection| {
            projection
                .segment
                .generated
                .as_ref()
                .is_none_or(|generated| !hidden.contains(&generated.candidate_id))
        })
        .collect())
}

fn append_generated_search_segments(
    graph: &mut WorkbenchGraph,
    timeline: &Timeline,
    search_root: &Path,
    state_root: &Path,
) -> Result<(), WorkbenchError> {
    graph.segments.extend(
        visible_generated_search_projections(timeline, search_root, state_root)?
            .into_iter()
            .map(|projection| projection.segment),
    );
    Ok(())
}

fn thumbnail_key(kind: &str, materialization: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(b"dusklight.route-workbench.thumbnail.v3-4x3\0");
    for value in [kind, materialization] {
        digest.update((value.len() as u64).to_le_bytes());
        digest.update(value.as_bytes());
    }
    format!("{:x}", digest.finalize())
}

fn graph_node_thumbnail_key(
    graph: &WorkbenchGraph,
    selection: &BrowserSelection,
) -> Result<String, WorkbenchError> {
    match selection {
        BrowserSelection::Segment { id } => {
            let segment = graph
                .segments
                .iter()
                .find(|segment| segment.id == *id)
                .ok_or_else(|| WorkbenchError::new(format!("unknown segment {id:?}")))?;
            if !segment.playable {
                return Err(WorkbenchError::new(format!(
                    "segment {id:?} is not playable"
                )));
            }
            Ok(thumbnail_key("segment", &segment.boundary_fingerprint))
        }
        BrowserSelection::Draft { id } => {
            let draft = graph
                .drafts
                .iter()
                .find(|draft| draft.id == *id)
                .ok_or_else(|| WorkbenchError::new(format!("unknown draft {id:?}")))?;
            if !draft.playable {
                return Err(WorkbenchError::new(format!("draft {id:?} is not playable")));
            }
            let identity = draft.result_tape_sha256.as_deref().ok_or_else(|| {
                WorkbenchError::new(format!("draft {id:?} has no finalized chain fingerprint"))
            })?;
            Ok(thumbnail_key("draft", identity))
        }
    }
}

fn thumbnail_url(key: &str) -> String {
    format!("/api/thumbnails/{key}.png")
}

fn thumbnail_cache_path(state_root: &Path, key: &str) -> PathBuf {
    state_root
        .join(THUMBNAIL_DIRECTORY)
        .join(format!("{key}.png"))
}

fn thumbnail_file_is_valid(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() || metadata.len() < 8 || metadata.len() > MAX_THUMBNAIL_BYTES {
        return false;
    }
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    let mut signature = [0_u8; 8];
    file.read_exact(&mut signature).is_ok() && signature == *b"\x89PNG\r\n\x1a\n"
}

fn reachable_thumbnail_keys(graph: &WorkbenchGraph) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    for segment in &graph.segments {
        keys.insert(thumbnail_key("segment", &segment.boundary_fingerprint));
    }
    for draft in &graph.drafts {
        if let Some(identity) = draft.result_tape_sha256.as_deref() {
            keys.insert(thumbnail_key("draft", identity));
        }
    }
    keys
}

#[derive(Clone, Debug, Serialize)]
pub struct ThumbnailPruneEntry {
    pub key: String,
    pub source: PathBuf,
    pub size: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct ThumbnailPruneReport {
    pub schema: &'static str,
    pub dry_run: bool,
    pub reachable: usize,
    pub orphaned: Vec<ThumbnailPruneEntry>,
    pub moved: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trash_transaction: Option<PathBuf>,
}

fn prune_orphaned_thumbnails(
    graph: &WorkbenchGraph,
    state_root: &Path,
    apply: bool,
) -> Result<ThumbnailPruneReport, WorkbenchError> {
    let thumbnail_root = state_root.join(THUMBNAIL_DIRECTORY);
    let entries = match fs::read_dir(&thumbnail_root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ThumbnailPruneReport {
                schema: THUMBNAIL_PRUNE_SCHEMA,
                dry_run: !apply,
                reachable: 0,
                orphaned: Vec::new(),
                moved: 0,
                trash_transaction: None,
            });
        }
        Err(error) => {
            return Err(WorkbenchError::new(format!(
                "cannot inspect thumbnail cache {}: {error}",
                thumbnail_root.display()
            )));
        }
    };
    let reachable = reachable_thumbnail_keys(graph);
    let mut orphaned = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            WorkbenchError::new(format!(
                "cannot inspect thumbnail cache {}: {error}",
                thumbnail_root.display()
            ))
        })?;
        let file_type = entry.file_type().map_err(|error| {
            WorkbenchError::new(format!(
                "cannot inspect thumbnail cache entry {}: {error}",
                entry.path().display()
            ))
        })?;
        if !file_type.is_file() {
            continue;
        }
        let Some(filename) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Some(key) = filename.strip_suffix(".png") else {
            continue;
        };
        if !valid_sha256(key) || reachable.contains(key) {
            continue;
        }
        orphaned.push(ThumbnailPruneEntry {
            key: key.into(),
            source: entry.path(),
            size: entry
                .metadata()
                .map_err(|error| {
                    WorkbenchError::new(format!(
                        "cannot inspect orphaned thumbnail {}: {error}",
                        entry.path().display()
                    ))
                })?
                .len(),
        });
    }
    orphaned.sort_by(|left, right| left.key.cmp(&right.key));
    let trash_transaction = if apply && !orphaned.is_empty() {
        let state_root = fs::canonicalize(state_root).map_err(|error| {
            WorkbenchError::new(format!("cannot resolve workbench state root: {error}"))
        })?;
        let trash_root = state_root.join(DRAFT_TRASH_DIRECTORY).join("thumbnails");
        fs::create_dir_all(&trash_root).map_err(|error| {
            WorkbenchError::new(format!("cannot create thumbnail trash: {error}"))
        })?;
        let trash_root = fs::canonicalize(&trash_root).map_err(|error| {
            WorkbenchError::new(format!("cannot resolve thumbnail trash: {error}"))
        })?;
        if !trash_root.starts_with(&state_root) || trash_root == state_root {
            return Err(WorkbenchError::new(
                "thumbnail trash escapes the workbench state root",
            ));
        }
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| WorkbenchError::new(error.to_string()))?
            .as_nanos();
        let transaction = trash_root.join(format!("prune-{}-{nonce}", std::process::id()));
        fs::create_dir(&transaction).map_err(|error| {
            WorkbenchError::new(format!(
                "cannot create thumbnail trash transaction: {error}"
            ))
        })?;
        let mut moved: Vec<&ThumbnailPruneEntry> = Vec::new();
        for entry in &orphaned {
            let destination = transaction.join(format!("{}.png", entry.key));
            if let Err(error) = fs::rename(&entry.source, &destination) {
                for prior in moved.iter().rev() {
                    let _ = fs::rename(
                        transaction.join(format!("{}.png", prior.key)),
                        &prior.source,
                    );
                }
                let _ = fs::remove_dir(&transaction);
                return Err(WorkbenchError::new(format!(
                    "cannot move orphaned thumbnail {} to recoverable trash: {error}",
                    entry.source.display()
                )));
            }
            moved.push(entry);
        }
        Some(transaction)
    } else {
        None
    };
    Ok(ThumbnailPruneReport {
        schema: THUMBNAIL_PRUNE_SCHEMA,
        dry_run: !apply,
        reachable: reachable.len(),
        moved: trash_transaction.as_ref().map_or(0, |_| orphaned.len()),
        orphaned,
        trash_transaction,
    })
}

/// Preview or apply cache pruning against the same graph projection used by
/// Route Workbench. Applying moves files into recoverable state-root trash.
pub fn prune_thumbnails(
    timeline: &Timeline,
    timeline_path: &Path,
    repository_root: &Path,
    state_root: &Path,
    apply: bool,
) -> Result<ThumbnailPruneReport, WorkbenchError> {
    let repository_root = fs::canonicalize(repository_root)
        .map_err(|error| WorkbenchError::new(format!("cannot resolve repository root: {error}")))?;
    let timeline_path = fs::canonicalize(timeline_path)
        .map_err(|error| WorkbenchError::new(format!("cannot resolve timeline: {error}")))?;
    if !timeline_path.starts_with(&repository_root) {
        return Err(WorkbenchError::new(
            "timeline is outside the repository root",
        ));
    }
    let artifact_root = timeline_path
        .parent()
        .ok_or_else(|| WorkbenchError::new("timeline has no artifact root"))?;
    let mut graph = graph_with_drafts(timeline, artifact_root, state_root)?;
    append_generated_search_segments(
        &mut graph,
        timeline,
        &repository_root.join("build/search"),
        state_root,
    )?;
    prune_orphaned_thumbnails(&graph, state_root, apply)
}

fn decorate_graph_thumbnails(
    graph: &mut WorkbenchGraph,
    config: &WorkbenchConfig,
) -> Result<(), WorkbenchError> {
    for segment in &mut graph.segments {
        let key = thumbnail_key("segment", &segment.boundary_fingerprint);
        let path = thumbnail_cache_path(&config.state_root, &key);
        if thumbnail_file_is_valid(&path) {
            content_address_thumbnail(config, &path)?;
            segment.thumbnail = Some(thumbnail_url(&key));
        }
    }
    for draft in &mut graph.drafts {
        let Some(identity) = draft.result_tape_sha256.as_deref() else {
            continue;
        };
        let key = thumbnail_key("draft", identity);
        let path = thumbnail_cache_path(&config.state_root, &key);
        if thumbnail_file_is_valid(&path) {
            content_address_thumbnail(config, &path)?;
            draft.thumbnail = Some(thumbnail_url(&key));
        }
    }
    Ok(())
}

fn content_address_thumbnail(config: &WorkbenchConfig, path: &Path) -> Result<(), WorkbenchError> {
    ContentStore::initialize(config.state_root.join("content"))
        .and_then(|store| store.put_file(path, ContentKind::Screenshot))
        .map(|_| ())
        .map_err(|error| WorkbenchError::new(format!("cannot content-address thumbnail: {error}")))
}

fn prepare_missing_playback_thumbnail(
    timeline: &Timeline,
    config: &WorkbenchConfig,
    selection: &BrowserSelection,
) -> Result<Option<PlaybackThumbnailCapture>, WorkbenchError> {
    let artifact_root = configured_artifact_root(config)?;
    let graph = graph_with_drafts(timeline, &artifact_root, &config.state_root)?;
    let key = graph_node_thumbnail_key(&graph, selection)?;
    let path = thumbnail_cache_path(&config.state_root, &key);
    if thumbnail_file_is_valid(&path) {
        return Ok(None);
    }
    fs::create_dir_all(config.state_root.join(THUMBNAIL_DIRECTORY)).map_err(|error| {
        WorkbenchError::new(format!("cannot create playback thumbnail cache: {error}"))
    })?;
    if path.exists() {
        fs::remove_file(&path).map_err(|error| {
            WorkbenchError::new(format!(
                "cannot remove invalid playback thumbnail {}: {error}",
                path.display()
            ))
        })?;
    }
    Ok(Some(PlaybackThumbnailCapture {
        path,
        url: thumbnail_url(&key),
    }))
}

fn install_recording_thumbnail(
    directory: &Path,
    manifest: &DraftManifest,
    config: &WorkbenchConfig,
) -> Result<(), WorkbenchError> {
    let source = directory.join(DRAFT_TERMINAL_THUMBNAIL);
    if manifest.status != DraftStatus::Ready {
        let _ = fs::remove_file(source);
        return Ok(());
    }
    if !source.exists() {
        return Ok(());
    }
    if !thumbnail_file_is_valid(&source) {
        let _ = fs::remove_file(source);
        return Err(WorkbenchError::new(
            "native recording terminal thumbnail is invalid",
        ));
    }
    let identity = manifest.result_tape_sha256.as_deref().ok_or_else(|| {
        WorkbenchError::new("ready recording lacks a finalized chain fingerprint")
    })?;
    let key = thumbnail_key("draft", identity);
    let thumbnail_root = config.state_root.join(THUMBNAIL_DIRECTORY);
    fs::create_dir_all(&thumbnail_root).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot create recording thumbnail cache {}: {error}",
            thumbnail_root.display()
        ))
    })?;
    let destination = thumbnail_cache_path(&config.state_root, &key);
    if thumbnail_file_is_valid(&destination) {
        content_address_thumbnail(config, &destination)?;
        fs::remove_file(&source).map_err(|error| {
            WorkbenchError::new(format!(
                "cannot remove duplicate recording thumbnail {}: {error}",
                source.display()
            ))
        })?;
        return Ok(());
    }
    if destination.exists() {
        fs::remove_file(&destination).map_err(|error| {
            WorkbenchError::new(format!(
                "cannot replace invalid recording thumbnail {}: {error}",
                destination.display()
            ))
        })?;
    }
    fs::rename(&source, &destination).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot install recording thumbnail {}: {error}",
            destination.display()
        ))
    })?;
    content_address_thumbnail(config, &destination)
}

fn drafts_root(state_root: &Path) -> Result<PathBuf, WorkbenchError> {
    let root = state_root.join("drafts");
    fs::create_dir_all(&root).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot create draft root {}: {error}",
            root.display()
        ))
    })?;
    fs::canonicalize(&root).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot resolve draft root {}: {error}",
            root.display()
        ))
    })
}

fn validated_drafts_root(state_root: &Path) -> Result<PathBuf, WorkbenchError> {
    fs::create_dir_all(state_root).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot create state root {}: {error}",
            state_root.display()
        ))
    })?;
    let state_root = fs::canonicalize(state_root).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot resolve state root {}: {error}",
            state_root.display()
        ))
    })?;
    let expected = state_root.join("drafts");
    fs::create_dir_all(&expected).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot create draft root {}: {error}",
            expected.display()
        ))
    })?;
    let metadata = fs::symlink_metadata(&expected).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot inspect draft root {}: {error}",
            expected.display()
        ))
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(WorkbenchError::new(
            "draft root is not a contained physical directory",
        ));
    }
    let resolved = fs::canonicalize(&expected).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot resolve draft root {}: {error}",
            expected.display()
        ))
    })?;
    if resolved != expected || resolved.parent() != Some(state_root.as_path()) {
        return Err(WorkbenchError::new(
            "draft root escapes the route workbench state root",
        ));
    }
    Ok(resolved)
}

fn scan_draft_manifests(
    state_root: &Path,
) -> Result<BTreeMap<String, DraftManifest>, WorkbenchError> {
    let active = active_recordings()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    scan_draft_manifests_with_active(state_root, &active)
}

fn scan_draft_manifests_with_active(
    state_root: &Path,
    active: &BTreeSet<String>,
) -> Result<BTreeMap<String, DraftManifest>, WorkbenchError> {
    let root = validated_drafts_root(state_root)?;
    let mut manifests = BTreeMap::new();
    let mut entries = fs::read_dir(&root)
        .map_err(|error| WorkbenchError::new(format!("cannot scan {}: {error}", root.display())))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| WorkbenchError::new(format!("cannot scan {}: {error}", root.display())))?;
    entries.sort_by_key(|entry| entry.file_name());
    if entries.len() > MAX_DRAFTS {
        return Err(WorkbenchError::new(format!(
            "draft store has {} entries; maximum is {MAX_DRAFTS}",
            entries.len()
        )));
    }
    for entry in entries {
        let file_type = entry
            .file_type()
            .map_err(|error| WorkbenchError::new(error.to_string()))?;
        if !file_type.is_dir() {
            continue;
        }
        let directory = match fs::canonicalize(entry.path()) {
            Ok(directory) if directory.starts_with(&root) && directory != root => directory,
            _ => continue,
        };
        let id = entry.file_name().to_string_lossy().into_owned();
        if !valid_draft_id(&id) {
            continue;
        }
        let final_path = directory.join(DRAFT_FINAL_MANIFEST);
        let path = if final_path.is_file() {
            final_path
        } else {
            directory.join(DRAFT_MANIFEST)
        };
        let path = match fs::canonicalize(&path) {
            Ok(path) if path.starts_with(&directory) => path,
            _ => continue,
        };
        if !path.is_file() {
            continue;
        }
        let bytes = fs::read(&path).map_err(|error| {
            WorkbenchError::new(format!("cannot read {}: {error}", path.display()))
        })?;
        if bytes.len() > 64 * 1024 {
            continue;
        }
        let mut manifest: DraftManifest = match serde_json::from_slice(&bytes) {
            Ok(manifest) => manifest,
            Err(_) => continue,
        };
        if manifest.schema != DRAFT_SCHEMA
            || manifest.id != id
            || manifest.tape != DRAFT_TAPE
            || manifest.endpoint_kind != "manual_stop"
            || manifest.verification != "unverified"
        {
            continue;
        }
        if matches!(
            manifest.status,
            DraftStatus::Preparing | DraftStatus::Recording
        ) {
            if active.contains(&id) {
                manifest.status = DraftStatus::Recording;
                manifests.insert(id, manifest);
                continue;
            }
            let status_exists = directory
                .join(format!("{DRAFT_TAPE}.status.json"))
                .is_file();
            let launch = read_draft_launch(&directory, &manifest);
            let launch_is_live = launch
                .as_ref()
                .is_some_and(|launch| process_is_alive(launch.pid));
            match (status_exists, launch.as_ref(), launch_is_live) {
                (true, _, true) => manifest.status = DraftStatus::Recording,
                (true, _, false) => {
                    finalize_recording(&directory, &mut manifest, None);
                    let _ = write_draft_manifest(&directory, &manifest, true);
                }
                (false, Some(_), true) => manifest.status = DraftStatus::Recording,
                (false, Some(_), false) => {
                    manifest.status = DraftStatus::ProcessFailure;
                    manifest.error = Some("recording process exited without final status".into());
                    let _ = write_draft_manifest(&directory, &manifest, true);
                }
                (false, None, _) => manifest.status = DraftStatus::Orphaned,
            }
        }
        manifests.insert(id, manifest);
    }
    Ok(manifests)
}

fn draft_descendants(
    manifests: &BTreeMap<String, DraftManifest>,
    id: &str,
) -> Result<BTreeSet<String>, WorkbenchError> {
    if !valid_draft_id(id) || !manifests.contains_key(id) {
        return Err(WorkbenchError::new(format!("unknown draft {id:?}")));
    }
    Ok(draft_descendants_from_roots(manifests, [id]))
}

fn draft_descendants_from_roots<'a>(
    manifests: &BTreeMap<String, DraftManifest>,
    roots: impl IntoIterator<Item = &'a str>,
) -> BTreeSet<String> {
    let mut children = BTreeMap::<&str, Vec<&str>>::new();
    for manifest in manifests.values() {
        if let DraftParent::Draft { id: parent, .. } = &manifest.parent {
            children
                .entry(parent.as_str())
                .or_default()
                .push(manifest.id.as_str());
        }
    }
    let mut deletion = BTreeSet::new();
    let mut pending = roots.into_iter().collect::<Vec<_>>();
    while let Some(next) = pending.pop() {
        if !deletion.insert(next.to_owned()) {
            continue;
        }
        if let Some(descendants) = children.get(next) {
            pending.extend(descendants.iter().copied());
        }
    }
    deletion
}

fn draft_graph_revision(
    manifests: &BTreeMap<String, DraftManifest>,
) -> Result<String, WorkbenchError> {
    let mut digest = Sha256::new();
    digest.update(b"dusklight.route-workbench.draft-graph.v2\0");
    for (id, manifest) in manifests {
        let encoded = serde_json::to_vec(manifest).map_err(|error| {
            WorkbenchError::new(format!("cannot encode draft graph revision: {error}"))
        })?;
        digest.update((id.len() as u64).to_le_bytes());
        digest.update(id.as_bytes());
        digest.update((encoded.len() as u64).to_le_bytes());
        digest.update(encoded);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn draft_delete_confirmation_token(graph_revision: &str, deletion: &BTreeSet<String>) -> String {
    let mut digest = Sha256::new();
    digest.update(b"dusklight.route-workbench.draft-delete.v1\0");
    digest.update(graph_revision.as_bytes());
    for id in deletion {
        digest.update((id.len() as u64).to_le_bytes());
        digest.update(id.as_bytes());
    }
    format!("{:x}", digest.finalize())
}

fn draft_is_active(directory: &Path, manifest: &DraftManifest, active: &BTreeSet<String>) -> bool {
    active.contains(&manifest.id)
        || matches!(
            manifest.status,
            DraftStatus::Preparing | DraftStatus::Recording
        )
        || read_draft_launch(directory, manifest).is_some_and(|launch| process_is_alive(launch.pid))
}

fn draft_delete_preview_locked(
    state_root: &Path,
    id: &str,
    manifests: &BTreeMap<String, DraftManifest>,
    active: &BTreeSet<String>,
) -> Result<DraftDeletePreview, WorkbenchError> {
    let deletion = draft_descendants(manifests, id)?;
    let root = validated_drafts_root(state_root)?;
    for draft_id in &deletion {
        let manifest = &manifests[draft_id];
        if draft_is_active(&root.join(draft_id), manifest, active) {
            return Err(WorkbenchError::new(format!(
                "cannot delete draft {id:?}: recording {draft_id:?} is active"
            )));
        }
    }
    let graph_revision = draft_graph_revision(manifests)?;
    let confirmation_token = draft_delete_confirmation_token(&graph_revision, &deletion);
    let drafts = deletion
        .iter()
        .map(|draft_id| {
            let manifest = &manifests[draft_id];
            DraftDeleteImpact {
                id: draft_id.clone(),
                label: manifest.label.clone(),
                status: manifest.status,
            }
        })
        .collect();
    Ok(DraftDeletePreview {
        schema: DRAFT_DELETE_PREVIEW_SCHEMA.into(),
        id: id.into(),
        graph_revision,
        drafts,
        confirmation_token,
    })
}

fn preview_draft_deletion(
    state_root: &Path,
    id: &str,
) -> Result<DraftDeletePreview, WorkbenchError> {
    let active = active_recordings()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let manifests = scan_draft_manifests_with_active(state_root, &active)?;
    draft_delete_preview_locked(state_root, id, &manifests, &active)
}

fn validated_draft_directory(root: &Path, id: &str) -> Result<PathBuf, WorkbenchError> {
    if !valid_draft_id(id) {
        return Err(WorkbenchError::new(format!("invalid draft id {id:?}")));
    }
    let expected = root.join(id);
    let metadata = fs::symlink_metadata(&expected)
        .map_err(|error| WorkbenchError::new(format!("cannot inspect draft {id:?}: {error}")))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(WorkbenchError::new(format!(
            "draft {id:?} is not a contained physical directory"
        )));
    }
    let resolved = fs::canonicalize(&expected)
        .map_err(|error| WorkbenchError::new(format!("cannot resolve draft {id:?}: {error}")))?;
    if resolved != expected || resolved.parent() != Some(root) {
        return Err(WorkbenchError::new(format!(
            "draft {id:?} directory escapes the draft store"
        )));
    }
    Ok(resolved)
}

#[derive(Debug)]
enum DraftRenameError {
    Conflict(String),
    Invalid(WorkbenchError),
}

impl fmt::Display for DraftRenameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Conflict(message) => formatter.write_str(message),
            Self::Invalid(error) => error.fmt(formatter),
        }
    }
}

impl From<WorkbenchError> for DraftRenameError {
    fn from(error: WorkbenchError) -> Self {
        Self::Invalid(error)
    }
}

fn validated_draft_manifest_path(directory: &Path) -> Result<PathBuf, WorkbenchError> {
    let final_path = directory.join(DRAFT_FINAL_MANIFEST);
    let path = match fs::symlink_metadata(&final_path) {
        Ok(_) => final_path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            directory.join(DRAFT_MANIFEST)
        }
        Err(error) => {
            return Err(WorkbenchError::new(format!(
                "cannot inspect draft manifest {}: {error}",
                final_path.display()
            )));
        }
    };
    let metadata = fs::symlink_metadata(&path).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot inspect draft manifest {}: {error}",
            path.display()
        ))
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(WorkbenchError::new(format!(
            "draft manifest {} is not a contained regular file",
            path.display()
        )));
    }
    let resolved = fs::canonicalize(&path).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot resolve draft manifest {}: {error}",
            path.display()
        ))
    })?;
    if resolved != path || resolved.parent() != Some(directory) {
        return Err(WorkbenchError::new(format!(
            "draft manifest {} escapes its draft directory",
            path.display()
        )));
    }
    Ok(resolved)
}

fn rollback_draft_manifest(backup: &Path, target: &Path) -> Result<(), WorkbenchError> {
    if fs::symlink_metadata(target).is_ok() {
        return Err(WorkbenchError::new(format!(
            "cannot restore draft manifest backup {} because {} now exists",
            backup.display(),
            target.display()
        )));
    }
    fs::rename(backup, target).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot restore draft manifest backup {} to {}: {error}",
            backup.display(),
            target.display()
        ))
    })
}

fn replace_draft_manifest(
    path: &Path,
    expected: &[u8],
    replacement: &[u8],
) -> Result<(), WorkbenchError> {
    let directory = path
        .parent()
        .ok_or_else(|| WorkbenchError::new("draft manifest has no parent directory"))?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| WorkbenchError::new("draft manifest filename is not UTF-8"))?;
    let nonce = random_session_token()?;
    let temporary = directory.join(format!(".{name}.{nonce}.tmp"));
    let backup = directory.join(format!(".{name}.{nonce}.rollback"));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| {
            WorkbenchError::new(format!(
                "cannot create adjacent draft manifest temporary file {}: {error}",
                temporary.display()
            ))
        })?;
    let mut cleanup = RemoveFileOnDrop(Some(temporary.clone()));
    file.write_all(replacement)
        .and_then(|()| file.sync_all())
        .map_err(|error| {
            WorkbenchError::new(format!(
                "cannot flush draft manifest temporary file {}: {error}",
                temporary.display()
            ))
        })?;
    drop(file);

    let metadata = fs::symlink_metadata(path).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot revalidate draft manifest {}: {error}",
            path.display()
        ))
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(WorkbenchError::new(
            "draft manifest changed to a non-regular file during rename",
        ));
    }
    if fs::canonicalize(path).ok().as_deref() != Some(path)
        || !fs::read(path).is_ok_and(|bytes| bytes == expected)
    {
        return Err(WorkbenchError::new(
            "draft manifest changed while preparing rename; reload the graph",
        ));
    }

    fs::rename(path, &backup).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot stage draft manifest rollback backup {}: {error}",
            backup.display()
        ))
    })?;
    let moved_matches = fs::symlink_metadata(&backup)
        .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
        && fs::read(&backup).is_ok_and(|bytes| bytes == expected);
    if !moved_matches {
        rollback_draft_manifest(&backup, path)?;
        return Err(WorkbenchError::new(
            "draft manifest changed while staging its rollback backup",
        ));
    }
    if let Err(error) = fs::rename(&temporary, path) {
        rollback_draft_manifest(&backup, path)?;
        return Err(WorkbenchError::new(format!(
            "cannot replace draft manifest {}: {error}",
            path.display()
        )));
    }
    cleanup.0 = None;
    let _ = fs::remove_file(backup);
    Ok(())
}

fn rename_draft_label(
    state_root: &Path,
    request: &BrowserDraftRenameRequest,
) -> Result<DraftRenameResult, DraftRenameError> {
    let label = validate_draft_label(&request.label)?;
    let active = active_recordings()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let manifests = scan_draft_manifests_with_active(state_root, &active)?;
    let revision = draft_graph_revision(&manifests)?;
    if revision != request.expected_graph_revision {
        return Err(DraftRenameError::Conflict(
            "draft graph changed; reload before renaming".into(),
        ));
    }
    let manifest = manifests
        .get(&request.id)
        .ok_or_else(|| WorkbenchError::new(format!("unknown draft {:?}", request.id)))?;
    let root = validated_drafts_root(state_root)?;
    let directory = validated_draft_directory(&root, &request.id)?;
    if draft_is_active(&directory, manifest, &active) {
        return Err(DraftRenameError::Conflict(format!(
            "cannot rename draft {:?} while its recording is active",
            request.id
        )));
    }
    let path = validated_draft_manifest_path(&directory)?;
    let original = fs::read(&path).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot read draft manifest {}: {error}",
            path.display()
        ))
    })?;
    let mut disk_manifest: DraftManifest = serde_json::from_slice(&original).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot decode draft manifest {}: {error}",
            path.display()
        ))
    })?;
    if disk_manifest.schema != DRAFT_SCHEMA || disk_manifest.id != request.id {
        return Err(
            WorkbenchError::new("draft manifest identity changed while preparing rename").into(),
        );
    }
    disk_manifest.label = label.clone();
    let replacement = serde_json::to_vec(&disk_manifest)
        .map_err(|error| WorkbenchError::new(format!("cannot encode draft manifest: {error}")))?;

    let latest = scan_draft_manifests_with_active(state_root, &active)?;
    if draft_graph_revision(&latest)? != request.expected_graph_revision {
        return Err(DraftRenameError::Conflict(
            "draft graph changed while preparing rename; reload the graph".into(),
        ));
    }
    replace_draft_manifest(&path, &original, &replacement)?;
    let updated = scan_draft_manifests_with_active(state_root, &active)?;
    Ok(DraftRenameResult {
        schema: DRAFT_RENAME_RESULT_SCHEMA.into(),
        id: request.id.clone(),
        label,
        graph_revision: draft_graph_revision(&updated)?,
    })
}

#[derive(Debug)]
enum SegmentRenameError {
    Conflict(String),
    Invalid(WorkbenchError),
}

impl fmt::Display for SegmentRenameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Conflict(message) => formatter.write_str(message),
            Self::Invalid(error) => error.fmt(formatter),
        }
    }
}

impl From<WorkbenchError> for SegmentRenameError {
    fn from(error: WorkbenchError) -> Self {
        Self::Invalid(error)
    }
}

fn validate_segment_name(name: &str) -> Result<String, WorkbenchError> {
    let name = name.trim();
    if name.is_empty()
        || name.len() > 160
        || name.chars().any(char::is_control)
        || name.contains(['"', '\\'])
    {
        return Err(WorkbenchError::new(
            "segment name must be 1 to 160 UTF-8 bytes without controls, quotes, or backslashes",
        ));
    }
    Ok(name.to_owned())
}

fn timeline_line_ending(line: &str) -> &str {
    if line.ends_with("\r\n") {
        "\r\n"
    } else if line.ends_with('\n') {
        "\n"
    } else {
        ""
    }
}

fn rename_segment_in_timeline_source(
    source: &str,
    id: &str,
    name: &str,
) -> Result<String, WorkbenchError> {
    let lines = source.split_inclusive('\n').collect::<Vec<_>>();
    let mut segment_index = None;
    let mut label_index = None;
    for (index, line) in lines.iter().enumerate() {
        let raw = line.trim_end_matches(['\r', '\n']);
        let tokens =
            tokenize(raw, index + 1).map_err(|error| WorkbenchError::new(error.to_string()))?;
        if tokens.first().map(String::as_str) == Some("segment")
            && tokens.get(1).map(String::as_str) == Some(id)
        {
            segment_index = Some(index);
        }
        if tokens.first().map(String::as_str) == Some("label")
            && tokens.get(1).map(String::as_str) == Some(id)
        {
            label_index = Some(index);
        }
    }
    let segment_index =
        segment_index.ok_or_else(|| WorkbenchError::new(format!("unknown segment {id:?}")))?;
    let preferred_ending = if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let authored = format!("label {id} \"{name}\"");
    let mut output = String::with_capacity(source.len() + authored.len() + 4);
    for (index, line) in lines.iter().enumerate() {
        if label_index == Some(index) {
            output.push_str(&authored);
            output.push_str(timeline_line_ending(line));
            continue;
        }
        output.push_str(line);
        if label_index.is_none() && index == segment_index {
            if timeline_line_ending(line).is_empty() {
                output.push_str(preferred_ending);
            }
            output.push_str(&authored);
            output.push_str(preferred_ending);
        }
    }
    Ok(output)
}

#[derive(Debug)]
struct SegmentSourceDeletion {
    segments: BTreeSet<String>,
    goals: BTreeSet<String>,
    proofs: usize,
    lineages: BTreeSet<String>,
    replacement: String,
}

fn segment_descendants_from_roots<'a>(
    timeline: &Timeline,
    roots: impl IntoIterator<Item = &'a str>,
) -> Result<BTreeSet<String>, WorkbenchError> {
    let mut children = BTreeMap::<&str, Vec<&str>>::new();
    for segment in timeline.segments.values() {
        if let Some(parent) = segment.parent.as_deref() {
            children.entry(parent).or_default().push(&segment.id);
        }
    }
    let mut deletion = BTreeSet::new();
    let mut pending = roots
        .into_iter()
        .map(|root| {
            if timeline.segments.contains_key(root) {
                Ok(root)
            } else {
                Err(WorkbenchError::new(format!("unknown segment {root:?}")))
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    if pending.is_empty() {
        return Err(WorkbenchError::new("segment deletion has no roots"));
    }
    while let Some(next) = pending.pop() {
        if !deletion.insert(next.to_owned()) {
            continue;
        }
        if let Some(descendants) = children.get(next) {
            pending.extend(descendants.iter().copied());
        }
    }
    Ok(deletion)
}

#[cfg(test)]
fn delete_segment_subtree_in_timeline_source(
    source: &str,
    id: &str,
) -> Result<SegmentSourceDeletion, WorkbenchError> {
    delete_segment_subtrees_in_timeline_source(source, [id])
}

#[cfg(test)]
fn delete_segment_subtrees_in_timeline_source<'a>(
    source: &str,
    roots: impl IntoIterator<Item = &'a str>,
) -> Result<SegmentSourceDeletion, WorkbenchError> {
    delete_segment_subtrees_in_timeline_source_preferring(source, roots, None)
}

fn delete_segment_subtrees_in_timeline_source_preferring<'a>(
    source: &str,
    roots: impl IntoIterator<Item = &'a str>,
    preferred_goal_anchor: Option<&str>,
) -> Result<SegmentSourceDeletion, WorkbenchError> {
    let timeline =
        Timeline::parse(source).map_err(|error| WorkbenchError::new(error.to_string()))?;
    let segments = segment_descendants_from_roots(&timeline, roots)?;
    let mut reanchored_goals = BTreeMap::<String, String>::new();
    let mut goals = BTreeSet::new();
    for goal in timeline
        .goals
        .values()
        .filter(|goal| segments.contains(&goal.segment))
    {
        let reference = &timeline.segments[&goal.segment];
        let mut compatible = timeline
            .proofs
            .iter()
            .filter(|proof| proof.goal == goal.id && !segments.contains(&proof.segment))
            .filter_map(|proof| timeline.segments.get(&proof.segment))
            .filter(|candidate| {
                candidate.parent == reference.parent
                    && candidate.profile == reference.profile
                    && candidate.start_fingerprint == reference.start_fingerprint
            })
            .map(|candidate| candidate.id.clone())
            .collect::<BTreeSet<_>>();
        let anchor = preferred_goal_anchor
            .filter(|preferred| compatible.contains(*preferred))
            .map(str::to_owned)
            .or_else(|| compatible.pop_first());
        if let Some(anchor) = anchor {
            reanchored_goals.insert(goal.id.clone(), anchor);
        } else {
            goals.insert(goal.id.clone());
        }
    }
    let proofs = timeline
        .proofs
        .iter()
        .filter(|proof| segments.contains(&proof.segment) || goals.contains(&proof.goal))
        .count();

    let mut reanchored_steps = BTreeMap::<(String, String), String>::new();
    if let Some(preferred_id) = preferred_goal_anchor
        && let Some(preferred) = timeline.segments.get(preferred_id)
        && !segments.contains(preferred_id)
    {
        let mut collect = |lineage: &str, steps: &[crate::timeline::ContinuationStep]| {
            if steps.iter().any(|step| step.segment == preferred_id) {
                return;
            }
            for step in steps.iter().filter(|step| segments.contains(&step.segment)) {
                let removed = &timeline.segments[&step.segment];
                let expected_parent = preferred.parent.as_deref().unwrap_or("root");
                if removed.parent == preferred.parent
                    && removed.profile == preferred.profile
                    && removed.start_fingerprint == preferred.start_fingerprint
                    && step.after.parent_segment == expected_parent
                    && step.after.checkpoint_fingerprint == preferred.start_fingerprint
                {
                    reanchored_steps.insert(
                        (lineage.to_owned(), step.segment.clone()),
                        preferred_id.to_owned(),
                    );
                }
            }
        };
        for continuation in timeline.continuations.values() {
            collect(&continuation.name, &continuation.steps);
        }
        for branch in timeline.branches.values() {
            collect(&branch.name, &branch.steps);
        }
    }

    let mut lineages = timeline
        .continuations
        .values()
        .filter(|continuation| {
            !continuation.steps.is_empty()
                && continuation.steps.iter().all(|step| {
                    segments.contains(&step.segment)
                        && !reanchored_steps
                            .contains_key(&(continuation.name.clone(), step.segment.clone()))
                })
        })
        .map(|continuation| continuation.name.clone())
        .collect::<BTreeSet<_>>();
    loop {
        let mut changed = false;
        for branch in timeline.branches.values() {
            if lineages.contains(&branch.name) {
                continue;
            }
            let all_steps_removed = !branch.steps.is_empty()
                && branch.steps.iter().all(|step| {
                    segments.contains(&step.segment)
                        && !reanchored_steps
                            .contains_key(&(branch.name.clone(), step.segment.clone()))
                });
            if segments.contains(&branch.after_segment)
                || lineages.contains(&branch.from_lineage)
                || all_steps_removed
            {
                changed |= lineages.insert(branch.name.clone());
            }
        }
        if !changed {
            break;
        }
    }

    let mut replacement = String::with_capacity(source.len());
    for (index, line) in source.split_inclusive('\n').enumerate() {
        let raw = line.trim_end_matches(['\r', '\n']);
        let tokens =
            tokenize(raw, index + 1).map_err(|error| WorkbenchError::new(error.to_string()))?;
        if tokens.first().map(String::as_str) == Some("goal")
            && let Some(goal_id) = tokens.get(1)
            && let Some(anchor) = reanchored_goals.get(goal_id)
        {
            replacement.push_str(&format!(
                "goal {} on {} predicate {}{}",
                goal_id,
                anchor,
                tokens
                    .get(5)
                    .expect("parsed goal declaration has a predicate"),
                timeline_line_ending(line)
            ));
            continue;
        }
        if tokens.first().map(String::as_str) == Some("continue")
            && let (Some(lineage), Some(segment), Some(pin)) =
                (tokens.get(1), tokens.get(3), tokens.get(5))
            && let Some(anchor) = reanchored_steps.get(&(lineage.clone(), segment.clone()))
        {
            replacement.push_str(&format!(
                "continue {lineage} with {anchor} after {pin}{}",
                timeline_line_ending(line)
            ));
            continue;
        }
        let remove = match tokens.first().map(String::as_str) {
            Some("segment") | Some("label") => tokens
                .get(1)
                .is_some_and(|segment| segments.contains(segment)),
            Some("goal") => {
                tokens.get(1).is_some_and(|goal| goals.contains(goal))
                    || tokens
                        .get(3)
                        .is_some_and(|segment| segments.contains(segment))
            }
            Some("proof") => {
                tokens
                    .get(1)
                    .is_some_and(|segment| segments.contains(segment))
                    || tokens.get(3).is_some_and(|goal| goals.contains(goal))
            }
            Some("continuation") | Some("branch") => tokens
                .get(1)
                .is_some_and(|lineage| lineages.contains(lineage)),
            Some("continue") => {
                let removed_lineage = tokens
                    .get(1)
                    .is_some_and(|lineage| lineages.contains(lineage));
                let removed_segment = tokens
                    .get(3)
                    .is_some_and(|segment| segments.contains(segment));
                let removed_parent = tokens.get(5).is_some_and(|pin| {
                    pin.rsplit_once('@')
                        .is_some_and(|(parent, _)| segments.contains(parent))
                });
                removed_lineage || removed_segment || removed_parent
            }
            _ => false,
        };
        if !remove {
            replacement.push_str(line);
        }
    }

    let replacement_timeline = Timeline::parse(&replacement)
        .map_err(|error| WorkbenchError::new(format!("deleted timeline is invalid: {error}")))?;
    if segments
        .iter()
        .any(|segment| replacement_timeline.segments.contains_key(segment))
        || replacement_timeline.segments.len() + segments.len() != timeline.segments.len()
    {
        return Err(WorkbenchError::new(
            "segment deletion changed unexpected timeline identities",
        ));
    }

    Ok(SegmentSourceDeletion {
        segments,
        goals,
        proofs,
        lineages,
        replacement,
    })
}

fn validated_timeline_edit_path(path: &Path) -> Result<PathBuf, WorkbenchError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot inspect timeline {}: {error}",
            path.display()
        ))
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(WorkbenchError::new(format!(
            "timeline {} is not a physical regular file",
            path.display()
        )));
    }
    fs::canonicalize(path).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot resolve timeline {}: {error}",
            path.display()
        ))
    })
}

struct SegmentDeletePlan {
    preview: SegmentDeletePreview,
    deletion_roots: Vec<String>,
    direct_draft_roots: Vec<String>,
    path: PathBuf,
    original: Vec<u8>,
    replacement: String,
    draft_ids: Vec<String>,
}

struct SegmentDeleteScope<'a> {
    deletion_roots: Vec<String>,
    direct_draft_roots: Vec<String>,
    operation_domain: &'static [u8],
    preferred_goal_anchor: Option<&'a str>,
}

fn segment_delete_plan(
    timeline_path: &Path,
    state_root: &Path,
    id: &str,
    manifests: &BTreeMap<String, DraftManifest>,
    active: &BTreeSet<String>,
) -> Result<SegmentDeletePlan, WorkbenchError> {
    segment_delete_plan_for_roots(
        timeline_path,
        state_root,
        id,
        SegmentDeleteScope {
            deletion_roots: vec![id.to_owned()],
            direct_draft_roots: Vec::new(),
            operation_domain: b"subtree",
            preferred_goal_anchor: None,
        },
        manifests,
        active,
    )
}

fn segment_delete_plan_for_roots(
    timeline_path: &Path,
    state_root: &Path,
    request_id: &str,
    scope: SegmentDeleteScope<'_>,
    manifests: &BTreeMap<String, DraftManifest>,
    active: &BTreeSet<String>,
) -> Result<SegmentDeletePlan, WorkbenchError> {
    let SegmentDeleteScope {
        deletion_roots,
        direct_draft_roots,
        operation_domain,
        preferred_goal_anchor,
    } = scope;
    let path = validated_timeline_edit_path(timeline_path)?;
    let original = fs::read(&path).map_err(|error| {
        WorkbenchError::new(format!("cannot read timeline {}: {error}", path.display()))
    })?;
    let source = std::str::from_utf8(&original)
        .map_err(|_| WorkbenchError::new("timeline source is not UTF-8"))?;
    let deletion = if deletion_roots.is_empty() {
        SegmentSourceDeletion {
            segments: BTreeSet::new(),
            goals: BTreeSet::new(),
            proofs: 0,
            lineages: BTreeSet::new(),
            replacement: source.into(),
        }
    } else {
        delete_segment_subtrees_in_timeline_source_preferring(
            source,
            deletion_roots.iter().map(String::as_str),
            preferred_goal_anchor,
        )?
    };

    for draft_id in &direct_draft_roots {
        if !manifests.contains_key(draft_id) {
            return Err(WorkbenchError::new(format!(
                "unknown direct sibling draft {draft_id:?}"
            )));
        }
    }
    let roots = manifests
        .values()
        .filter_map(|manifest| match &manifest.parent {
            DraftParent::Segment { id, .. } if deletion.segments.contains(id) => {
                Some(manifest.id.as_str())
            }
            _ => None,
        })
        .chain(direct_draft_roots.iter().map(String::as_str));
    let draft_deletion = draft_descendants_from_roots(manifests, roots);
    let drafts_root = validated_drafts_root(state_root)?;
    for draft_id in &draft_deletion {
        let manifest = &manifests[draft_id];
        if draft_is_active(&drafts_root.join(draft_id), manifest, active) {
            return Err(WorkbenchError::new(format!(
                "cannot delete segment {request_id:?}: attached recording {draft_id:?} is active"
            )));
        }
    }

    let graph_revision = draft_graph_revision(manifests)?;
    let mut digest = Sha256::new();
    digest.update(b"dusklight.route-workbench.segment-delete.v1\0");
    digest.update((operation_domain.len() as u64).to_le_bytes());
    digest.update(operation_domain);
    digest.update((original.len() as u64).to_le_bytes());
    digest.update(&original);
    digest.update(graph_revision.as_bytes());
    digest.update(deletion.replacement.as_bytes());
    for segment in &deletion.segments {
        digest.update((segment.len() as u64).to_le_bytes());
        digest.update(segment.as_bytes());
    }
    for draft in &draft_deletion {
        digest.update((draft.len() as u64).to_le_bytes());
        digest.update(draft.as_bytes());
    }
    let confirmation_token = format!("{:x}", digest.finalize());
    let timeline = Timeline::parse(source).expect("validated segment deletion source");
    let segments = deletion
        .segments
        .iter()
        .map(|segment_id| {
            let segment = &timeline.segments[segment_id];
            SegmentDeleteImpact {
                id: segment_id.clone(),
                name: segment.name.clone().unwrap_or_else(|| segment_id.clone()),
            }
        })
        .collect();
    let drafts = draft_deletion
        .iter()
        .map(|draft_id| {
            let manifest = &manifests[draft_id];
            DraftDeleteImpact {
                id: draft_id.clone(),
                label: manifest.label.clone(),
                status: manifest.status,
            }
        })
        .collect();
    let draft_ids = draft_deletion.into_iter().collect();
    Ok(SegmentDeletePlan {
        preview: SegmentDeletePreview {
            schema: SEGMENT_DELETE_PREVIEW_SCHEMA.into(),
            id: request_id.into(),
            segments,
            goals: deletion.goals.into_iter().collect(),
            proofs: deletion.proofs,
            lineages: deletion.lineages.into_iter().collect(),
            drafts,
            confirmation_token,
        },
        deletion_roots,
        direct_draft_roots,
        path,
        original,
        replacement: deletion.replacement,
        draft_ids,
    })
}

fn preview_segment_deletion(
    timeline_path: &Path,
    state_root: &Path,
    id: &str,
) -> Result<SegmentDeletePreview, WorkbenchError> {
    let active = active_recordings()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let manifests = scan_draft_manifests_with_active(state_root, &active)?;
    Ok(segment_delete_plan(timeline_path, state_root, id, &manifests, &active)?.preview)
}

fn structural_sibling_context(
    timeline: &Timeline,
    keep_id: &str,
) -> Result<(String, Vec<String>), WorkbenchError> {
    let selected = timeline
        .segments
        .get(keep_id)
        .ok_or_else(|| WorkbenchError::new(format!("unknown checked-in segment {keep_id:?}")))?;
    let parent = selected.parent.as_deref().ok_or_else(|| {
        WorkbenchError::new("the root segment has no structural siblings to delete")
    })?;
    let roots = timeline
        .segments
        .values()
        .filter(|segment| segment.id != keep_id && segment.parent.as_deref() == Some(parent))
        .map(|segment| segment.id.clone())
        .collect::<Vec<_>>();
    Ok((parent.into(), roots))
}

struct SiblingDeletePlan {
    deletion: SegmentDeletePlan,
    generated: Vec<GeneratedDeleteImpact>,
    generated_candidate_ids: Vec<String>,
}

fn sibling_delete_plan(
    timeline_path: &Path,
    repository_root: &Path,
    state_root: &Path,
    keep_id: &str,
    manifests: &BTreeMap<String, DraftManifest>,
    active: &BTreeSet<String>,
) -> Result<SiblingDeletePlan, WorkbenchError> {
    let initial_path = validated_timeline_edit_path(timeline_path)?;
    let initial_source = fs::read_to_string(&initial_path).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot read timeline {}: {error}",
            initial_path.display()
        ))
    })?;
    let initial_timeline =
        Timeline::parse(&initial_source).map_err(|error| WorkbenchError::new(error.to_string()))?;
    let (parent_id, roots) = structural_sibling_context(&initial_timeline, keep_id)?;
    let direct_draft_roots = manifests
        .values()
        .filter_map(|manifest| match &manifest.parent {
            DraftParent::Segment { id, .. } if id == &parent_id => Some(manifest.id.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let mut plan = segment_delete_plan_for_roots(
        timeline_path,
        state_root,
        keep_id,
        SegmentDeleteScope {
            deletion_roots: roots.clone(),
            direct_draft_roots,
            operation_domain: b"delete-siblings",
            preferred_goal_anchor: Some(keep_id),
        },
        manifests,
        active,
    )?;

    // The roots must have been derived from the exact bytes guarded by the plan's token.
    let planned_source = std::str::from_utf8(&plan.original)
        .map_err(|_| WorkbenchError::new("timeline source is not UTF-8"))?;
    let planned_timeline =
        Timeline::parse(planned_source).map_err(|error| WorkbenchError::new(error.to_string()))?;
    if structural_sibling_context(&planned_timeline, keep_id)? != (parent_id.clone(), roots)
        || plan
            .preview
            .segments
            .iter()
            .any(|segment| segment.id == keep_id)
    {
        return Err(WorkbenchError::new(
            "timeline topology changed while planning sibling deletion; reload and retry",
        ));
    }
    let generated = visible_generated_search_projections(
        &planned_timeline,
        &repository_root.join("build/search"),
        state_root,
    )?
    .into_iter()
    .filter(|projection| projection.segment.parent.as_deref() == Some(parent_id.as_str()))
    .filter_map(|projection| {
        let generated = projection.segment.generated?;
        Some(GeneratedDeleteImpact {
            id: projection.segment.id,
            name: projection
                .segment
                .name
                .unwrap_or_else(|| generated.candidate_id.clone()),
            candidate_id: generated.candidate_id,
            run: generated.run,
        })
    })
    .collect::<Vec<_>>();
    if plan.deletion_roots.is_empty() && plan.direct_draft_roots.is_empty() && generated.is_empty()
    {
        return Err(WorkbenchError::new(format!(
            "segment {keep_id:?} has no displayed siblings to delete"
        )));
    }
    let tombstones = load_generated_search_tombstones(state_root)?;
    let mut digest = Sha256::new();
    digest.update(b"dusklight.route-workbench.displayed-sibling-delete.v1\0");
    digest.update(plan.preview.confirmation_token.as_bytes());
    digest.update(
        serde_json::to_vec(&tombstones)
            .map_err(|error| WorkbenchError::new(format!("cannot hash tombstones: {error}")))?,
    );
    for candidate in &generated {
        digest.update(candidate.candidate_id.as_bytes());
        digest.update(candidate.run.as_bytes());
    }
    plan.preview.confirmation_token = format!("{:x}", digest.finalize());
    let generated_candidate_ids = generated
        .iter()
        .map(|candidate| candidate.candidate_id.clone())
        .collect();
    Ok(SiblingDeletePlan {
        deletion: plan,
        generated,
        generated_candidate_ids,
    })
}

fn sibling_preview(plan: &SiblingDeletePlan) -> SiblingDeletePreview {
    let deletion = &plan.deletion;
    let root_ids = plan
        .deletion
        .deletion_roots
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    SiblingDeletePreview {
        schema: SIBLING_DELETE_PREVIEW_SCHEMA.into(),
        keep_id: deletion.preview.id.clone(),
        sibling_roots: deletion
            .preview
            .segments
            .iter()
            .filter(|segment| root_ids.contains(segment.id.as_str()))
            .cloned()
            .collect(),
        draft_roots: deletion
            .preview
            .drafts
            .iter()
            .filter(|draft| deletion.direct_draft_roots.contains(&draft.id))
            .cloned()
            .collect(),
        generated: plan.generated.clone(),
        segments: deletion.preview.segments.clone(),
        goals: deletion.preview.goals.clone(),
        proofs: deletion.preview.proofs,
        lineages: deletion.preview.lineages.clone(),
        drafts: deletion.preview.drafts.clone(),
        confirmation_token: deletion.preview.confirmation_token.clone(),
    }
}

fn preview_sibling_deletion(
    timeline_path: &Path,
    repository_root: &Path,
    state_root: &Path,
    keep_id: &str,
) -> Result<SiblingDeletePreview, WorkbenchError> {
    let active = active_recordings()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let manifests = scan_draft_manifests_with_active(state_root, &active)?;
    let plan = sibling_delete_plan(
        timeline_path,
        repository_root,
        state_root,
        keep_id,
        &manifests,
        &active,
    )?;
    Ok(sibling_preview(&plan))
}

fn rename_segment(
    timeline_path: &Path,
    request: &BrowserSegmentRenameRequest,
) -> Result<SegmentRenameResult, SegmentRenameError> {
    let name = validate_segment_name(&request.name)?;
    let _edit = timeline_edits()
        .lock()
        .map_err(|_| WorkbenchError::new("timeline label edit lock is poisoned"))?;
    let path = validated_timeline_edit_path(timeline_path)?;
    let original = fs::read(&path).map_err(|error| {
        WorkbenchError::new(format!("cannot read timeline {}: {error}", path.display()))
    })?;
    let source = String::from_utf8(original.clone())
        .map_err(|_| WorkbenchError::new("timeline source is not UTF-8"))?;
    let timeline =
        Timeline::parse(&source).map_err(|error| WorkbenchError::new(error.to_string()))?;
    let segment = timeline
        .segments
        .get(&request.id)
        .ok_or_else(|| WorkbenchError::new(format!("unknown segment {:?}", request.id)))?;
    if segment.name != request.expected_name {
        return Err(SegmentRenameError::Conflict(
            "segment name changed; reload before renaming".into(),
        ));
    }
    let replacement_source = rename_segment_in_timeline_source(&source, &request.id, &name)?;
    let replacement_timeline = Timeline::parse(&replacement_source)
        .map_err(|error| WorkbenchError::new(error.to_string()))?;
    if replacement_timeline
        .segments
        .get(&request.id)
        .and_then(|segment| segment.name.as_deref())
        != Some(name.as_str())
    {
        return Err(
            WorkbenchError::new("renamed timeline did not preserve segment identity").into(),
        );
    }

    let directory = path
        .parent()
        .ok_or_else(|| WorkbenchError::new("timeline has no parent directory"))?;
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| WorkbenchError::new("timeline filename is not UTF-8"))?;
    let nonce = random_session_token()?;
    let temporary = directory.join(format!(".{filename}.{nonce}.tmp"));
    let backup = directory.join(format!(".{filename}.{nonce}.rollback"));
    let mut temporary_file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| {
            WorkbenchError::new(format!(
                "cannot create adjacent timeline temporary file {}: {error}",
                temporary.display()
            ))
        })?;
    let mut temporary_cleanup = RemoveFileOnDrop(Some(temporary.clone()));
    temporary_file
        .write_all(replacement_source.as_bytes())
        .and_then(|()| temporary_file.sync_all())
        .map_err(|error| {
            WorkbenchError::new(format!(
                "cannot flush timeline temporary file {}: {error}",
                temporary.display()
            ))
        })?;
    drop(temporary_file);

    if validated_timeline_edit_path(timeline_path)? != path
        || fs::read(&path).ok() != Some(original.clone())
    {
        return Err(SegmentRenameError::Conflict(
            "timeline changed while preparing rename; reload and retry".into(),
        ));
    }
    fs::rename(&path, &backup).map_err(|error| {
        WorkbenchError::new(format!("cannot stage timeline rollback backup: {error}"))
    })?;
    if fs::read(&backup).ok() != Some(original) {
        fs::rename(&backup, &path).map_err(|rollback| {
            WorkbenchError::new(format!(
                "timeline changed while staging its rollback backup and could not be restored: {rollback}"
            ))
        })?;
        return Err(
            WorkbenchError::new("timeline changed while staging its rollback backup").into(),
        );
    }
    if let Err(error) = fs::rename(&temporary, &path) {
        fs::rename(&backup, &path).map_err(|rollback| {
            WorkbenchError::new(format!(
                "cannot replace timeline ({error}) or restore rollback backup ({rollback})"
            ))
        })?;
        return Err(WorkbenchError::new(format!("cannot replace timeline: {error}")).into());
    }
    temporary_cleanup.0 = None;
    let _ = fs::remove_file(backup);
    Ok(SegmentRenameResult {
        schema: SEGMENT_RENAME_RESULT_SCHEMA.into(),
        id: request.id.clone(),
        name,
    })
}

fn draft_trash_root(state_root: &Path) -> Result<PathBuf, WorkbenchError> {
    fs::create_dir_all(state_root).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot create state root {}: {error}",
            state_root.display()
        ))
    })?;
    let state_root = fs::canonicalize(state_root).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot resolve state root {}: {error}",
            state_root.display()
        ))
    })?;
    let trash = state_root.join(DRAFT_TRASH_DIRECTORY).join("drafts");
    fs::create_dir_all(&trash).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot create draft trash {}: {error}",
            trash.display()
        ))
    })?;
    let trash = fs::canonicalize(&trash).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot resolve draft trash {}: {error}",
            trash.display()
        ))
    })?;
    if !trash.starts_with(&state_root) || trash == state_root {
        return Err(WorkbenchError::new("draft trash escapes the state root"));
    }
    Ok(trash)
}

struct DraftTrashMove {
    root: PathBuf,
    transaction: PathBuf,
    moved: Vec<(String, PathBuf)>,
}

impl DraftTrashMove {
    fn rollback(&mut self) -> Result<(), WorkbenchError> {
        let mut failures = Vec::new();
        for (draft_id, moved_path) in self.moved.iter().rev() {
            if let Err(error) = fs::rename(moved_path, self.root.join(draft_id)) {
                failures.push(format!("{draft_id}: {error}"));
            }
        }
        if failures.is_empty() {
            self.moved.clear();
            let _ = fs::remove_dir(&self.transaction);
            Ok(())
        } else {
            Err(WorkbenchError::new(format!(
                "cannot restore drafts after failed timeline edit: {}",
                failures.join(", ")
            )))
        }
    }
}

fn move_draft_set_to_trash(
    state_root: &Path,
    draft_ids: &[String],
    token: &str,
) -> Result<Option<DraftTrashMove>, WorkbenchError> {
    if draft_ids.is_empty() {
        return Ok(None);
    }
    let root = validated_drafts_root(state_root)?;
    let mut sources = Vec::with_capacity(draft_ids.len());
    for draft_id in draft_ids {
        sources.push((
            draft_id.clone(),
            validated_draft_directory(&root, draft_id)?,
        ));
    }
    let trash = draft_trash_root(state_root)?;
    let nonce = random_session_token()?;
    let transaction = trash.join(format!(
        "{}-{}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
        &token[..16],
        nonce
    ));
    fs::create_dir(&transaction).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot create draft trash transaction {}: {error}",
            transaction.display()
        ))
    })?;
    let transaction = fs::canonicalize(&transaction).map_err(|error| {
        WorkbenchError::new(format!("cannot resolve draft trash transaction: {error}"))
    })?;
    if transaction.parent() != Some(trash.as_path()) {
        return Err(WorkbenchError::new(
            "draft trash transaction escapes the trash root",
        ));
    }

    let mut moved = Vec::new();
    for (draft_id, source) in &sources {
        let destination = transaction.join(draft_id);
        if let Err(error) = fs::rename(source, &destination) {
            let mut transaction_state = DraftTrashMove {
                root,
                transaction,
                moved,
            };
            let rollback = transaction_state.rollback().err();
            let suffix = rollback
                .map(|error| format!("; {error}"))
                .unwrap_or_default();
            return Err(WorkbenchError::new(format!(
                "cannot move draft {draft_id:?} into recoverable trash: {error}{suffix}"
            )));
        }
        moved.push((draft_id.clone(), destination));
    }
    Ok(Some(DraftTrashMove {
        root,
        transaction,
        moved,
    }))
}

#[derive(Debug)]
enum SegmentDeleteError {
    Conflict(String),
    Invalid(WorkbenchError),
}

impl fmt::Display for SegmentDeleteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Conflict(message) => formatter.write_str(message),
            Self::Invalid(error) => error.fmt(formatter),
        }
    }
}

impl From<WorkbenchError> for SegmentDeleteError {
    fn from(error: WorkbenchError) -> Self {
        Self::Invalid(error)
    }
}

fn rollback_draft_move(moved: &mut Option<DraftTrashMove>) -> String {
    moved
        .as_mut()
        .and_then(|transaction| transaction.rollback().err())
        .map(|error| format!("; {error}"))
        .unwrap_or_default()
}

struct AppliedSegmentDeletion {
    segments: Vec<String>,
    drafts: Vec<String>,
    trash_transaction: Option<PathBuf>,
}

fn apply_segment_delete_plan(
    timeline_path: &Path,
    state_root: &Path,
    plan: SegmentDeletePlan,
) -> Result<AppliedSegmentDeletion, SegmentDeleteError> {
    let directory = plan
        .path
        .parent()
        .ok_or_else(|| WorkbenchError::new("timeline has no parent directory"))?;
    let filename = plan
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| WorkbenchError::new("timeline filename is not UTF-8"))?;
    let nonce = random_session_token()?;
    let temporary = directory.join(format!(".{filename}.{nonce}.tmp"));
    let backup = directory.join(format!(".{filename}.{nonce}.rollback"));
    let mut temporary_file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| {
            WorkbenchError::new(format!(
                "cannot create adjacent timeline temporary file {}: {error}",
                temporary.display()
            ))
        })?;
    let mut temporary_cleanup = RemoveFileOnDrop(Some(temporary.clone()));
    temporary_file
        .write_all(plan.replacement.as_bytes())
        .and_then(|()| temporary_file.sync_all())
        .map_err(|error| {
            WorkbenchError::new(format!(
                "cannot flush timeline temporary file {}: {error}",
                temporary.display()
            ))
        })?;
    drop(temporary_file);

    if validated_timeline_edit_path(timeline_path)? != plan.path
        || fs::read(&plan.path).ok() != Some(plan.original.clone())
    {
        return Err(SegmentDeleteError::Conflict(
            "timeline changed while preparing deletion; reload and retry".into(),
        ));
    }

    let mut moved = move_draft_set_to_trash(
        state_root,
        &plan.draft_ids,
        &plan.preview.confirmation_token,
    )?;
    if let Err(error) = fs::rename(&plan.path, &backup) {
        let rollback = rollback_draft_move(&mut moved);
        return Err(WorkbenchError::new(format!(
            "cannot stage timeline rollback backup: {error}{rollback}"
        ))
        .into());
    }
    if fs::read(&backup).ok() != Some(plan.original.clone()) {
        let restore = fs::rename(&backup, &plan.path).err();
        let rollback = rollback_draft_move(&mut moved);
        let restore = restore
            .map(|error| format!("; cannot restore timeline: {error}"))
            .unwrap_or_default();
        return Err(WorkbenchError::new(format!(
            "timeline changed while staging its rollback backup{restore}{rollback}"
        ))
        .into());
    }
    if let Err(error) = fs::rename(&temporary, &plan.path) {
        let restore = fs::rename(&backup, &plan.path).err();
        let rollback = rollback_draft_move(&mut moved);
        let restore = restore
            .map(|restore| format!("; cannot restore timeline: {restore}"))
            .unwrap_or_default();
        return Err(WorkbenchError::new(format!(
            "cannot replace timeline: {error}{restore}{rollback}"
        ))
        .into());
    }
    temporary_cleanup.0 = None;
    let _ = fs::remove_file(backup);

    Ok(AppliedSegmentDeletion {
        segments: plan
            .preview
            .segments
            .into_iter()
            .map(|segment| segment.id)
            .collect(),
        drafts: plan.draft_ids,
        trash_transaction: moved.map(|transaction| transaction.transaction),
    })
}

fn apply_segment_deletion(
    timeline_path: &Path,
    state_root: &Path,
    request: &BrowserSegmentDeleteApplyRequest,
) -> Result<SegmentDeleteResult, SegmentDeleteError> {
    let _edit = timeline_edits()
        .lock()
        .map_err(|_| WorkbenchError::new("timeline edit lock is poisoned"))?;
    let active = active_recordings()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let manifests = scan_draft_manifests_with_active(state_root, &active)?;
    let plan = segment_delete_plan(timeline_path, state_root, &request.id, &manifests, &active)?;
    if request.confirmation_token != plan.preview.confirmation_token {
        return Err(SegmentDeleteError::Conflict(
            "timeline or attached drafts changed after preview; reload and confirm deletion again"
                .into(),
        ));
    }
    let result = apply_segment_delete_plan(timeline_path, state_root, plan)?;
    Ok(SegmentDeleteResult {
        schema: SEGMENT_DELETE_RESULT_SCHEMA.into(),
        id: request.id.clone(),
        segments: result.segments,
        drafts: result.drafts,
        trash_transaction: result.trash_transaction,
    })
}

struct AppliedTombstoneEdit {
    target: PathBuf,
    backup: Option<PathBuf>,
    had_original: bool,
    active: bool,
}

impl AppliedTombstoneEdit {
    fn rollback(&mut self) -> Result<(), WorkbenchError> {
        if !self.active {
            return Ok(());
        }
        if self.target.exists() {
            fs::remove_file(&self.target).map_err(|error| {
                WorkbenchError::new(format!("cannot roll back search tombstones: {error}"))
            })?;
        }
        if self.had_original {
            let backup = self.backup.as_ref().ok_or_else(|| {
                WorkbenchError::new("search tombstone rollback backup is missing")
            })?;
            fs::rename(backup, &self.target).map_err(|error| {
                WorkbenchError::new(format!("cannot restore search tombstones: {error}"))
            })?;
        }
        self.active = false;
        Ok(())
    }

    fn commit(mut self) {
        if let Some(backup) = self.backup.take() {
            let _ = fs::remove_file(backup);
        }
        self.active = false;
    }
}

impl Drop for AppliedTombstoneEdit {
    fn drop(&mut self) {
        let _ = self.rollback();
    }
}

fn apply_generated_search_tombstones(
    state_root: &Path,
    candidate_ids: &[String],
) -> Result<Option<AppliedTombstoneEdit>, WorkbenchError> {
    if candidate_ids.is_empty() {
        return Ok(None);
    }
    fs::create_dir_all(state_root).map_err(|error| {
        WorkbenchError::new(format!("cannot create state root for tombstones: {error}"))
    })?;
    let root = fs::canonicalize(state_root)
        .map_err(|error| WorkbenchError::new(format!("cannot resolve state root: {error}")))?;
    let target = root.join(GENERATED_SEARCH_TOMBSTONES);
    let original = match fs::symlink_metadata(&target) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(WorkbenchError::new(
                    "generated search tombstones are not a physical file",
                ));
            }
            Some(fs::read(&target).map_err(|error| {
                WorkbenchError::new(format!("cannot read search tombstones: {error}"))
            })?)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(WorkbenchError::new(format!(
                "cannot inspect search tombstones: {error}"
            )));
        }
    };
    let mut tombstones = load_generated_search_tombstones(&root)?;
    tombstones
        .candidate_ids
        .extend(candidate_ids.iter().cloned());
    let replacement = serde_json::to_vec_pretty(&tombstones)
        .map_err(|error| WorkbenchError::new(format!("cannot encode tombstones: {error}")))?;
    let nonce = random_session_token()?;
    let temporary = root.join(format!(".{GENERATED_SEARCH_TOMBSTONES}.{nonce}.tmp"));
    let backup = original
        .as_ref()
        .map(|_| root.join(format!(".{GENERATED_SEARCH_TOMBSTONES}.{nonce}.rollback")));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| WorkbenchError::new(format!("cannot stage tombstones: {error}")))?;
    file.write_all(&replacement)
        .and_then(|()| file.sync_all())
        .map_err(|error| WorkbenchError::new(format!("cannot flush tombstones: {error}")))?;
    drop(file);
    let mut temporary_cleanup = RemoveFileOnDrop(Some(temporary.clone()));
    if fs::read(&target).ok() != original {
        return Err(WorkbenchError::new(
            "generated search tombstones changed while preparing deletion",
        ));
    }
    if let Some(backup) = &backup {
        fs::rename(&target, backup).map_err(|error| {
            WorkbenchError::new(format!("cannot stage search tombstone rollback: {error}"))
        })?;
    }
    if let Err(error) = fs::rename(&temporary, &target) {
        if let Some(backup) = &backup {
            let _ = fs::rename(backup, &target);
        }
        return Err(WorkbenchError::new(format!(
            "cannot install generated search tombstones: {error}"
        )));
    }
    temporary_cleanup.0 = None;
    Ok(Some(AppliedTombstoneEdit {
        target,
        backup,
        had_original: original.is_some(),
        active: true,
    }))
}

fn apply_sibling_deletion(
    timeline_path: &Path,
    repository_root: &Path,
    state_root: &Path,
    request: &BrowserSiblingDeleteApplyRequest,
) -> Result<SiblingDeleteResult, SegmentDeleteError> {
    let _edit = timeline_edits()
        .lock()
        .map_err(|_| WorkbenchError::new("timeline edit lock is poisoned"))?;
    let active = active_recordings()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let manifests = scan_draft_manifests_with_active(state_root, &active)?;
    let plan = sibling_delete_plan(
        timeline_path,
        repository_root,
        state_root,
        &request.keep_id,
        &manifests,
        &active,
    )?;
    if request.confirmation_token != plan.deletion.preview.confirmation_token {
        return Err(SegmentDeleteError::Conflict(
            "timeline or attached drafts changed after preview; reload and confirm sibling deletion again"
                .into(),
        ));
    }
    let sibling_roots = plan.deletion.deletion_roots.clone();
    let draft_roots = plan.deletion.direct_draft_roots.clone();
    let generated_candidates = plan.generated_candidate_ids.clone();
    let mut tombstone_edit =
        apply_generated_search_tombstones(state_root, &plan.generated_candidate_ids)?;
    let result = match apply_segment_delete_plan(timeline_path, state_root, plan.deletion) {
        Ok(result) => result,
        Err(error) => {
            if let Some(edit) = tombstone_edit.as_mut() {
                edit.rollback()?;
            }
            return Err(error);
        }
    };
    if let Some(edit) = tombstone_edit {
        edit.commit();
    }
    Ok(SiblingDeleteResult {
        schema: SIBLING_DELETE_RESULT_SCHEMA.into(),
        keep_id: request.keep_id.clone(),
        sibling_roots,
        draft_roots,
        generated_candidates,
        segments: result.segments,
        drafts: result.drafts,
        trash_transaction: result.trash_transaction,
    })
}

fn apply_draft_deletion(
    state_root: &Path,
    request: &BrowserDraftDeleteApplyRequest,
) -> Result<DraftDeleteResult, WorkbenchError> {
    let active = active_recordings()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let manifests = scan_draft_manifests_with_active(state_root, &active)?;
    let preview = draft_delete_preview_locked(state_root, &request.id, &manifests, &active)?;
    if request.confirmation_token != preview.confirmation_token {
        return Err(WorkbenchError::new(
            "draft graph changed after preview; request a new deletion preview",
        ));
    }

    let draft_ids = preview
        .drafts
        .iter()
        .map(|draft| draft.id.clone())
        .collect::<Vec<_>>();
    let moved = move_draft_set_to_trash(state_root, &draft_ids, &preview.confirmation_token)?
        .expect("a draft deletion always moves at least one draft");

    Ok(DraftDeleteResult {
        schema: DRAFT_DELETE_RESULT_SCHEMA.into(),
        id: request.id.clone(),
        graph_revision: preview.graph_revision,
        drafts: preview.drafts.into_iter().map(|draft| draft.id).collect(),
        trash_transaction: moved.transaction,
    })
}

fn read_draft_launch(directory: &Path, manifest: &DraftManifest) -> Option<DraftLaunch> {
    let bytes = fs::read(directory.join(DRAFT_LAUNCH)).ok()?;
    let launch: DraftLaunch = serde_json::from_slice(&bytes).ok()?;
    (launch.schema == "dusklight.route-workbench.launch.v2"
        && launch.id == manifest.id
        && launch.session_token == manifest.session_token)
        .then_some(launch)
}

fn graph_drafts_from_manifests(
    timeline: &Timeline,
    repository_root: &Path,
    state_root: &Path,
    manifests: BTreeMap<String, DraftManifest>,
) -> Result<Vec<GraphDraft>, WorkbenchError> {
    let root = drafts_root(state_root)?;
    let mut memo = BTreeMap::new();
    let mut anchor_digests = BTreeMap::new();
    for id in manifests.keys() {
        let _ = validate_draft_structure(
            id,
            &manifests,
            timeline,
            repository_root,
            &root,
            &mut memo,
            &mut anchor_digests,
        );
    }
    Ok(manifests
        .into_values()
        .map(|manifest| {
            let mut error = manifest.error.clone();
            if error.is_none() && manifest.status == DraftStatus::Ready {
                error = memo
                    .get(&manifest.id)
                    .and_then(|result| result.clone().err());
            }
            let playable = manifest.status == DraftStatus::Ready && error.is_none();
            GraphDraft {
                id: manifest.id,
                label: manifest.label,
                parent: manifest.parent,
                created_unix_ms: manifest.created_unix_ms,
                status: manifest.status,
                frame_count: manifest.frames,
                playable,
                endpoint_kind: manifest.endpoint_kind,
                verification: manifest.verification,
                tape_sha256: manifest.tape_sha256,
                result_tape_sha256: manifest.result_tape_sha256,
                tape_bytes: manifest.tape_bytes,
                thumbnail: None,
                error,
            }
        })
        .collect())
}

fn validate_draft_structure(
    id: &str,
    manifests: &BTreeMap<String, DraftManifest>,
    timeline: &Timeline,
    repository_root: &Path,
    drafts_root: &Path,
    memo: &mut BTreeMap<String, Result<(), String>>,
    anchor_digests: &mut BTreeMap<(String, usize), Result<String, String>>,
) -> Result<(), String> {
    if let Some(result) = memo.get(id) {
        return result.clone();
    }
    memo.insert(
        id.to_owned(),
        Err("draft parent graph contains a cycle".into()),
    );
    let result = (|| {
        let manifest = manifests
            .get(id)
            .ok_or_else(|| "parent draft is missing".to_owned())?;
        if manifest.status != DraftStatus::Ready {
            return Err(format!("parent draft {id:?} is not ready"));
        }
        for (name, digest) in [
            ("parent", Some(manifest.parent_tape_sha256.as_str())),
            ("continuation", manifest.tape_sha256.as_deref()),
            ("result", manifest.result_tape_sha256.as_deref()),
        ] {
            if !digest.is_some_and(valid_sha256) {
                return Err(format!("draft {id:?} has invalid {name} tape digest"));
            }
        }
        if manifest.tape_bytes.is_none() || manifest.frames.is_none() {
            return Err(format!("draft {id:?} lacks finalized tape metadata"));
        }
        let draft_directory = fs::canonicalize(drafts_root.join(id))
            .map_err(|_| format!("draft {id:?} directory is missing"))?;
        let tape_path = fs::canonicalize(draft_directory.join(DRAFT_TAPE))
            .map_err(|_| format!("draft {id:?} continuation tape is missing"))?;
        if !draft_directory.starts_with(drafts_root) || !tape_path.starts_with(&draft_directory) {
            return Err(format!("draft {id:?} continuation tape escapes the store"));
        }
        let tape_bytes = fs::read(&tape_path)
            .map_err(|_| format!("draft {id:?} continuation tape is unreadable"))?;
        if manifest.tape_bytes != Some(tape_bytes.len() as u64)
            || manifest.tape_sha256.as_deref()
                != Some(format!("{:x}", Sha256::digest(&tape_bytes)).as_str())
        {
            return Err(format!("draft {id:?} continuation tape was tampered"));
        }
        let continuation = InputTape::decode(&tape_bytes)
            .map_err(|_| format!("draft {id:?} continuation tape is invalid"))?
            .tape;
        if continuation.frames.is_empty()
            || manifest.frames != Some(continuation.frames.len() as u64)
        {
            return Err(format!(
                "draft {id:?} continuation frame metadata is inconsistent"
            ));
        }
        match &manifest.parent {
            DraftParent::Milestone {
                id,
                program_sha256,
                definition_sha256,
                boundary_fingerprint,
            } => {
                let program = milestone_program_projection(timeline, repository_root)
                    .map_err(|error| error.to_string())?
                    .ok_or_else(|| "Boot parent has no authored milestone program".to_owned())?;
                let definition = program
                    .definitions
                    .iter()
                    .find(|definition| definition.name == *id)
                    .ok_or_else(|| "Boot parent milestone no longer exists".to_owned())?;
                if program.program_sha256 != *program_sha256
                    || definition.definition_sha256 != *definition_sha256
                    || !is_exact_boot_boundary_predicate(definition)
                    || !manifest.start_boundary_verified
                    || !boundary_fingerprint
                        .as_deref()
                        .is_some_and(native_fingerprint)
                    || manifest.parent_tape_sha256
                        != tape_digest(&InputTape::default()).map_err(|error| error.to_string())?
                {
                    return Err("Boot milestone parent proof is missing or stale".into());
                }
            }
            DraftParent::Segment {
                id: segment_id,
                terminal_milestone: _,
                boundary_fingerprint,
            } => {
                let segment = timeline
                    .segments
                    .get(segment_id)
                    .ok_or_else(|| "parent segment no longer exists".to_owned())?;
                if segment.end_fingerprint != *boundary_fingerprint
                    || !manifest.start_boundary_verified
                {
                    return Err("curated segment anchor is no longer exact".into());
                }
                let key = (segment_id.clone(), 0);
                let parent_digest = if let Some(result) = anchor_digests.get(&key) {
                    result.clone()?
                } else {
                    let result = materialize_segment_chain(timeline, repository_root, segment_id)
                        .map_err(|error| error.to_string())
                        .and_then(|materialized| {
                            tape_digest(&materialized.tape).map_err(|error| error.to_string())
                        });
                    anchor_digests.insert(key, result.clone());
                    result?
                };
                if manifest.parent_tape_sha256 != parent_digest {
                    return Err("curated parent tape digest changed".into());
                }
            }
            DraftParent::Draft {
                id: parent_id,
                parent_tape_sha256,
            } => {
                validate_draft_structure(
                    parent_id,
                    manifests,
                    timeline,
                    repository_root,
                    drafts_root,
                    memo,
                    anchor_digests,
                )?;
                let parent = &manifests[parent_id];
                if parent.result_tape_sha256.as_deref() != Some(parent_tape_sha256)
                    || manifest.parent_tape_sha256 != *parent_tape_sha256
                {
                    return Err("draft parent chain digest mismatch".into());
                }
            }
        }
        Ok(())
    })();
    memo.insert(id.to_owned(), result.clone());
    result
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn valid_draft_id(id: &str) -> bool {
    id.starts_with("draft-")
        && id.len() <= 80
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

#[cfg(windows)]
fn process_is_alive(pid: u32) -> bool {
    type Handle = *mut std::ffi::c_void;
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, process_id: u32) -> Handle;
        fn GetExitCodeProcess(process: Handle, exit_code: *mut u32) -> i32;
        fn CloseHandle(object: Handle) -> i32;
    }
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const STILL_ACTIVE: u32 = 259;
    // SAFETY: handles are checked for null, the output pointer is valid for the
    // duration of the call, and every opened handle is closed exactly once.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return false;
        }
        let mut exit_code = 0;
        let ok = GetExitCodeProcess(process, &mut exit_code);
        let _ = CloseHandle(process);
        ok != 0 && exit_code == STILL_ACTIVE
    }
}

#[cfg(target_os = "linux")]
fn process_is_alive(pid: u32) -> bool {
    Path::new("/proc").join(pid.to_string()).exists()
}

#[cfg(all(unix, not(target_os = "linux")))]
fn process_is_alive(pid: u32) -> bool {
    unsafe extern "C" {
        fn kill(pid: i32, signal: i32) -> i32;
    }
    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };
    // SAFETY: signal zero does not deliver a signal; it only asks the kernel
    // whether this process exists and is visible to the caller.
    let visible = unsafe { kill(pid, 0) == 0 };
    visible || std::io::Error::last_os_error().kind() == std::io::ErrorKind::PermissionDenied
}

#[cfg(not(any(windows, unix)))]
fn process_is_alive(_pid: u32) -> bool {
    false
}

fn write_draft_manifest(
    directory: &Path,
    manifest: &DraftManifest,
    finalized: bool,
) -> Result<(), WorkbenchError> {
    let bytes = serde_json::to_vec(manifest)
        .map_err(|error| WorkbenchError::new(format!("cannot encode draft manifest: {error}")))?;
    let target = directory.join(if finalized {
        DRAFT_FINAL_MANIFEST
    } else {
        DRAFT_MANIFEST
    });
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = directory.join(format!(".draft-{nonce}.tmp"));
    let mut file = fs::File::create(&temporary).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot write draft manifest {}: {error}",
            directory.display()
        ))
    })?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| {
            WorkbenchError::new(format!("cannot flush {}: {error}", temporary.display()))
        })?;
    match fs::rename(&temporary, &target) {
        Ok(()) => Ok(()),
        Err(_) if target.is_file() => {
            let _ = fs::remove_file(&temporary);
            Ok(())
        }
        Err(error) => Err(WorkbenchError::new(format!(
            "cannot atomically install {}: {error}",
            target.display()
        ))),
    }
}

fn write_draft_launch(directory: &Path, launch: &DraftLaunch) -> Result<(), WorkbenchError> {
    let bytes = serde_json::to_vec(launch)
        .map_err(|error| WorkbenchError::new(format!("cannot encode draft launch: {error}")))?;
    let target = directory.join(DRAFT_LAUNCH);
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = directory.join(format!(".launch-{nonce}.tmp"));
    let mut file = fs::File::create(&temporary)
        .map_err(|error| WorkbenchError::new(format!("cannot create launch state: {error}")))?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| WorkbenchError::new(format!("cannot flush launch state: {error}")))?;
    fs::rename(&temporary, &target)
        .map_err(|error| WorkbenchError::new(format!("cannot install launch state: {error}")))
}

fn random_session_token() -> Result<String, WorkbenchError> {
    let mut bytes = [0_u8; 16];
    fill_random(&mut bytes)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[cfg(windows)]
fn fill_random(output: &mut [u8]) -> Result<(), WorkbenchError> {
    #[link(name = "bcrypt")]
    unsafe extern "system" {
        fn BCryptGenRandom(
            algorithm: *mut std::ffi::c_void,
            buffer: *mut u8,
            length: u32,
            flags: u32,
        ) -> i32;
    }
    const BCRYPT_USE_SYSTEM_PREFERRED_RNG: u32 = 2;
    // SAFETY: `output` is a live writable slice and its length fits in u32.
    let status = unsafe {
        BCryptGenRandom(
            std::ptr::null_mut(),
            output.as_mut_ptr(),
            output.len() as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    if status == 0 {
        Ok(())
    } else {
        Err(WorkbenchError::new(format!(
            "system random generator failed with status {status}"
        )))
    }
}

#[cfg(not(windows))]
fn fill_random(output: &mut [u8]) -> Result<(), WorkbenchError> {
    fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(output))
        .map_err(|error| WorkbenchError::new(format!("system random generator failed: {error}")))
}

fn tape_digest(tape: &InputTape) -> Result<String, WorkbenchError> {
    let encoded = tape
        .encode()
        .map_err(|error| WorkbenchError::new(error.to_string()))?;
    Ok(format!("{:x}", Sha256::digest(encoded)))
}

fn read_draft_tape(directory: &Path) -> Result<(Vec<u8>, InputTape), WorkbenchError> {
    let directory = fs::canonicalize(directory)
        .map_err(|error| WorkbenchError::new(format!("cannot resolve draft directory: {error}")))?;
    let path = fs::canonicalize(directory.join(DRAFT_TAPE))
        .map_err(|error| WorkbenchError::new(format!("cannot resolve draft tape: {error}")))?;
    if !path.starts_with(&directory) || !path.is_file() {
        return Err(WorkbenchError::new(
            "draft tape escapes its draft directory",
        ));
    }
    let bytes = fs::read(&path).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot read draft tape {}: {error}",
            path.display()
        ))
    })?;
    let decoded = InputTape::decode(&bytes).map_err(|error| {
        WorkbenchError::new(format!("invalid draft tape {}: {error}", path.display()))
    })?;
    Ok((bytes, decoded.tape))
}

/// Resolve a pinned continuation and concatenate its immutable segment artifacts. A frame
/// selector is inclusive: frame N's input is present in the output tape.
pub fn materialize_lineage(
    timeline: &Timeline,
    repository_root: &Path,
    lineage_name: &str,
    target: MaterializeTarget,
) -> Result<MaterializedLineage, WorkbenchError> {
    let inspection = timeline
        .inspect()
        .map_err(|error| WorkbenchError::new(error.to_string()))?;
    let lineage = inspection
        .lineages
        .iter()
        .find(|lineage| lineage.name == lineage_name)
        .ok_or_else(|| WorkbenchError::new(format!("unknown lineage {lineage_name:?}")))?;
    let selected = selected_step_count(timeline, lineage, &target)?;
    if selected == 0 {
        return Ok(MaterializedLineage {
            lineage: lineage_name.into(),
            tape: InputTape::default(),
            steps: Vec::new(),
        });
    }
    ensure_composable_lineage(timeline, lineage, selected)?;

    let mut chain = Vec::with_capacity(selected);
    for (index, step) in lineage.steps.iter().take(selected).enumerate() {
        let segment = &timeline.segments[&step.segment];
        let tape = load_segment_tape(segment, repository_root)?;
        let logical_last = logical_last_frame(segment, &tape)?;
        let frame_window = match &target {
            MaterializeTarget::ThroughSegmentFrame { segment, frame }
                if index + 1 == selected && step.segment == *segment =>
            {
                if *frame > logical_last {
                    return Err(WorkbenchError::new(format!(
                        "frame {frame} is outside segment {segment:?} (last logical frame is {logical_last})"
                    )));
                }
                SegmentFrames::ThroughMilestone { tape_frame: *frame }
            }
            _ => SegmentFrames::ThroughMilestone {
                tape_frame: logical_last,
            },
        };
        chain.push(ChainSegment {
            name: Some(segment.id.clone()),
            tape,
            markers: Vec::new(),
            frames: frame_window,
        });
    }
    let chained = concatenate(chain).map_err(|error| WorkbenchError::new(error.to_string()))?;
    let steps = chained
        .segments
        .iter()
        .map(|boundary| {
            let segment_id = boundary
                .segment_name
                .as_ref()
                .expect("workbench always names chain segments");
            MaterializedStep {
                segment: segment_id.clone(),
                source_start_frame: boundary.source_start_tick,
                source_end_frame: boundary.source_end_tick,
                chain_start_frame: boundary.chain_start_tick,
                chain_end_frame: boundary.chain_end_tick,
            }
        })
        .collect();
    Ok(MaterializedLineage {
        lineage: lineage_name.into(),
        tape: chained.tape,
        steps,
    })
}

/// Materialize the unique Boot-rooted ancestry of a segment. Named
/// continuations are bookmarks, not playback authorization: the parent links
/// and their exact fingerprints are the structural authority.
pub fn materialize_segment_chain(
    timeline: &Timeline,
    repository_root: &Path,
    segment_id: &str,
) -> Result<MaterializedLineage, WorkbenchError> {
    let mut reversed = Vec::new();
    let mut cursor = segment_id;
    let mut seen = BTreeSet::new();
    loop {
        if !seen.insert(cursor.to_owned()) {
            return Err(WorkbenchError::new(format!(
                "segment ancestry contains a cycle at {cursor:?}"
            )));
        }
        let segment = timeline
            .segments
            .get(cursor)
            .ok_or_else(|| WorkbenchError::new(format!("unknown segment {cursor:?}")))?;
        reversed.push(segment);
        let Some(parent_id) = segment.parent.as_deref() else {
            break;
        };
        let parent = timeline.segments.get(parent_id).ok_or_else(|| {
            WorkbenchError::new(format!(
                "segment {:?} references missing parent {parent_id:?}",
                segment.id
            ))
        })?;
        if parent.end_fingerprint != segment.start_fingerprint {
            return Err(WorkbenchError::new(format!(
                "segment {:?} starts at {}, but parent {parent_id:?} ends at {}",
                segment.id, segment.start_fingerprint, parent.end_fingerprint
            )));
        }
        cursor = parent_id;
    }
    reversed.reverse();

    let mut chain = Vec::with_capacity(reversed.len());
    for segment in reversed {
        if !artifact_is_canonical_payload(&segment.artifact) {
            return Err(WorkbenchError::new(format!(
                "segment {} is a stage-seeded baseline/candidate, not a canonical continuation tape",
                segment.id
            )));
        }
        if !fingerprints_are_exact(segment) {
            return Err(WorkbenchError::new(format!(
                "segment {} uses placeholder fingerprints",
                segment.id
            )));
        }
        let tape = load_segment_tape(segment, repository_root)?;
        let logical_last = logical_last_frame(segment, &tape)?;
        chain.push(ChainSegment {
            name: Some(segment.id.clone()),
            tape,
            markers: Vec::new(),
            frames: SegmentFrames::ThroughMilestone {
                tape_frame: logical_last,
            },
        });
    }
    let chained = concatenate(chain).map_err(|error| WorkbenchError::new(error.to_string()))?;
    let steps = chained
        .segments
        .iter()
        .map(|boundary| MaterializedStep {
            segment: boundary
                .segment_name
                .clone()
                .expect("workbench always names chain segments"),
            source_start_frame: boundary.source_start_tick,
            source_end_frame: boundary.source_end_tick,
            chain_start_frame: boundary.chain_start_tick,
            chain_end_frame: boundary.chain_end_tick,
        })
        .collect();
    Ok(MaterializedLineage {
        lineage: segment_id.to_owned(),
        tape: chained.tape,
        steps,
    })
}

/// Materialize and launch a headful fixed-step process. No shell is involved;
/// all paths become individual process arguments.
pub fn play(
    timeline: &Timeline,
    config: &WorkbenchConfig,
    request: &PlayRequest,
) -> Result<(PlayResponse, Child), WorkbenchError> {
    validate_play_request(request)?;
    let artifact_root = configured_artifact_root(config)?;
    let materialized = materialize_play_request(timeline, &artifact_root, request)?;
    launch_materialized(
        timeline,
        config,
        materialized,
        MaterializedLaunchOptions {
            takeover: request.takeover,
            origin: PlaybackOrigin::Boot,
            fast_forward_frames: None,
            thumbnail: None,
            playback: PlaybackSettings {
                speed_percent: 100,
                fast: false,
            },
        },
    )
}

fn launch_materialized(
    timeline: &Timeline,
    config: &WorkbenchConfig,
    materialized: MaterializedPlayback,
    options: MaterializedLaunchOptions,
) -> Result<(PlayResponse, Child), WorkbenchError> {
    let game = canonical_file(&config.game, "game executable")?;
    let dvd = canonical_file(&config.dvd, "DVD image")?;
    fs::create_dir_all(&config.state_root).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot create state root {}: {error}",
            config.state_root.display()
        ))
    })?;
    let state_parent = fs::canonicalize(&config.state_root).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot resolve state root {}: {error}",
            config.state_root.display()
        ))
    })?;
    let renderer_cache_root = state_parent.join("renderer-cache");
    fs::create_dir_all(&renderer_cache_root).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot create renderer cache {}: {error}",
            renderer_cache_root.display()
        ))
    })?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let session_id = format!("{}-{nonce}", std::process::id());
    let state_root = state_parent.join(format!("session-{session_id}"));
    fs::create_dir(&state_root).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot create fresh session {}: {error}",
            state_root.display()
        ))
    })?;
    let tape_path = state_root.join("playback.tape");
    let encoded = materialized
        .tape
        .encode()
        .map_err(|error| WorkbenchError::new(error.to_string()))?;
    fs::write(&tape_path, encoded).map_err(|error| {
        WorkbenchError::new(format!("cannot write {}: {error}", tape_path.display()))
    })?;
    let end = if options.takeover { "release" } else { "hold" };
    let mut command = Command::new(game);
    command.current_dir(&config.working_directory);
    append_playback_args(
        &mut command,
        &dvd,
        &tape_path,
        end,
        &state_root,
        PlaybackCliOptions {
            seed_stage: materialized.seed_stage,
            fast_forward_frames: options.fast_forward_frames,
            playback: options.playback,
        },
    );
    if let Some(thumbnail) = &options.thumbnail {
        command
            .arg("--input-tape-thumbnail-png")
            .arg(&thumbnail.path);
    }
    let artifact_root = configured_artifact_root(config)?;
    append_authored_milestone_args(timeline, &artifact_root, &state_root, &mut command, None)?;
    let child = command
        .spawn()
        .map_err(|error| WorkbenchError::new(format!("cannot launch Dusklight: {error}")))?;
    let response = PlayResponse {
        pid: child.id(),
        lineage: materialized.lineage,
        segment: materialized.segment,
        tape: tape_path,
        session_state_root: state_root,
        session_id,
        frames: materialized.tape.frames.len() as u64,
        input_tape_end: end.into(),
        origin: options.origin,
        speed_percent: options.playback.speed_percent,
        fast: options.playback.fast,
        fast_forward_frames: options.fast_forward_frames,
        thumbnail: options.thumbnail.map(|thumbnail| thumbnail.url),
    };
    Ok((response, child))
}

fn capture_thumbnail(
    timeline: &Timeline,
    config: &WorkbenchConfig,
    request: &BrowserThumbnailCaptureRequest,
) -> Result<(ThumbnailCaptureResponse, Child), WorkbenchError> {
    let game = canonical_file(&config.game, "game executable")?;
    let dvd = canonical_file(&config.dvd, "DVD image")?;
    let artifact_root = configured_artifact_root(config)?;
    let graph = graph_with_drafts(timeline, &artifact_root, &config.state_root)?;
    let key = graph_node_thumbnail_key(&graph, &request.selection)?;
    let materialized = match &request.selection {
        BrowserSelection::Segment { id } => {
            materialize_segment_playback(timeline, &artifact_root, id, None)?
        }
        BrowserSelection::Draft { id } => {
            materialize_draft(timeline, &artifact_root, &config.state_root, id)?
        }
    };

    fs::create_dir_all(&config.state_root).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot create state root {}: {error}",
            config.state_root.display()
        ))
    })?;
    let state_parent = fs::canonicalize(&config.state_root).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot resolve state root {}: {error}",
            config.state_root.display()
        ))
    })?;
    let thumbnail_root = state_parent.join(THUMBNAIL_DIRECTORY);
    fs::create_dir_all(&thumbnail_root).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot create thumbnail cache {}: {error}",
            thumbnail_root.display()
        ))
    })?;
    let thumbnail_root = fs::canonicalize(&thumbnail_root).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot resolve thumbnail cache {}: {error}",
            thumbnail_root.display()
        ))
    })?;
    if thumbnail_root.parent() != Some(state_parent.as_path()) {
        return Err(WorkbenchError::new(
            "thumbnail cache escapes the workbench state root",
        ));
    }
    let thumbnail_path = thumbnail_root.join(format!("{key}.png"));
    let renderer_cache_root = state_parent.join("renderer-cache");
    fs::create_dir_all(&renderer_cache_root).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot create renderer cache {}: {error}",
            renderer_cache_root.display()
        ))
    })?;

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let session_root =
        state_parent.join(format!("thumbnail-session-{}-{nonce}", std::process::id()));
    fs::create_dir(&session_root).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot create fresh thumbnail session {}: {error}",
            session_root.display()
        ))
    })?;
    let tape_path = session_root.join("playback.tape");
    let encoded = materialized
        .tape
        .encode()
        .map_err(|error| WorkbenchError::new(error.to_string()))?;
    fs::write(&tape_path, encoded).map_err(|error| {
        WorkbenchError::new(format!("cannot write {}: {error}", tape_path.display()))
    })?;

    let mut command = Command::new(game);
    command.current_dir(&config.working_directory);
    append_playback_args(
        &mut command,
        &dvd,
        &tape_path,
        "release",
        &session_root,
        PlaybackCliOptions {
            seed_stage: materialized.seed_stage,
            fast_forward_frames: None,
            playback: PlaybackSettings {
                speed_percent: 100,
                fast: false,
            },
        },
    );
    command
        .arg("--unpaced")
        .arg("--exit-after-tape")
        .arg("--frame-capture-png")
        .arg(&thumbnail_path)
        .arg("--frame-capture-width")
        .arg(THUMBNAIL_WIDTH.to_string())
        .arg("--frame-capture-height")
        .arg(THUMBNAIL_HEIGHT.to_string());
    append_authored_milestone_args(timeline, &artifact_root, &session_root, &mut command, None)?;
    let child = command.spawn().map_err(|error| {
        WorkbenchError::new(format!("cannot launch thumbnail capture: {error}"))
    })?;
    let response = ThumbnailCaptureResponse {
        schema: THUMBNAIL_CAPTURE_SCHEMA.into(),
        pid: child.id(),
        key: key.clone(),
        thumbnail: thumbnail_url(&key),
        frames: materialized.tape.frames.len() as u64,
    };
    Ok((response, child))
}

fn play_draft(
    timeline: &Timeline,
    config: &WorkbenchConfig,
    draft_id: &str,
    origin: PlaybackOrigin,
    speed_percent: u16,
    fast: bool,
) -> Result<(PlayResponse, Child), WorkbenchError> {
    let artifact_root = configured_artifact_root(config)?;
    let materialized = materialize_draft(timeline, &artifact_root, &config.state_root, draft_id)?;
    let fast_forward_frames = match origin {
        PlaybackOrigin::Boot => None,
        PlaybackOrigin::Parent => {
            draft_parent_frame_count(
                timeline,
                &artifact_root,
                &config.state_root,
                draft_id,
                materialized.tape.frames.len() as u64,
            )?;
            fast.then_some(materialized.tape.frames.len() as u64)
        }
    };
    let thumbnail = prepare_missing_playback_thumbnail(
        timeline,
        config,
        &BrowserSelection::Draft {
            id: draft_id.into(),
        },
    )?;
    launch_materialized(
        timeline,
        config,
        materialized,
        MaterializedLaunchOptions {
            takeover: true,
            origin,
            fast_forward_frames,
            thumbnail,
            playback: PlaybackSettings {
                speed_percent: if fast { 0 } else { speed_percent },
                fast,
            },
        },
    )
}

fn draft_parent_frame_count(
    timeline: &Timeline,
    repository_root: &Path,
    state_root: &Path,
    draft_id: &str,
    full_frames: u64,
) -> Result<u64, WorkbenchError> {
    let manifests = scan_draft_manifests(state_root)?;
    let manifest = manifests
        .get(draft_id)
        .ok_or_else(|| WorkbenchError::new(format!("unknown draft {draft_id:?}")))?;
    if manifest.status != DraftStatus::Ready {
        return Err(WorkbenchError::new(format!(
            "draft {draft_id:?} is not ready"
        )));
    }
    let draft_directory = drafts_root(state_root)?.join(draft_id);
    let (_, continuation) = read_draft_tape(&draft_directory)?;
    let continuation_frames = continuation.frames.len() as u64;
    let parent = match &manifest.parent {
        DraftParent::Milestone { .. } => InputTape::default(),
        DraftParent::Segment { id, .. } => {
            materialize_segment_chain(timeline, repository_root, id)?.tape
        }
        DraftParent::Draft { id, .. } => {
            materialize_draft(timeline, repository_root, state_root, id)?.tape
        }
    };
    let parent_frames = parent.frames.len() as u64;
    if tape_digest(&parent)? != manifest.parent_tape_sha256 {
        return Err(WorkbenchError::new(format!(
            "draft {draft_id:?} direct-parent tape fingerprint changed"
        )));
    }
    validate_parent_boundary_metadata(
        parent_frames,
        continuation_frames,
        manifest.parent_frames,
        manifest.frames,
        full_frames,
    )
    .map_err(|_| {
        WorkbenchError::new(format!(
            "draft {draft_id:?} has invalid direct-parent playback boundary metadata"
        ))
    })?;
    Ok(parent_frames)
}

fn validate_parent_boundary_metadata(
    actual_parent_frames: u64,
    actual_continuation_frames: u64,
    declared_parent_frames: u64,
    declared_continuation_frames: Option<u64>,
    full_frames: u64,
) -> Result<(), ()> {
    if declared_parent_frames != actual_parent_frames
        || declared_continuation_frames != Some(actual_continuation_frames)
    {
        return Err(());
    }
    validate_parent_boundary(
        actual_parent_frames,
        actual_continuation_frames,
        full_frames,
    )
}

fn validate_parent_boundary(
    parent_frames: u64,
    continuation_frames: u64,
    full_frames: u64,
) -> Result<(), ()> {
    if parent_frames == 0
        || parent_frames >= full_frames
        || continuation_frames == 0
        || parent_frames.checked_add(continuation_frames) != Some(full_frames)
    {
        Err(())
    } else {
        Ok(())
    }
}

fn append_authored_milestone_args(
    timeline: &Timeline,
    artifact_root: &Path,
    state_root: &Path,
    command: &mut Command,
    additional_builtin: Option<&str>,
) -> Result<(), WorkbenchError> {
    let Some(source_path) = validated_milestone_program_path(timeline, artifact_root)? else {
        return Ok(());
    };
    let source = fs::read_to_string(&source_path).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot read configured milestone program {}: {error}",
            source_path.display()
        ))
    })?;
    let (_, compiled) = validate_milestone_program_source(timeline, &source)?;
    let program_path = state_root.join("route-milestones.dmsp");
    let result_path = state_root.join("route-milestones.json");
    fs::write(&program_path, &compiled.bytes).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot write compiled milestone program {}: {error}",
            program_path.display()
        ))
    })?;
    let mut requested = timeline
        .origin
        .iter()
        .map(|origin| origin.predicate.clone())
        .collect::<Vec<_>>();
    for predicate in timeline.goals.values().map(|goal| goal.predicate.clone()) {
        if !requested.contains(&predicate) {
            requested.push(predicate);
        }
    }
    if let Some(id) = additional_builtin
        && !requested.iter().any(|existing| existing == id)
    {
        requested.push(id.to_owned());
    }
    command
        .arg("--milestone-program")
        .arg(program_path)
        .arg("--milestones")
        .arg(requested.join(","))
        .arg("--milestone-result")
        .arg(result_path);
    Ok(())
}

fn append_playback_args(
    command: &mut Command,
    dvd: &Path,
    tape: &Path,
    end: &str,
    state_root: &Path,
    options: PlaybackCliOptions<'_>,
) {
    let renderer_cache_root = state_root
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join("renderer-cache");
    command
        .arg("--dvd")
        .arg(dvd)
        .arg("--input-tape")
        .arg(tape)
        .arg("--input-tape-end")
        .arg(end);
    if let Some(frames) = options.fast_forward_frames {
        command
            .arg("--input-tape-fast-forward-frames")
            .arg(frames.to_string());
    }
    command
        .arg("--automation-data-root")
        .arg(state_root)
        .arg("--renderer-cache-root")
        .arg(&renderer_cache_root)
        .arg("--cvar")
        .arg("game.instantSaves=true")
        .arg("--cvar")
        .arg("backend.cardFileType=1")
        .arg("--cvar")
        .arg("backend.wasPresetChosen=true")
        .arg("--cvar")
        .arg("game.enableMenuPointer=false");
    append_fixed_step_pacing(command, options.playback.speed_percent);
    if let Some(stage) = options.seed_stage {
        command.arg("--stage").arg(stage);
    }
}

fn append_fixed_step_pacing(command: &mut Command, speed_percent: u16) {
    command
        .arg("--fixed-step")
        .arg("--fixed-step-speed-percent")
        .arg(speed_percent.to_string());
}

fn validate_draft_label(label: &str) -> Result<String, WorkbenchError> {
    let label = label.trim();
    if label.is_empty() || label.len() > 160 || label.chars().any(char::is_control) {
        return Err(WorkbenchError::new(
            "draft label must be 1 to 160 UTF-8 bytes without controls",
        ));
    }
    Ok(label.to_owned())
}

fn append_accelerated_recording_prefix(
    command: &mut Command,
    playback: &Path,
    parent_frames: usize,
    countdown_seconds: u8,
) {
    command
        .arg("--input-tape")
        .arg(playback)
        .arg("--input-tape-end")
        .arg("release")
        .arg("--input-tape-fast-forward-frames")
        .arg(parent_frames.to_string())
        .arg("--record-input-countdown-seconds")
        .arg(countdown_seconds.to_string());
}

fn record_continuation(
    timeline: &Timeline,
    config: &WorkbenchConfig,
    request: BrowserRecordRequest,
) -> Result<RecordResponse, WorkbenchError> {
    let artifact_root = configured_artifact_root(config)?;
    let game = canonical_file(&config.game, "game executable")?;
    let dvd = canonical_file(&config.dvd, "DVD image")?;
    let existing = scan_draft_manifests(&config.state_root)?;
    let generated_number = existing.len() + 1;
    let label = if request.label.trim().is_empty() {
        format!("Manual branch {generated_number}")
    } else {
        validate_draft_label(&request.label)?
    };
    let (
        materialized,
        parent,
        expected_start_milestone,
        expected_start_fingerprint,
        record_from_boot,
    ) = match request.parent {
        BrowserRecordParent::Origin { id } => {
            let graph = graph_from_timeline(timeline, &artifact_root)?;
            let origin = graph
                .origin
                .as_ref()
                .filter(|origin| origin.id == id)
                .ok_or_else(|| WorkbenchError::new(format!("unknown origin {id:?}")))?;
            if !origin.recordable_from_boot {
                return Err(WorkbenchError::new(format!(
                    "origin {id:?} is not the exact authored Boot boundary"
                )));
            }
            let program = graph
                .predicate_program
                .ok_or_else(|| WorkbenchError::new("Boot recording requires milestone source"))?;
            let definition = program
                .definitions
                .iter()
                .find(|definition| definition.name == origin.predicate)
                .expect("graph origin predicate definition must exist");
            (
                MaterializedPlayback {
                    lineage: None,
                    segment: Some(format!("origin:{id}")),
                    tape: InputTape::default(),
                    seed_stage: None,
                },
                DraftParent::Milestone {
                    id: id.clone(),
                    program_sha256: program.program_sha256,
                    definition_sha256: definition.definition_sha256.clone(),
                    boundary_fingerprint: None,
                },
                Some(origin.predicate.clone()),
                None,
                true,
            )
        }
        BrowserRecordParent::Segment { id, terminal_goal } => {
            let segment = timeline
                .segments
                .get(&id)
                .ok_or_else(|| WorkbenchError::new(format!("unknown segment {id:?}")))?;
            let anchors = graph_from_timeline(timeline, &artifact_root)?
                .segments
                .into_iter()
                .find(|candidate| candidate.id == id)
                .expect("timeline segment must appear in its graph")
                .record_anchors;
            let anchor = anchors.iter().find(|anchor| anchor.goal == terminal_goal);
            if anchor.is_none() || !native_fingerprint(&segment.end_fingerprint) {
                return Err(WorkbenchError::new(
                    "recording requires a verified goal attached to the selected segment",
                ));
            }
            let segment_chain = materialize_segment_chain(timeline, &artifact_root, &id)?;
            let seed_stage = segment_chain.steps.first().and_then(|step| {
                legacy_seed_stage(
                    &segment_chain.tape,
                    timeline.segments[&step.segment].profile,
                )
            });
            let materialized = MaterializedPlayback {
                lineage: None,
                segment: Some(id.clone()),
                tape: segment_chain.tape,
                seed_stage,
            };
            let parent = DraftParent::Segment {
                id: id.clone(),
                terminal_milestone: anchor.expect("checked anchor").predicate.clone(),
                boundary_fingerprint: segment.end_fingerprint.clone(),
            };
            (
                materialized,
                parent,
                Some(anchor.expect("checked anchor").predicate.clone()),
                Some(segment.end_fingerprint.clone()),
                false,
            )
        }
        BrowserRecordParent::Draft { id } => {
            let materialized =
                materialize_draft(timeline, &artifact_root, &config.state_root, &id)?;
            let digest = tape_digest(&materialized.tape)?;
            (
                materialized,
                DraftParent::Draft {
                    id,
                    parent_tape_sha256: digest,
                },
                None,
                None,
                false,
            )
        }
    };
    let parent_tape_sha256 = tape_digest(&materialized.tape)?;
    let root = drafts_root(&config.state_root)?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let draft_id = format!("draft-{}-{nonce}", std::process::id());
    let directory = root.join(&draft_id);
    fs::create_dir(&directory).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot create draft {}: {error}",
            directory.display()
        ))
    })?;
    let playback = directory.join("playback.tape");
    fs::write(
        &playback,
        materialized
            .tape
            .encode()
            .map_err(|error| WorkbenchError::new(error.to_string()))?,
    )
    .map_err(|error| WorkbenchError::new(format!("cannot write playback prefix: {error}")))?;
    let continuation = directory.join(DRAFT_TAPE);
    let state = directory.join("state");
    fs::create_dir(&state).map_err(|error| WorkbenchError::new(error.to_string()))?;
    let renderer_cache_root = config.state_root.join("renderer-cache");
    fs::create_dir_all(&renderer_cache_root).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot create renderer cache {}: {error}",
            renderer_cache_root.display()
        ))
    })?;
    let created_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let session_token = random_session_token()?;
    let manifest = DraftManifest {
        schema: DRAFT_SCHEMA.into(),
        id: draft_id.clone(),
        label,
        parent,
        parent_tape_sha256,
        created_unix_ms,
        session_token: session_token.clone(),
        expected_start_milestone: expected_start_milestone.clone(),
        expected_start_fingerprint: expected_start_fingerprint.clone(),
        tape: DRAFT_TAPE.into(),
        status: DraftStatus::Preparing,
        endpoint_kind: "manual_stop".into(),
        verification: "unverified".into(),
        start_boundary_verified: false,
        accelerated_parent_replay: !record_from_boot,
        parent_frames: materialized.tape.frames.len() as u64,
        tape_sha256: None,
        tape_bytes: None,
        result_tape_sha256: None,
        frames: None,
        error: None,
    };
    write_draft_manifest(&directory, &manifest, false)?;
    let mut command = Command::new(game);
    command
        .current_dir(&config.working_directory)
        .arg("--dvd")
        .arg(dvd);
    if record_from_boot {
        command.arg("--record-input-from-boot");
    } else {
        append_accelerated_recording_prefix(
            &mut command,
            &playback,
            materialized.tape.frames.len(),
            request.countdown_seconds,
        );
    }
    command
        .arg("--record-input-tape")
        .arg(&continuation)
        .arg("--record-input-thumbnail-png")
        .arg(directory.join(DRAFT_TERMINAL_THUMBNAIL))
        .arg("--record-input-capacity")
        .arg("1080000")
        .arg("--record-input-session")
        .arg(&session_token)
        .arg("--automation-data-root")
        .arg(&state)
        .arg("--renderer-cache-root")
        .arg(&renderer_cache_root)
        .arg("--cvar")
        .arg("game.instantSaves=true")
        .arg("--cvar")
        .arg("backend.cardFileType=1")
        .arg("--cvar")
        .arg("backend.wasPresetChosen=true")
        .arg("--cvar")
        .arg("game.enableMenuPointer=false");
    append_fixed_step_pacing(&mut command, request.speed_percent);
    if record_from_boot {
        command.arg("--record-input-start-milestone").arg(
            expected_start_milestone
                .as_deref()
                .expect("Boot recording has an authored start milestone"),
        );
    } else if let (Some(milestone), Some(fingerprint)) =
        (&expected_start_milestone, &expected_start_fingerprint)
    {
        command
            .arg("--record-input-start-milestone")
            .arg(milestone)
            .arg("--record-input-start-fingerprint")
            .arg(fingerprint);
    }
    if let Some(stage) = materialized.seed_stage {
        command.arg("--stage").arg(stage);
    }
    append_authored_milestone_args(
        timeline,
        &artifact_root,
        &state,
        &mut command,
        (!record_from_boot)
            .then_some(expected_start_milestone.as_deref())
            .flatten(),
    )?;
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            let mut failed = manifest;
            failed.status = DraftStatus::ProcessFailure;
            failed.error = Some(format!("cannot launch Dusklight: {error}"));
            let _ = write_draft_manifest(&directory, &failed, true);
            return Err(WorkbenchError::new(format!(
                "cannot launch Dusklight: {error}"
            )));
        }
    };
    let pid = child.id();
    let launch = DraftLaunch {
        schema: "dusklight.route-workbench.launch.v2".into(),
        id: draft_id.clone(),
        pid,
        session_token,
    };
    if let Err(error) = write_draft_launch(&directory, &launch) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    let monitor_directory = directory.clone();
    active_recordings()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(draft_id.clone());
    let monitor_id = draft_id.clone();
    let monitor_config = config.clone();
    thread::spawn(move || {
        monitor_recording(
            child,
            monitor_directory,
            manifest,
            monitor_id,
            monitor_config,
        )
    });
    Ok(RecordResponse {
        pid,
        draft_id,
        manifest: directory.join(DRAFT_MANIFEST),
        tape: continuation,
        frames_before_recording: materialized.tape.frames.len() as u64,
        speed_percent: request.speed_percent,
    })
}

fn monitor_recording(
    mut child: Child,
    directory: PathBuf,
    mut manifest: DraftManifest,
    draft_id: String,
    config: WorkbenchConfig,
) {
    match child.wait() {
        Ok(exit) => finalize_recording(&directory, &mut manifest, Some(exit.success())),
        Err(error) => {
            manifest.status = DraftStatus::ProcessFailure;
            manifest.error = Some(format!("cannot wait for Dusklight: {error}"));
        }
    }
    if let Err(error) = install_recording_thumbnail(&directory, &manifest, &config) {
        eprintln!("Route Workbench: {error}");
    }
    let _ = write_draft_manifest(&directory, &manifest, true);
    active_recordings()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(&draft_id);
}

fn finalize_recording(directory: &Path, manifest: &mut DraftManifest, exit_success: Option<bool>) {
    let status_path = directory.join(format!("{DRAFT_TAPE}.status.json"));
    let native: NativeRecordStatus = match fs::read(&status_path)
        .map_err(|error| error.to_string())
        .and_then(|bytes| serde_json::from_slice(&bytes).map_err(|error| error.to_string()))
    {
        Ok(status) => status,
        Err(error) => {
            manifest.status = DraftStatus::ProcessFailure;
            manifest.error = Some(format!(
                "missing or invalid native recording status: {error}"
            ));
            return;
        }
    };
    if native.schema != "dusklight.input-recording/v2"
        || native.session_token.as_deref() != Some(&manifest.session_token)
        || native.frame_capacity != 1_080_000
        || native.frame_count > native.frame_capacity
    {
        manifest.status = DraftStatus::ProcessFailure;
        manifest.error = Some("native recording status is inconsistent".into());
        return;
    }
    if exit_success.is_some_and(|exit_success| exit_success != native.process_success)
        || (native.process_success && native.status != "success")
    {
        manifest.status = DraftStatus::ProcessFailure;
        manifest.error = Some("native status and observed process exit disagree".into());
        return;
    }
    let boot_parent = match &manifest.parent {
        DraftParent::Milestone {
            id,
            program_sha256,
            definition_sha256,
            ..
        } => Some((id, program_sha256, definition_sha256)),
        DraftParent::Segment { .. } | DraftParent::Draft { .. } => None,
    };
    let expected_boundary_matches = match (
        boot_parent,
        &manifest.expected_start_milestone,
        &manifest.expected_start_fingerprint,
    ) {
        (Some((id, program, definition)), Some(milestone), None) => {
            milestone == id
                && native.start_milestone.as_deref() == Some(id)
                && native
                    .start_fingerprint
                    .as_deref()
                    .is_some_and(native_fingerprint)
                && native.expected_start_fingerprint.is_none()
                && native.start_boundary_kind.as_deref() == Some("boot")
                && native.start_boundary_index == Some(0)
                && native.start_program_digest.as_deref() == Some(program)
                && native.start_definition_digest.as_deref() == Some(definition)
                && native.start_tape_frame.is_none()
        }
        (None, Some(milestone), Some(fingerprint)) => {
            native.start_milestone.as_deref() == Some(milestone)
                && native.start_fingerprint.as_deref() == Some(fingerprint)
                && native.expected_start_fingerprint.as_deref() == Some(fingerprint)
                && (!manifest.accelerated_parent_replay
                    || (native.start_boundary_kind.as_deref() == Some("tick")
                        && native.start_boundary_index == Some(manifest.parent_frames)))
                && native.start_tape_frame == manifest.parent_frames.checked_sub(1)
        }
        (None, None, None) if manifest.accelerated_parent_replay => {
            native.start_milestone.is_none()
                && native.start_fingerprint.is_none()
                && native.expected_start_fingerprint.is_none()
                && native.start_boundary_kind.as_deref() == Some("tick")
                && native.start_boundary_index == Some(manifest.parent_frames)
                && native.start_tape_frame == manifest.parent_frames.checked_sub(1)
        }
        (None, None, None) => {
            native.start_milestone.is_none()
                && native.start_fingerprint.is_none()
                && native.expected_start_fingerprint.is_none()
                && native.start_tape_frame.is_none()
        }
        _ => false,
    };
    manifest.frames = Some(native.frame_count);
    manifest.error = native.error;
    manifest.status = match native.status.as_str() {
        "success"
            if native.process_success
                && native.handoff_reached
                && !native.capacity_exhausted
                && native.frame_count > 0
                && expected_boundary_matches =>
        {
            if capture_tape_metadata(directory, manifest, native.frame_count, false) {
                manifest.start_boundary_verified =
                    manifest.expected_start_milestone.is_some() && expected_boundary_matches;
                if let DraftParent::Milestone {
                    boundary_fingerprint,
                    ..
                } = &mut manifest.parent
                {
                    *boundary_fingerprint = native.start_fingerprint.clone();
                }
                DraftStatus::Ready
            } else {
                DraftStatus::ProcessFailure
            }
        }
        "zero_frames"
            if native.handoff_reached
                && !native.capacity_exhausted
                && native.frame_count == 0
                && expected_boundary_matches =>
        {
            if capture_tape_metadata(directory, manifest, 0, true) {
                DraftStatus::ZeroFrames
            } else {
                DraftStatus::ProcessFailure
            }
        }
        "never_reached_handoff" if !native.handoff_reached && native.frame_count == 0 => {
            DraftStatus::NeverReachedHandoff
        }
        "capacity_exhausted"
            if native.handoff_reached
                && native.capacity_exhausted
                && native.frame_count == native.frame_capacity
                && expected_boundary_matches =>
        {
            if capture_tape_metadata(directory, manifest, native.frame_count, false) {
                DraftStatus::CapacityExhausted
            } else {
                DraftStatus::ProcessFailure
            }
        }
        "write_failure" => DraftStatus::WriteFailure,
        "start_boundary_mismatch" => DraftStatus::StartBoundaryMismatch,
        _ => {
            manifest.error = Some("native recording status contradicts process result".into());
            DraftStatus::ProcessFailure
        }
    };
}

fn capture_tape_metadata(
    directory: &Path,
    manifest: &mut DraftManifest,
    expected_frames: u64,
    allow_empty: bool,
) -> bool {
    let expected = directory.join(DRAFT_TAPE);
    let status_tape_matches = fs::canonicalize(&expected)
        .ok()
        .zip(
            fs::read(directory.join(format!("{DRAFT_TAPE}.status.json")))
                .ok()
                .and_then(|bytes| serde_json::from_slice::<NativeRecordStatus>(&bytes).ok())
                .and_then(|status| fs::canonicalize(status.tape).ok()),
        )
        .is_some_and(|(left, right)| left == right);
    let result = read_draft_tape(directory);
    match result {
        Ok((bytes, tape))
            if status_tape_matches
                && tape.frames.len() as u64 == expected_frames
                && (allow_empty || !tape.frames.is_empty()) =>
        {
            manifest.tape_sha256 = Some(format!("{:x}", Sha256::digest(&bytes)));
            manifest.tape_bytes = Some(bytes.len() as u64);
            manifest.result_tape_sha256 = fs::read(directory.join("playback.tape"))
                .ok()
                .and_then(|prefix| InputTape::decode(&prefix).ok())
                .and_then(|prefix| {
                    concatenate(vec![
                        ChainSegment::all(prefix.tape),
                        ChainSegment::all(tape),
                    ])
                    .ok()
                })
                .and_then(|chain| chain.tape.encode().ok())
                .map(|result| format!("{:x}", Sha256::digest(result)));
            if manifest.result_tape_sha256.is_none() {
                manifest.error = Some("cannot fingerprint finalized draft chain".into());
                return false;
            }
            true
        }
        _ => {
            manifest.error = Some("native recording tape is missing or inconsistent".into());
            false
        }
    }
}

/// Serve the graph and playback API. The listener must be loopback-only: the
/// play endpoint can start a user-selected executable and must not be exposed.
fn graph_artifact(source: &ArtifactSource) -> GraphArtifact {
    match source {
        ArtifactSource::Baseline(profile) => GraphArtifact {
            kind: "baseline".into(),
            value: profile.as_str().into(),
        },
        ArtifactSource::Candidate(path) => GraphArtifact {
            kind: "candidate".into(),
            value: path.display().to_string(),
        },
        ArtifactSource::Tas(path) => GraphArtifact {
            kind: "tas".into(),
            value: path.display().to_string(),
        },
        ArtifactSource::Tape(path) => GraphArtifact {
            kind: "tape".into(),
            value: path.display().to_string(),
        },
        #[allow(unreachable_patterns)]
        _ => GraphArtifact {
            kind: "unsupported".into(),
            value: "artifact source is not supported by this workbench build".into(),
        },
    }
}

fn selected_step_count(
    timeline: &Timeline,
    lineage: &ResolvedLineage,
    target: &MaterializeTarget,
) -> Result<usize, WorkbenchError> {
    match target {
        MaterializeTarget::FullLineage => Ok(lineage.steps.len()),
        MaterializeTarget::ThroughSegment(segment) => {
            unique_segment_step(timeline, lineage, segment)
        }
        MaterializeTarget::ThroughSegmentFrame { segment, .. } => {
            unique_segment_step(timeline, lineage, segment)
        }
        MaterializeTarget::ThroughStepCount(count) => {
            if *count <= lineage.steps.len() {
                Ok(*count)
            } else {
                Err(WorkbenchError::new(format!(
                    "step count {count} is outside lineage {:?} ({} steps)",
                    lineage.name,
                    lineage.steps.len()
                )))
            }
        }
    }
}

fn unique_segment_step(
    _timeline: &Timeline,
    lineage: &ResolvedLineage,
    segment: &str,
) -> Result<usize, WorkbenchError> {
    let matches = lineage
        .steps
        .iter()
        .enumerate()
        .filter(|(_, step)| step.segment == segment)
        .map(|(index, _)| index + 1)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [step] => Ok(*step),
        [] => Err(WorkbenchError::new(format!(
            "segment {segment:?} is not on lineage {:?}",
            lineage.name
        ))),
        _ => Err(WorkbenchError::new(format!(
            "segment {segment:?} occurs more than once on lineage {:?}; use an exact step count",
            lineage.name
        ))),
    }
}

fn ensure_composable_lineage(
    timeline: &Timeline,
    lineage: &ResolvedLineage,
    selected: usize,
) -> Result<(), WorkbenchError> {
    // A single segment can always be played under its profile seed. Crossing a
    // boundary is stricter: generated baselines/candidates may contain their
    // evaluation seed harness and therefore are not continuation payloads.
    if selected <= 1 {
        return Ok(());
    }
    ensure_canonical_prefix(timeline, lineage, selected)
}

fn ensure_canonical_prefix(
    timeline: &Timeline,
    lineage: &ResolvedLineage,
    selected: usize,
) -> Result<(), WorkbenchError> {
    for step in lineage.steps.iter().take(selected) {
        let segment = &timeline.segments[&step.segment];
        if !artifact_is_canonical_payload(&segment.artifact) {
            return Err(WorkbenchError::new(format!(
                "continuation {:?} cannot cross segment {}: it is a stage-seeded baseline/candidate, not a canonical continuation tape",
                lineage.name, segment.id
            )));
        }
        if !fingerprints_are_exact(segment)
            || contains_placeholder(&step.after.checkpoint_fingerprint)
        {
            return Err(WorkbenchError::new(format!(
                "continuation {:?} cannot cross segment {} because it uses placeholder fingerprints",
                lineage.name, segment.id
            )));
        }
    }
    Ok(())
}

fn artifact_is_canonical_payload(source: &ArtifactSource) -> bool {
    // `uses tape` is the current DSL's explicit compact, immutable payload.
    // Baseline and candidate sources are profile-seeded evaluation programs.
    matches!(source, ArtifactSource::Tas(_) | ArtifactSource::Tape(_))
}

fn fingerprints_are_exact(segment: &Segment) -> bool {
    !contains_placeholder(&segment.start_fingerprint)
        && !contains_placeholder(&segment.end_fingerprint)
}

fn contains_placeholder(value: &str) -> bool {
    value.trim().is_empty() || value.to_ascii_lowercase().contains("unknown")
}

fn logical_last_frame(segment: &Segment, tape: &InputTape) -> Result<u64, WorkbenchError> {
    if tape.frames.is_empty() {
        return Err(WorkbenchError::new(format!(
            "segment {} has an empty tape",
            segment.id
        )));
    }
    // first_hit_tick is a simulation score, not an artifact-local frame
    // boundary. Until canonical bundles carry an explicit tape_frame, the
    // complete artifact is the only safe payload boundary.
    Ok(tape.frames.len() as u64 - 1)
}

fn option_diagnostic_relative_path(source: &ArtifactSource) -> Option<PathBuf> {
    let artifact = match source {
        ArtifactSource::Candidate(path)
        | ArtifactSource::Tas(path)
        | ArtifactSource::Tape(path) => path,
        ArtifactSource::Baseline(_) => return None,
        #[allow(unreachable_patterns)]
        _ => return None,
    };
    let mut sidecar = artifact.as_os_str().to_os_string();
    sidecar.push(".options.json");
    Some(PathBuf::from(sidecar))
}

fn load_option_visualization(
    segment: &Segment,
    repository_root: &Path,
    tape: &InputTape,
) -> Result<Vec<OptionVisualization>, WorkbenchError> {
    let Some(relative) = option_diagnostic_relative_path(&segment.artifact) else {
        return Ok(Vec::new());
    };
    let unresolved = repository_root.join(&relative);
    let Ok(metadata) = fs::symlink_metadata(&unresolved) else {
        return Ok(Vec::new());
    };
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > MAX_OPTION_DIAGNOSTIC_BYTES
    {
        return Err(WorkbenchError::new(format!(
            "option diagnostic sidecar {} must be a regular file no larger than {} bytes",
            relative.display(),
            MAX_OPTION_DIAGNOSTIC_BYTES
        )));
    }
    let path = checked_artifact_path(repository_root, &relative)?;
    let bytes = fs::read(&path)
        .map_err(|error| WorkbenchError::new(format!("cannot read {}: {error}", path.display())))?;
    let bundle: OptionDiagnosticBundle = serde_json::from_slice(&bytes).map_err(|error| {
        WorkbenchError::new(format!("cannot decode {}: {error}", path.display()))
    })?;
    bundle.validate_against_tape(tape).map_err(|error| {
        WorkbenchError::new(format!(
            "invalid option diagnostics {}: {error}",
            path.display()
        ))
    })?;
    Ok(bundle.visualization())
}

fn load_segment_tape(
    segment: &Segment,
    repository_root: &Path,
) -> Result<InputTape, WorkbenchError> {
    let profile = segment.profile;
    match &segment.artifact {
        ArtifactSource::Baseline(candidate_profile) => {
            if *candidate_profile != profile {
                return Err(WorkbenchError::new(format!(
                    "segment {} baseline profile does not match its profile",
                    segment.id
                )));
            }
            Candidate::baseline(*candidate_profile)
                .compile()
                .map_err(|error| WorkbenchError::new(error.to_string()))
        }
        ArtifactSource::Candidate(path) => {
            let path = checked_artifact_path(repository_root, path)?;
            let bytes = fs::read(&path).map_err(|error| {
                WorkbenchError::new(format!("cannot read {}: {error}", path.display()))
            })?;
            let candidate: Candidate = serde_json::from_slice(&bytes).map_err(|error| {
                WorkbenchError::new(format!("cannot decode {}: {error}", path.display()))
            })?;
            if candidate.segment != profile {
                return Err(WorkbenchError::new(format!(
                    "candidate {} has the wrong segment profile",
                    path.display()
                )));
            }
            candidate
                .compile()
                .map_err(|error| WorkbenchError::new(error.to_string()))
        }
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
                })
        }
        ArtifactSource::Tape(path) => {
            let path = checked_artifact_path(repository_root, path)?;
            let bytes = fs::read(&path).map_err(|error| {
                WorkbenchError::new(format!("cannot read {}: {error}", path.display()))
            })?;
            InputTape::decode(&bytes)
                .map(|decoded| decoded.tape)
                .map_err(|error| {
                    WorkbenchError::new(format!("cannot decode {}: {error}", path.display()))
                })
        }
        #[allow(unreachable_patterns)]
        _ => Err(WorkbenchError::new(
            "artifact source is not supported by this workbench build",
        )),
    }
}

fn checked_artifact_path(root: &Path, relative: &Path) -> Result<PathBuf, WorkbenchError> {
    if relative.is_absolute() {
        return Err(WorkbenchError::new(format!(
            "artifact path {} must be repository-relative",
            relative.display()
        )));
    }
    let canonical_root = fs::canonicalize(root).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot resolve repository root {}: {error}",
            root.display()
        ))
    })?;
    let candidate = fs::canonicalize(canonical_root.join(relative)).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot resolve artifact {}: {error}",
            relative.display()
        ))
    })?;
    if !candidate.starts_with(&canonical_root) {
        return Err(WorkbenchError::new(format!(
            "artifact {} escapes the repository root",
            relative.display()
        )));
    }
    if !candidate.is_file() {
        return Err(WorkbenchError::new(format!(
            "artifact {} is not a file",
            relative.display()
        )));
    }
    Ok(candidate)
}

fn canonical_file(path: &Path, label: &str) -> Result<PathBuf, WorkbenchError> {
    let canonical = fs::canonicalize(path).map_err(|error| {
        WorkbenchError::new(format!(
            "cannot resolve {label} {}: {error}",
            path.display()
        ))
    })?;
    if !canonical.is_file() {
        return Err(WorkbenchError::new(format!(
            "{label} {} is not a file",
            path.display()
        )));
    }
    Ok(canonical)
}

fn validate_play_request(request: &PlayRequest) -> Result<(), WorkbenchError> {
    match (&request.lineage, &request.standalone_segment) {
        (Some(lineage), None) if !lineage.trim().is_empty() => {}
        (None, Some(segment)) if !segment.trim().is_empty() => {}
        (Some(_), Some(_)) => {
            return Err(WorkbenchError::new(
                "lineage and standalone_segment are mutually exclusive",
            ));
        }
        _ => {
            return Err(WorkbenchError::new(
                "lineage or standalone_segment is required",
            ));
        }
    }
    match (
        &request.segment,
        request.frame,
        request.standalone_segment.is_some(),
    ) {
        (Some(_), Some(_), false) | (None, None, _) | (None, Some(_), true) => {}
        _ => {
            return Err(WorkbenchError::new(
                "segment and frame must be supplied together",
            ));
        }
    }
    if request.through_segment.is_some() && request.segment.is_some() {
        return Err(WorkbenchError::new(
            "through_segment and segment/frame are mutually exclusive",
        ));
    }
    if request.standalone_segment.is_some()
        && (request.through_segment.is_some() || request.segment.is_some())
    {
        return Err(WorkbenchError::new(
            "standalone segment playback accepts frame only, not lineage segment selectors",
        ));
    }
    Ok(())
}

fn materialize_play_request(
    timeline: &Timeline,
    repository_root: &Path,
    request: &PlayRequest,
) -> Result<MaterializedPlayback, WorkbenchError> {
    validate_play_request(request)?;
    if let Some(segment_id) = &request.standalone_segment {
        return materialize_segment_playback(timeline, repository_root, segment_id, request.frame);
    }
    let lineage = request.lineage.as_deref().expect("validated lineage");
    let materialized =
        materialize_lineage(timeline, repository_root, lineage, play_target(request)?)?;
    let seed_stage = materialized.steps.first().and_then(|step| {
        legacy_seed_stage(&materialized.tape, timeline.segments[&step.segment].profile)
    });
    Ok(MaterializedPlayback {
        lineage: Some(lineage.into()),
        segment: None,
        tape: materialized.tape,
        seed_stage,
    })
}

fn materialize_segment_playback(
    timeline: &Timeline,
    repository_root: &Path,
    segment_id: &str,
    local_frame: Option<u64>,
) -> Result<MaterializedPlayback, WorkbenchError> {
    let segment = timeline
        .segments
        .get(segment_id)
        .ok_or_else(|| WorkbenchError::new(format!("unknown segment {segment_id:?}")))?;
    let mut chain = materialize_segment_chain(timeline, repository_root, segment_id)?;
    if let Some(frame) = local_frame {
        let local_last =
            logical_last_frame(segment, &load_segment_tape(segment, repository_root)?)?;
        if frame > local_last {
            return Err(WorkbenchError::new(format!(
                "frame {frame} is outside segment {segment_id:?} (last logical frame is {local_last})"
            )));
        }
        let selected = chain.steps.last().expect("segment chain is non-empty");
        let chain_last = selected
            .chain_start_frame
            .checked_add(frame)
            .ok_or_else(|| WorkbenchError::new("frame selection overflow"))?;
        chain.tape.frames.truncate(chain_last as usize + 1);
    }
    let seed_profile = chain
        .steps
        .first()
        .map(|step| timeline.segments[&step.segment].profile)
        .unwrap_or(segment.profile);
    let seed_stage = legacy_seed_stage(&chain.tape, seed_profile);
    Ok(MaterializedPlayback {
        lineage: None,
        segment: Some(segment_id.into()),
        tape: chain.tape,
        seed_stage,
    })
}

fn play_segment(
    timeline: &Timeline,
    config: &WorkbenchConfig,
    segment_id: &str,
    stop: &BrowserStop,
    options: SegmentPlaybackOptions,
) -> Result<(PlayResponse, Child), WorkbenchError> {
    if !timeline.segments.contains_key(segment_id) {
        if !matches!(stop, BrowserStop::Segment { segment } if segment == segment_id) {
            return Err(WorkbenchError::new(
                "generated search playback only supports its proved endpoint",
            ));
        }
        let projection = visible_generated_search_projections(
            timeline,
            &config.repository_root.join("build/search"),
            &config.state_root,
        )?
        .into_iter()
        .find(|projection| projection.segment.id == segment_id)
        .ok_or_else(|| {
            WorkbenchError::new(format!(
                "unknown or expired generated search segment {segment_id:?}"
            ))
        })?;
        let bytes = fs::read(&projection.full_tape).map_err(|error| {
            WorkbenchError::new(format!(
                "cannot read generated search tape {}: {error}",
                projection.full_tape.display()
            ))
        })?;
        let tape = InputTape::decode(&bytes)
            .map_err(|error| WorkbenchError::new(format!("invalid generated tape: {error}")))?
            .tape;
        let materialized = MaterializedPlayback {
            lineage: None,
            segment: Some(segment_id.into()),
            tape,
            seed_stage: None,
        };
        let artifact_root = configured_artifact_root(config)?;
        let fast_forward_frames = match options.origin {
            PlaybackOrigin::Boot => None,
            PlaybackOrigin::Parent => {
                segment_parent_frame_count(
                    timeline,
                    &artifact_root,
                    projection.segment.parent.as_deref(),
                    &materialized.tape,
                    segment_id,
                )?;
                options
                    .playback
                    .fast
                    .then_some(materialized.tape.frames.len() as u64)
            }
        };
        return launch_materialized(
            timeline,
            config,
            materialized,
            MaterializedLaunchOptions {
                takeover: options.handoff,
                origin: options.origin,
                fast_forward_frames,
                thumbnail: None,
                playback: options.playback,
            },
        );
    }
    let local_frame = match stop {
        BrowserStop::Segment { segment } if segment == segment_id => None,
        BrowserStop::Segment { segment } => {
            return Err(WorkbenchError::new(format!(
                "selected segment {segment_id:?} cannot stop at {segment:?}"
            )));
        }
        BrowserStop::Tick { tick } => Some(*tick),
    };
    let artifact_root = configured_artifact_root(config)?;
    let materialized =
        materialize_segment_playback(timeline, &artifact_root, segment_id, local_frame)?;
    let fast_forward_frames = match options.origin {
        PlaybackOrigin::Boot => None,
        PlaybackOrigin::Parent => {
            segment_parent_frame_count(
                timeline,
                &artifact_root,
                timeline.segments[segment_id].parent.as_deref(),
                &materialized.tape,
                segment_id,
            )?;
            options
                .playback
                .fast
                .then_some(materialized.tape.frames.len() as u64)
        }
    };
    let thumbnail = prepare_missing_playback_thumbnail(
        timeline,
        config,
        &BrowserSelection::Segment {
            id: segment_id.into(),
        },
    )?;
    launch_materialized(
        timeline,
        config,
        materialized,
        MaterializedLaunchOptions {
            takeover: options.handoff,
            origin: options.origin,
            fast_forward_frames,
            thumbnail,
            playback: options.playback,
        },
    )
}

fn segment_parent_frame_count(
    timeline: &Timeline,
    repository_root: &Path,
    parent_id: Option<&str>,
    full_tape: &InputTape,
    segment_id: &str,
) -> Result<u64, WorkbenchError> {
    let parent_id = parent_id.ok_or_else(|| {
        WorkbenchError::new(format!(
            "segment {segment_id:?} begins at Boot and has no parent playback boundary"
        ))
    })?;
    let parent = materialize_segment_chain(timeline, repository_root, parent_id)?.tape;
    let parent_frames = parent.frames.len();
    let continuation_frames = full_tape.frames.len().saturating_sub(parent_frames);
    validate_parent_boundary(
        parent_frames as u64,
        continuation_frames as u64,
        full_tape.frames.len() as u64,
    )
    .map_err(|_| {
        WorkbenchError::new(format!(
            "segment {segment_id:?} has no nonempty continuation after its parent"
        ))
    })?;
    if full_tape.tick_rate_numerator != parent.tick_rate_numerator
        || full_tape.tick_rate_denominator != parent.tick_rate_denominator
        || full_tape.frames[..parent_frames] != parent.frames
    {
        return Err(WorkbenchError::new(format!(
            "segment {segment_id:?} playback does not begin with its exact parent chain"
        )));
    }
    Ok(parent_frames as u64)
}

fn materialize_draft(
    timeline: &Timeline,
    repository_root: &Path,
    state_root: &Path,
    draft_id: &str,
) -> Result<MaterializedPlayback, WorkbenchError> {
    enum DraftBase {
        Boot,
        Segment { id: String },
    }

    let manifests = scan_draft_manifests(state_root)?;
    let mut cursor = draft_id.to_owned();
    let mut seen = BTreeSet::new();
    let mut continuations = Vec::new();
    let base_segment = loop {
        if !seen.insert(cursor.clone()) {
            return Err(WorkbenchError::new("draft parent graph contains a cycle"));
        }
        let manifest = manifests
            .get(&cursor)
            .ok_or_else(|| WorkbenchError::new(format!("unknown draft {cursor:?}")))?;
        if manifest.status != DraftStatus::Ready {
            return Err(WorkbenchError::new(format!(
                "draft {cursor:?} is {:?}, not ready",
                manifest.status
            )));
        }
        continuations.push(manifest.clone());
        match &manifest.parent {
            DraftParent::Milestone {
                id,
                program_sha256,
                definition_sha256,
                boundary_fingerprint,
            } => {
                let program = milestone_program_projection(timeline, repository_root)?
                    .ok_or_else(|| WorkbenchError::new("Boot parent has no milestone program"))?;
                let definition = program
                    .definitions
                    .iter()
                    .find(|definition| definition.name == *id)
                    .ok_or_else(|| WorkbenchError::new("Boot parent milestone is missing"))?;
                if program.program_sha256 != *program_sha256
                    || definition.definition_sha256 != *definition_sha256
                    || !is_exact_boot_boundary_predicate(definition)
                    || !manifest.start_boundary_verified
                    || !boundary_fingerprint
                        .as_deref()
                        .is_some_and(native_fingerprint)
                    || manifest.parent_tape_sha256 != tape_digest(&InputTape::default())?
                {
                    return Err(WorkbenchError::new("Boot parent proof is missing or stale"));
                }
                break DraftBase::Boot;
            }
            DraftParent::Segment {
                id,
                terminal_milestone: _,
                boundary_fingerprint,
            } => {
                let segment = timeline
                    .segments
                    .get(id)
                    .ok_or_else(|| WorkbenchError::new("draft parent segment is missing"))?;
                if segment.end_fingerprint != *boundary_fingerprint {
                    return Err(WorkbenchError::new("draft parent segment boundary changed"));
                }
                break DraftBase::Segment { id: id.clone() };
            }
            DraftParent::Draft { id, .. } => cursor = id.clone(),
        }
    };
    continuations.reverse();

    let (mut tape, seed_stage, base_label) = match base_segment {
        DraftBase::Boot => (InputTape::default(), None, "boot".to_owned()),
        DraftBase::Segment { id: base_segment } => {
            let base_tape = materialize_segment_chain(timeline, repository_root, &base_segment)?;
            let seed_stage = base_tape.steps.first().and_then(|step| {
                legacy_seed_stage(&base_tape.tape, timeline.segments[&step.segment].profile)
            });
            (base_tape.tape, seed_stage, base_segment)
        }
    };
    let root = drafts_root(state_root)?;
    for manifest in continuations {
        let digest = tape_digest(&tape)?;
        if digest != manifest.parent_tape_sha256 {
            return Err(WorkbenchError::new(format!(
                "draft {:?} parent tape fingerprint changed",
                manifest.id
            )));
        }
        if let DraftParent::Draft {
            parent_tape_sha256, ..
        } = &manifest.parent
            && *parent_tape_sha256 != digest
        {
            return Err(WorkbenchError::new(format!(
                "draft {:?} has inconsistent parent metadata",
                manifest.id
            )));
        }
        let (encoded, continuation) = read_draft_tape(&root.join(&manifest.id))?;
        if continuation.frames.is_empty()
            || manifest.tape_bytes != Some(encoded.len() as u64)
            || manifest.frames != Some(continuation.frames.len() as u64)
            || manifest.tape_sha256.as_deref()
                != Some(format!("{:x}", Sha256::digest(&encoded)).as_str())
        {
            return Err(WorkbenchError::new(format!(
                "draft {:?} continuation metadata is unverified",
                manifest.id
            )));
        }
        tape = concatenate(vec![
            ChainSegment::all(tape),
            ChainSegment::all(continuation),
        ])
        .map_err(|error| WorkbenchError::new(error.to_string()))?
        .tape;
        if manifest.result_tape_sha256.as_deref() != Some(tape_digest(&tape)?.as_str()) {
            return Err(WorkbenchError::new(format!(
                "draft {:?} finalized chain fingerprint changed",
                manifest.id
            )));
        }
    }
    Ok(MaterializedPlayback {
        lineage: None,
        segment: Some(format!("{base_label}:{draft_id}")),
        tape,
        seed_stage,
    })
}

fn play_target(request: &PlayRequest) -> Result<MaterializeTarget, WorkbenchError> {
    if let Some(segment) = &request.through_segment {
        return Ok(MaterializeTarget::ThroughSegment(segment.clone()));
    }
    match (&request.segment, request.frame) {
        (Some(segment), Some(frame)) => Ok(MaterializeTarget::ThroughSegmentFrame {
            segment: segment.clone(),
            frame,
        }),
        (None, None) => Ok(MaterializeTarget::FullLineage),
        _ => Err(WorkbenchError::new(
            "segment and frame must be supplied together",
        )),
    }
}

fn validate_playback_origin(request: &BrowserPlayRequest) -> Result<(), WorkbenchError> {
    if request.fast && request.origin != PlaybackOrigin::Parent {
        return Err(WorkbenchError::new(
            "fast playback requires a parent-origin segment or ready draft selection",
        ));
    }
    if request.origin == PlaybackOrigin::Parent
        && !matches!(
            request.selection,
            BrowserSelection::Draft { .. } | BrowserSelection::Segment { .. }
        )
    {
        return Err(WorkbenchError::new(
            "parent-origin playback requires a segment or ready draft selection",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
