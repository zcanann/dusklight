//! Local, Git-backed route workbench primitives.
//!
//! Authored `.timeline` files and the artifacts they name are authoritative.
//! This module deliberately does not mutate timelines or introduce another
//! version-control layer. It only projects them as graph JSON and materializes
//! exact, pinned lineages for visual playback.

use crate::search::Candidate;
use crate::tape::InputTape;
use crate::tape_chain::{ChainSegment, SegmentFrames, concatenate};
use crate::timeline::{ArtifactSource, LineageKind, ResolvedLineage, Timeline, Variant};
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

const GRAPH_SCHEMA: &str = "dusklight.route-workbench.graph.v2";
const DRAFT_SCHEMA: &str = "dusklight.route-workbench.draft.v1";
const DRAFT_MANIFEST: &str = "draft.json";
const DRAFT_FINAL_MANIFEST: &str = "draft.final.json";
const DRAFT_LAUNCH: &str = "launch.json";
const DRAFT_TAPE: &str = "continuation.tape";
const MAX_DRAFTS: usize = 10_000;
const MAX_HTTP_HEADER: usize = 64 * 1024;
const MAX_HTTP_BODY: usize = 1024 * 1024;

fn active_recordings() -> &'static Mutex<BTreeSet<String>> {
    static ACTIVE: OnceLock<Mutex<BTreeSet<String>>> = OnceLock::new();
    ACTIVE.get_or_init(|| Mutex::new(BTreeSet::new()))
}

#[derive(Clone, Debug, Serialize)]
pub struct WorkbenchGraph {
    pub schema: String,
    pub timeline: String,
    pub milestones: Vec<GraphMilestone>,
    pub segments: Vec<GraphSegment>,
    pub variants: Vec<GraphVariant>,
    pub lineages: Vec<GraphLineage>,
    pub drafts: Vec<GraphDraft>,
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
    pub tape_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DraftParent {
    Variant {
        id: String,
        lineage: String,
        step_index: usize,
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
pub struct GraphMilestone {
    pub id: String,
    pub ordinal: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct GraphSegment {
    pub id: String,
    pub from: String,
    pub to: String,
    pub profile: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct GraphVariant {
    pub id: String,
    pub segment: String,
    pub name: String,
    pub incumbent: bool,
    pub artifact: GraphArtifact,
    pub start_fingerprint: String,
    pub boundary_fingerprint: String,
    pub first_hit_tick: Option<u64>,
    pub frame_count: Option<u64>,
    pub start_tick: u64,
    pub end_tick: Option<u64>,
    pub ticks: Option<u64>,
    pub playable: bool,
    pub lineage_composable: bool,
    pub recordable: bool,
    pub record_anchors: Vec<GraphRecordAnchor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct GraphRecordAnchor {
    pub lineage: String,
    pub step_index: usize,
    pub terminal_milestone: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct GraphArtifact {
    pub kind: String,
    pub value: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct GraphLineage {
    pub name: String,
    pub kind: String,
    pub root_fingerprint: String,
    pub steps: Vec<GraphLineageStep>,
    pub frame_count: Option<u64>,
    pub playable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct GraphLineageStep {
    pub variant: String,
    pub segment: String,
    pub from: String,
    pub to: String,
    pub parent_variant: String,
    pub checkpoint_fingerprint: String,
    pub source_frames: Option<u64>,
    pub chain_start_frame: Option<u64>,
    pub chain_end_frame: Option<u64>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayRequest {
    #[serde(default)]
    pub lineage: Option<String>,
    #[serde(default)]
    pub variant: Option<String>,
    #[serde(default)]
    pub through_milestone: Option<String>,
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
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BrowserSelection {
    Variant { id: String },
    Draft { id: String },
    Segment { id: String },
    Milestone { id: String },
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserRecordRequest {
    pub parent: BrowserRecordParent,
    #[serde(default)]
    pub label: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BrowserRecordParent {
    Variant {
        id: String,
        lineage: String,
        step_index: usize,
        terminal_milestone: String,
    },
    Draft {
        id: String,
    },
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BrowserStop {
    Milestone { milestone: String },
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
    pub variant: Option<String>,
    pub tape: PathBuf,
    pub session_state_root: PathBuf,
    pub session_id: String,
    pub frames: u64,
    pub input_tape_end: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct RecordResponse {
    pub pid: u32,
    pub draft_id: String,
    pub manifest: PathBuf,
    pub tape: PathBuf,
    pub frames_before_recording: u64,
}

#[derive(Clone, Debug)]
pub enum MaterializeTarget {
    FullLineage,
    ThroughMilestone(String),
    ThroughSegmentFrame { segment: String, frame: u64 },
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
    variant: Option<String>,
    tape: InputTape,
    seed_stage: Option<&'static str>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MaterializedStep {
    pub variant: String,
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

/// Parse the checked-in timeline on every request so edits in the Git working
/// tree are visible without synchronizing a secondary store.
pub fn load_authoritative_timeline(path: &Path) -> Result<Timeline, WorkbenchError> {
    let source = fs::read_to_string(path)
        .map_err(|error| WorkbenchError::new(format!("cannot read {}: {error}", path.display())))?;
    Timeline::parse(&source).map_err(|error| WorkbenchError::new(error.to_string()))
}

/// Build the JSON-ready graph projection used by the visual selector.
/// Missing or unsupported artifacts remain visible with `playable: false`.
pub fn graph_from_timeline(
    timeline: &Timeline,
    repository_root: &Path,
) -> Result<WorkbenchGraph, WorkbenchError> {
    let inspection = timeline
        .inspect()
        .map_err(|error| WorkbenchError::new(error.to_string()))?;
    let milestones = timeline
        .milestones
        .iter()
        .enumerate()
        .map(|(ordinal, id)| GraphMilestone {
            id: id.clone(),
            ordinal,
        })
        .collect();
    let segments = timeline
        .segments
        .values()
        .map(|segment| GraphSegment {
            id: segment.name.clone(),
            from: segment.from.clone(),
            to: segment.to.clone(),
            profile: segment.profile.as_str().into(),
        })
        .collect();

    let variants = timeline
        .variants
        .values()
        .map(|variant| {
            let loaded = load_variant_tape(timeline, variant, repository_root);
            let record_anchors =
                record_anchors_for_variant(timeline, &inspection.lineages, &variant.id);
            let record_anchors = record_anchors
                .into_iter()
                .filter(|anchor| {
                    materialize_lineage(
                        timeline,
                        repository_root,
                        &anchor.lineage,
                        MaterializeTarget::ThroughMilestone(anchor.terminal_milestone.clone()),
                    )
                    .is_ok_and(|materialized| {
                        materialized.steps.len() == anchor.step_index + 1
                            && materialized
                                .steps
                                .last()
                                .is_some_and(|step| step.variant == variant.id)
                    })
                })
                .collect::<Vec<_>>();
            GraphVariant {
                id: variant.id.clone(),
                segment: variant.segment.clone(),
                name: variant.name.clone(),
                incumbent: variant.incumbent,
                artifact: graph_artifact(&variant.artifact),
                start_fingerprint: variant.start_fingerprint.clone(),
                boundary_fingerprint: variant.boundary_fingerprint.clone(),
                first_hit_tick: variant.first_hit_tick,
                frame_count: loaded.as_ref().ok().map(|tape| tape.frames.len() as u64),
                start_tick: 0,
                end_tick: loaded
                    .as_ref()
                    .ok()
                    .and_then(|tape| (tape.frames.len() as u64).checked_sub(1)),
                ticks: variant.first_hit_tick,
                playable: loaded.is_ok(),
                lineage_composable: artifact_is_canonical_payload(&variant.artifact)
                    && fingerprints_are_exact(variant),
                recordable: loaded.is_ok() && !record_anchors.is_empty(),
                record_anchors,
                error: loaded.err().map(|error| error.to_string()),
            }
        })
        .collect();
    let lineages = inspection
        .lineages
        .iter()
        .map(|lineage| graph_lineage(timeline, lineage, repository_root))
        .collect();
    Ok(WorkbenchGraph {
        schema: GRAPH_SCHEMA.into(),
        timeline: timeline.name.clone(),
        milestones,
        segments,
        variants,
        lineages,
        drafts: Vec::new(),
    })
}

fn record_anchors_for_variant(
    timeline: &Timeline,
    lineages: &[ResolvedLineage],
    variant_id: &str,
) -> Vec<GraphRecordAnchor> {
    let mut candidates = lineages
        .iter()
        .filter_map(|lineage| {
            let step = lineage
                .steps
                .iter()
                .position(|step| step.variant == variant_id)?;
            ensure_composable_lineage(timeline, lineage, step + 1).ok()?;
            if !native_fingerprint(&timeline.variants[variant_id].boundary_fingerprint) {
                return None;
            }
            Some(GraphRecordAnchor {
                lineage: lineage.name.clone(),
                step_index: step,
                terminal_milestone: timeline.segments[&timeline.variants[variant_id].segment]
                    .to
                    .clone(),
            })
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(|anchor| {
        (
            anchor.lineage != "main",
            anchor.lineage.clone(),
            anchor.step_index,
        )
    });
    candidates
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
    graph.drafts = scan_drafts(timeline, repository_root, state_root)?;
    Ok(graph)
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

fn scan_draft_manifests(
    state_root: &Path,
) -> Result<BTreeMap<String, DraftManifest>, WorkbenchError> {
    let root = drafts_root(state_root)?;
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
            if active_recordings()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .contains(&id)
            {
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

fn read_draft_launch(directory: &Path, manifest: &DraftManifest) -> Option<DraftLaunch> {
    let bytes = fs::read(directory.join(DRAFT_LAUNCH)).ok()?;
    let launch: DraftLaunch = serde_json::from_slice(&bytes).ok()?;
    (launch.schema == "dusklight.route-workbench.launch.v1"
        && launch.id == manifest.id
        && launch.session_token == manifest.session_token)
        .then_some(launch)
}

fn scan_drafts(
    timeline: &Timeline,
    repository_root: &Path,
    state_root: &Path,
) -> Result<Vec<GraphDraft>, WorkbenchError> {
    let manifests = scan_draft_manifests(state_root)?;
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
                tape_bytes: manifest.tape_bytes,
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
        match &manifest.parent {
            DraftParent::Variant {
                id: variant_id,
                lineage,
                step_index,
                terminal_milestone,
                boundary_fingerprint,
            } => {
                let variant = timeline
                    .variants
                    .get(variant_id)
                    .ok_or_else(|| "parent variant no longer exists".to_owned())?;
                if variant.boundary_fingerprint != *boundary_fingerprint
                    || !manifest.start_boundary_verified
                    || !record_anchors_for_variant(
                        timeline,
                        &timeline
                            .inspect()
                            .map_err(|error| error.to_string())?
                            .lineages,
                        variant_id,
                    )
                    .iter()
                    .any(|anchor| {
                        anchor.lineage == *lineage
                            && anchor.step_index == *step_index
                            && anchor.terminal_milestone == *terminal_milestone
                    })
                {
                    return Err("curated lineage anchor is no longer exact".into());
                }
                let key = (lineage.clone(), *step_index);
                let parent_digest = if let Some(result) = anchor_digests.get(&key) {
                    result.clone()?
                } else {
                    let result = materialize_lineage(
                        timeline,
                        repository_root,
                        lineage,
                        MaterializeTarget::ThroughMilestone(terminal_milestone.clone()),
                    )
                    .map_err(|error| error.to_string())
                    .and_then(|materialized| {
                        if materialized.steps.len() != *step_index + 1
                            || materialized
                                .steps
                                .last()
                                .is_none_or(|step| step.variant != *variant_id)
                        {
                            Err("curated lineage occurrence changed".into())
                        } else {
                            tape_digest(&materialized.tape).map_err(|error| error.to_string())
                        }
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

#[cfg(not(any(windows, target_os = "linux")))]
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

/// Resolve a pinned lineage and concatenate its immutable artifacts. A frame
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
        let variant = &timeline.variants[&step.variant];
        let tape = load_variant_tape(timeline, variant, repository_root)?;
        let logical_last = logical_last_frame(variant, &tape)?;
        let frame_window = match &target {
            MaterializeTarget::ThroughSegmentFrame { segment, frame }
                if index + 1 == selected && variant.segment == *segment =>
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
            name: Some(variant.id.clone()),
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
            let variant_id = boundary
                .segment_name
                .as_ref()
                .expect("workbench always names chain segments");
            MaterializedStep {
                variant: variant_id.clone(),
                segment: timeline.variants[variant_id].segment.clone(),
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
    launch_materialized(config, materialized, request.takeover)
}

fn launch_materialized(
    config: &WorkbenchConfig,
    materialized: MaterializedPlayback,
    takeover: bool,
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
    let end = if takeover { "release" } else { "hold" };
    let mut command = Command::new(game);
    command
        .current_dir(&config.working_directory)
        .arg("--dvd")
        .arg(dvd)
        .arg("--input-tape")
        .arg(&tape_path)
        .arg("--input-tape-end")
        .arg(end)
        .arg("--fixed-step")
        .arg("--automation-data-root")
        .arg(&state_root)
        .arg("--cvar")
        .arg("game.instantSaves=true")
        .arg("--cvar")
        .arg("backend.cardFileType=1")
        .arg("--cvar")
        .arg("backend.wasPresetChosen=true")
        .arg("--cvar")
        .arg("game.enableMenuPointer=false");
    if let Some(stage) = materialized.seed_stage {
        command.arg("--stage").arg(stage);
    }
    let child = command
        .spawn()
        .map_err(|error| WorkbenchError::new(format!("cannot launch Dusklight: {error}")))?;
    let response = PlayResponse {
        pid: child.id(),
        lineage: materialized.lineage,
        variant: materialized.variant,
        tape: tape_path,
        session_state_root: state_root,
        session_id,
        frames: materialized.tape.frames.len() as u64,
        input_tape_end: end.into(),
    };
    Ok((response, child))
}

fn play_draft(
    timeline: &Timeline,
    config: &WorkbenchConfig,
    draft_id: &str,
) -> Result<(PlayResponse, Child), WorkbenchError> {
    let artifact_root = configured_artifact_root(config)?;
    let materialized = materialize_draft(timeline, &artifact_root, &config.state_root, draft_id)?;
    launch_materialized(config, materialized, true)
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
        let label = request.label.trim();
        if label.len() > 160 || label.chars().any(char::is_control) {
            return Err(WorkbenchError::new(
                "draft label must be at most 160 characters without controls",
            ));
        }
        label.to_owned()
    };
    let (materialized, parent, expected_start_milestone, expected_start_fingerprint) = match request
        .parent
    {
        BrowserRecordParent::Variant {
            id,
            lineage,
            step_index,
            terminal_milestone,
        } => {
            let variant = timeline
                .variants
                .get(&id)
                .ok_or_else(|| WorkbenchError::new(format!("unknown variant {id:?}")))?;
            let anchors = record_anchors_for_variant(
                timeline,
                &timeline
                    .inspect()
                    .map_err(|error| WorkbenchError::new(error.to_string()))?
                    .lineages,
                &id,
            );
            if !anchors.iter().any(|anchor| {
                anchor.lineage == lineage
                    && anchor.step_index == step_index
                    && anchor.terminal_milestone == terminal_milestone
            }) || !native_fingerprint(&variant.boundary_fingerprint)
            {
                return Err(WorkbenchError::new(
                    "recording requires the concrete composable lineage anchor advertised by the graph",
                ));
            }
            let lineage_tape = materialize_lineage(
                timeline,
                &artifact_root,
                &lineage,
                MaterializeTarget::ThroughMilestone(terminal_milestone.clone()),
            )?;
            if lineage_tape.steps.len() != step_index + 1
                || lineage_tape.steps.last().map(|step| step.variant.as_str()) != Some(id.as_str())
            {
                return Err(WorkbenchError::new(
                    "lineage anchor does not resolve to the requested concrete occurrence",
                ));
            }
            let first_segment = lineage_tape.steps.first().map(|step| step.segment.clone());
            let seed_stage = first_segment.as_deref().and_then(|segment| {
                match timeline.segments[segment].profile {
                    crate::search::SegmentProfile::BootToFsp103 => None,
                    crate::search::SegmentProfile::Fsp103ToFsp104 => Some("F_SP103,1,1,3"),
                }
            });
            let materialized = MaterializedPlayback {
                lineage: Some(lineage.clone()),
                variant: Some(id.clone()),
                tape: lineage_tape.tape,
                seed_stage,
            };
            let parent = DraftParent::Variant {
                id,
                lineage,
                step_index,
                terminal_milestone,
                boundary_fingerprint: variant.boundary_fingerprint.clone(),
            };
            let native_milestone = match timeline.segments[&variant.segment].profile {
                crate::search::SegmentProfile::BootToFsp103 => "gameplay-ready-f-sp103",
                crate::search::SegmentProfile::Fsp103ToFsp104 => "entered-f-sp104",
            };
            (
                materialized,
                parent,
                Some(native_milestone.to_owned()),
                Some(variant.boundary_fingerprint.clone()),
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
        .arg(dvd)
        .arg("--input-tape")
        .arg(&playback)
        .arg("--input-tape-end")
        .arg("release")
        .arg("--record-input-tape")
        .arg(&continuation)
        .arg("--record-input-capacity")
        .arg("1080000")
        .arg("--record-input-session")
        .arg(&session_token)
        .arg("--fixed-step")
        .arg("--automation-data-root")
        .arg(&state)
        .arg("--cvar")
        .arg("game.instantSaves=true")
        .arg("--cvar")
        .arg("backend.cardFileType=1")
        .arg("--cvar")
        .arg("backend.wasPresetChosen=true")
        .arg("--cvar")
        .arg("game.enableMenuPointer=false");
    if let (Some(milestone), Some(fingerprint)) =
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
        schema: "dusklight.route-workbench.launch.v1".into(),
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
    thread::spawn(move || monitor_recording(child, monitor_directory, manifest, monitor_id));
    Ok(RecordResponse {
        pid,
        draft_id,
        manifest: directory.join(DRAFT_MANIFEST),
        tape: continuation,
        frames_before_recording: materialized.tape.frames.len() as u64,
    })
}

fn monitor_recording(
    mut child: Child,
    directory: PathBuf,
    mut manifest: DraftManifest,
    draft_id: String,
) {
    match child.wait() {
        Ok(exit) => finalize_recording(&directory, &mut manifest, Some(exit.success())),
        Err(error) => {
            manifest.status = DraftStatus::ProcessFailure;
            manifest.error = Some(format!("cannot wait for Dusklight: {error}"));
        }
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
    let expected_boundary_matches = match (
        &manifest.expected_start_milestone,
        &manifest.expected_start_fingerprint,
    ) {
        (Some(milestone), Some(fingerprint)) => {
            native.start_milestone.as_deref() == Some(milestone)
                && native.start_fingerprint.as_deref() == Some(fingerprint)
                && native.start_tape_frame == manifest.parent_frames.checked_sub(1)
        }
        (None, None) => {
            native.start_milestone.is_none()
                && native.start_fingerprint.is_none()
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
pub fn serve(listener: TcpListener, config: WorkbenchConfig) -> Result<(), WorkbenchError> {
    let address = listener
        .local_addr()
        .map_err(|error| WorkbenchError::new(error.to_string()))?;
    if !address.ip().is_loopback() {
        return Err(WorkbenchError::new(
            "route workbench must bind to a loopback address",
        ));
    }
    configured_artifact_root(&config)?;
    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => {
                let response = handle_http(&mut stream, address, &config);
                let _ = write_http_response(&mut stream, response);
            }
            Err(error) => return Err(WorkbenchError::new(format!("HTTP accept failed: {error}"))),
        }
    }
    Ok(())
}

fn graph_lineage(
    timeline: &Timeline,
    lineage: &ResolvedLineage,
    repository_root: &Path,
) -> GraphLineage {
    let mut cursor = Some(0_u64);
    let mut steps = Vec::new();
    for step in &lineage.steps {
        let variant = &timeline.variants[&step.variant];
        let source_frames = load_variant_tape(timeline, variant, repository_root)
            .and_then(|tape| logical_frame_count(variant, &tape))
            .ok();
        let start = cursor;
        let end = match (cursor, source_frames) {
            (Some(value), Some(count)) => value.checked_add(count),
            _ => None,
        };
        steps.push(GraphLineageStep {
            variant: variant.id.clone(),
            segment: variant.segment.clone(),
            from: timeline.segments[&variant.segment].from.clone(),
            to: timeline.segments[&variant.segment].to.clone(),
            parent_variant: step.after.parent_variant.clone(),
            checkpoint_fingerprint: step.after.checkpoint_fingerprint.clone(),
            source_frames,
            chain_start_frame: start,
            chain_end_frame: end,
        });
        cursor = end;
    }
    let composition_error = ensure_composable_lineage(timeline, lineage, lineage.steps.len())
        .err()
        .map(|error| error.to_string());
    GraphLineage {
        name: lineage.name.clone(),
        kind: match lineage.kind {
            LineageKind::Continuation => "continuation",
            LineageKind::Branch => "branch",
        }
        .into(),
        root_fingerprint: lineage.root_fingerprint.clone(),
        steps,
        frame_count: cursor,
        playable: cursor.is_some() && composition_error.is_none(),
        error: composition_error,
    }
}

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
        MaterializeTarget::ThroughMilestone(milestone) => {
            if lineage.steps.is_empty() {
                return Err(WorkbenchError::new(format!(
                    "lineage {:?} has no milestones",
                    lineage.name
                )));
            }
            let first = &timeline.variants[&lineage.steps[0].variant];
            if timeline.segments[&first.segment].from == *milestone {
                return Ok(0);
            }
            lineage
                .steps
                .iter()
                .position(|step| {
                    let variant = &timeline.variants[&step.variant];
                    timeline.segments[&variant.segment].to == *milestone
                })
                .map(|index| index + 1)
                .ok_or_else(|| {
                    WorkbenchError::new(format!(
                        "milestone {milestone:?} is not on lineage {:?}",
                        lineage.name
                    ))
                })
        }
        MaterializeTarget::ThroughSegmentFrame { segment, .. } => lineage
            .steps
            .iter()
            .position(|step| timeline.variants[&step.variant].segment == *segment)
            .map(|index| index + 1)
            .ok_or_else(|| {
                WorkbenchError::new(format!(
                    "segment {segment:?} is not on lineage {:?}",
                    lineage.name
                ))
            }),
    }
}

fn ensure_composable_lineage(
    timeline: &Timeline,
    lineage: &ResolvedLineage,
    selected: usize,
) -> Result<(), WorkbenchError> {
    // A single variant can always be played under its profile seed. Crossing a
    // boundary is stricter: generated baselines/candidates may contain their
    // evaluation seed harness and therefore are not continuation payloads.
    if selected <= 1 {
        return Ok(());
    }
    for step in lineage.steps.iter().take(selected) {
        let variant = &timeline.variants[&step.variant];
        if !artifact_is_canonical_payload(&variant.artifact) {
            return Err(WorkbenchError::new(format!(
                "lineage {:?} cannot cross segment {}: variant {} is a stage-seeded baseline/candidate, not a canonical continuation tape",
                lineage.name, variant.segment, variant.id
            )));
        }
        if !fingerprints_are_exact(variant)
            || contains_placeholder(&step.after.checkpoint_fingerprint)
        {
            return Err(WorkbenchError::new(format!(
                "lineage {:?} cannot cross segment {}: variant {} uses placeholder fingerprints",
                lineage.name, variant.segment, variant.id
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

fn fingerprints_are_exact(variant: &Variant) -> bool {
    !contains_placeholder(&variant.start_fingerprint)
        && !contains_placeholder(&variant.boundary_fingerprint)
}

fn contains_placeholder(value: &str) -> bool {
    value.trim().is_empty() || value.to_ascii_lowercase().contains("unknown")
}

fn logical_frame_count(variant: &Variant, tape: &InputTape) -> Result<u64, WorkbenchError> {
    logical_last_frame(variant, tape).and_then(|last| {
        last.checked_add(1)
            .ok_or_else(|| WorkbenchError::new("frame count overflow"))
    })
}

fn logical_last_frame(variant: &Variant, tape: &InputTape) -> Result<u64, WorkbenchError> {
    if tape.frames.is_empty() {
        return Err(WorkbenchError::new(format!(
            "variant {} has an empty tape",
            variant.id
        )));
    }
    // first_hit_tick is a simulation score, not an artifact-local frame
    // boundary. Until canonical bundles carry an explicit tape_frame, the
    // complete artifact is the only safe payload boundary.
    Ok(tape.frames.len() as u64 - 1)
}

fn load_variant_tape(
    timeline: &Timeline,
    variant: &Variant,
    repository_root: &Path,
) -> Result<InputTape, WorkbenchError> {
    let profile = timeline
        .segments
        .get(&variant.segment)
        .ok_or_else(|| WorkbenchError::new(format!("unknown segment {:?}", variant.segment)))?
        .profile;
    match &variant.artifact {
        ArtifactSource::Baseline(candidate_profile) => {
            if *candidate_profile != profile {
                return Err(WorkbenchError::new(format!(
                    "variant {} baseline profile does not match its segment",
                    variant.id
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
    match (&request.lineage, &request.variant) {
        (Some(lineage), None) if !lineage.trim().is_empty() => {}
        (None, Some(variant)) if !variant.trim().is_empty() => {}
        (Some(_), Some(_)) => {
            return Err(WorkbenchError::new(
                "lineage and variant are mutually exclusive",
            ));
        }
        _ => return Err(WorkbenchError::new("lineage or variant is required")),
    }
    match (&request.segment, request.frame, request.variant.is_some()) {
        (Some(_), Some(_), false) | (None, None, _) | (None, Some(_), true) => {}
        _ => {
            return Err(WorkbenchError::new(
                "segment and frame must be supplied together",
            ));
        }
    }
    if request.through_milestone.is_some() && request.segment.is_some() {
        return Err(WorkbenchError::new(
            "through_milestone and segment/frame are mutually exclusive",
        ));
    }
    if request.variant.is_some()
        && (request.through_milestone.is_some() || request.segment.is_some())
    {
        return Err(WorkbenchError::new(
            "standalone variant playback accepts frame only, not lineage milestone/segment selectors",
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
    if let Some(variant_id) = &request.variant {
        let variant = timeline
            .variants
            .get(variant_id)
            .ok_or_else(|| WorkbenchError::new(format!("unknown variant {variant_id:?}")))?;
        let mut tape = load_variant_tape(timeline, variant, repository_root)?;
        let last = logical_last_frame(variant, &tape)?;
        let selected = request.frame.unwrap_or(last);
        if selected > last {
            return Err(WorkbenchError::new(format!(
                "frame {selected} is outside variant {variant_id:?} (last logical frame is {last})"
            )));
        }
        tape.frames.truncate(selected as usize + 1);
        let profile = timeline.segments[&variant.segment].profile;
        let seed_stage = match profile {
            crate::search::SegmentProfile::BootToFsp103 => None,
            crate::search::SegmentProfile::Fsp103ToFsp104 => Some("F_SP103,1,1,3"),
        };
        return Ok(MaterializedPlayback {
            lineage: None,
            variant: Some(variant_id.clone()),
            tape,
            seed_stage,
        });
    }
    let lineage = request.lineage.as_deref().expect("validated lineage");
    let materialized =
        materialize_lineage(timeline, repository_root, lineage, play_target(request)?)?;
    let seed_stage = materialized.steps.first().and_then(|step| {
        match timeline.segments[&step.segment].profile {
            crate::search::SegmentProfile::BootToFsp103 => None,
            crate::search::SegmentProfile::Fsp103ToFsp104 => Some("F_SP103,1,1,3"),
        }
    });
    Ok(MaterializedPlayback {
        lineage: Some(lineage.into()),
        variant: None,
        tape: materialized.tape,
        seed_stage,
    })
}

fn materialize_draft(
    timeline: &Timeline,
    repository_root: &Path,
    state_root: &Path,
    draft_id: &str,
) -> Result<MaterializedPlayback, WorkbenchError> {
    let manifests = scan_draft_manifests(state_root)?;
    let mut cursor = draft_id.to_owned();
    let mut seen = BTreeSet::new();
    let mut continuations = Vec::new();
    let base_variant = loop {
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
            DraftParent::Variant {
                id,
                lineage,
                step_index,
                terminal_milestone,
                boundary_fingerprint,
            } => {
                let variant = timeline
                    .variants
                    .get(id)
                    .ok_or_else(|| WorkbenchError::new("draft parent variant is missing"))?;
                if variant.boundary_fingerprint != *boundary_fingerprint
                    || timeline.segments[&variant.segment].to != *terminal_milestone
                    || !record_anchors_for_variant(
                        timeline,
                        &timeline
                            .inspect()
                            .map_err(|error| WorkbenchError::new(error.to_string()))?
                            .lineages,
                        id,
                    )
                    .iter()
                    .any(|anchor| {
                        anchor.lineage == *lineage
                            && anchor.step_index == *step_index
                            && anchor.terminal_milestone == *terminal_milestone
                    })
                {
                    return Err(WorkbenchError::new("draft parent lineage anchor changed"));
                }
                break (id.clone(), lineage.clone(), terminal_milestone.clone());
            }
            DraftParent::Draft { id, .. } => cursor = id.clone(),
        }
    };
    continuations.reverse();

    let (base_variant, base_lineage, terminal_milestone) = base_variant;
    let base_lineage_tape = materialize_lineage(
        timeline,
        repository_root,
        &base_lineage,
        MaterializeTarget::ThroughMilestone(terminal_milestone),
    )?;
    let expected_step_index = continuations
        .first()
        .and_then(|manifest| match &manifest.parent {
            DraftParent::Variant { step_index, .. } => Some(*step_index),
            DraftParent::Draft { .. } => None,
        })
        .expect("first continuation has curated parent");
    if base_lineage_tape.steps.len() != expected_step_index + 1
        || base_lineage_tape
            .steps
            .last()
            .map(|step| step.variant.as_str())
            != Some(base_variant.as_str())
    {
        return Err(WorkbenchError::new(
            "draft lineage anchor no longer resolves to its concrete occurrence",
        ));
    }
    let seed_stage = base_lineage_tape.steps.first().and_then(|step| {
        match timeline.segments[&step.segment].profile {
            crate::search::SegmentProfile::BootToFsp103 => None,
            crate::search::SegmentProfile::Fsp103ToFsp104 => Some("F_SP103,1,1,3"),
        }
    });
    let mut tape = base_lineage_tape.tape;
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
        variant: Some(format!("{base_variant}:{draft_id}")),
        tape,
        seed_stage,
    })
}

fn play_target(request: &PlayRequest) -> Result<MaterializeTarget, WorkbenchError> {
    if let Some(milestone) = &request.through_milestone {
        return Ok(MaterializeTarget::ThroughMilestone(milestone.clone()));
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

fn adapt_browser_request(
    timeline: &Timeline,
    request: BrowserPlayRequest,
) -> Result<PlayRequest, WorkbenchError> {
    let mut play = PlayRequest {
        lineage: None,
        variant: None,
        through_milestone: None,
        segment: None,
        frame: None,
        takeover: request.handoff,
    };
    match request.selection {
        BrowserSelection::Variant { id } => {
            if !timeline.variants.contains_key(&id) {
                return Err(WorkbenchError::new(format!("unknown variant {id:?}")));
            }
            play.variant = Some(id);
        }
        BrowserSelection::Draft { .. } => {
            return Err(WorkbenchError::new(
                "draft selections use persistent draft playback",
            ));
        }
        BrowserSelection::Segment { id } => {
            if !timeline.segments.contains_key(&id) {
                return Err(WorkbenchError::new(format!("unknown segment {id:?}")));
            }
            let incumbent = timeline
                .variants
                .values()
                .find(|variant| variant.segment == id && variant.incumbent)
                .ok_or_else(|| {
                    WorkbenchError::new(format!("segment {id:?} has no incumbent variant"))
                })?;
            play.variant = Some(incumbent.id.clone());
        }
        BrowserSelection::Milestone { id } => {
            if !timeline.milestones.contains(&id) {
                return Err(WorkbenchError::new(format!("unknown milestone {id:?}")));
            }
            let inspection = timeline
                .inspect()
                .map_err(|error| WorkbenchError::new(error.to_string()))?;
            let lineage = inspection
                .lineages
                .iter()
                .find(|lineage| lineage.name == "main")
                .or_else(|| inspection.lineages.first())
                .ok_or_else(|| WorkbenchError::new("timeline has no playable lineage"))?;
            play.lineage = Some(lineage.name.clone());
        }
    }

    match request.stop {
        BrowserStop::Tick { tick } => {
            if play.variant.is_some() {
                play.frame = Some(tick);
            } else {
                return Err(WorkbenchError::new(
                    "exact-tick scrubbing currently requires a variant or segment selection",
                ));
            }
        }
        BrowserStop::Milestone { milestone } => {
            if !timeline.milestones.contains(&milestone) {
                return Err(WorkbenchError::new(format!(
                    "unknown stop milestone {milestone:?}"
                )));
            }
            if let Some(variant_id) = &play.variant {
                let segment = &timeline.segments[&timeline.variants[variant_id].segment];
                if segment.to != milestone {
                    return Err(WorkbenchError::new(format!(
                        "standalone variant {variant_id} ends at {:?}, not {milestone:?}",
                        segment.to
                    )));
                }
            } else {
                play.through_milestone = Some(milestone);
            }
        }
    }
    Ok(play)
}

struct HttpResponse {
    status: u16,
    reason: &'static str,
    content_type: &'static str,
    body: Vec<u8>,
}

fn handle_http(
    stream: &mut TcpStream,
    server_address: SocketAddr,
    config: &WorkbenchConfig,
) -> HttpResponse {
    match read_http_request(stream) {
        Ok(request) => {
            if !origin_allowed(request.origin.as_deref(), server_address) {
                return json_error(403, "Forbidden", "cross-origin requests are not allowed");
            }
            match (request.method.as_str(), request.path.as_str()) {
                ("GET", "/") => html_response(include_bytes!("../assets/route_workbench.html")),
                ("GET", "/api/graph") => load_authoritative_timeline(&config.timeline_path)
                    .and_then(|timeline| {
                        let artifact_root = configured_artifact_root(config)?;
                        graph_with_drafts(&timeline, &artifact_root, &config.state_root)
                    })
                    .and_then(|graph| json_response(&graph))
                    .unwrap_or_else(|error| {
                        json_error(500, "Internal Server Error", &error.to_string())
                    }),
                ("POST", "/api/play") => {
                    let result = serde_json::from_slice::<BrowserPlayRequest>(&request.body)
                        .map_err(|error| {
                            WorkbenchError::new(format!("invalid play request: {error}"))
                        })
                        .and_then(|browser_request| {
                            let timeline = load_authoritative_timeline(&config.timeline_path)?;
                            let (response, _child) = match &browser_request.selection {
                                BrowserSelection::Draft { id } => {
                                    play_draft(&timeline, config, id)?
                                }
                                _ => {
                                    let play_request =
                                        adapt_browser_request(&timeline, browser_request)?;
                                    play(&timeline, config, &play_request)?
                                }
                            };
                            Ok(response)
                        });
                    match result {
                        Ok(response) => json_response(&response).unwrap_or_else(|error| {
                            json_error(500, "Internal Server Error", &error.to_string())
                        }),
                        Err(error) => json_error(400, "Bad Request", &error.to_string()),
                    }
                }
                ("POST", "/api/record") => {
                    let result = serde_json::from_slice::<BrowserRecordRequest>(&request.body)
                        .map_err(|error| {
                            WorkbenchError::new(format!("invalid record request: {error}"))
                        })
                        .and_then(|record_request| {
                            let timeline = load_authoritative_timeline(&config.timeline_path)?;
                            record_continuation(&timeline, config, record_request)
                        });
                    match result {
                        Ok(response) => json_response(&response).unwrap_or_else(|error| {
                            json_error(500, "Internal Server Error", &error.to_string())
                        }),
                        Err(error) => json_error(400, "Bad Request", &error.to_string()),
                    }
                }
                _ => json_error(404, "Not Found", "unknown route workbench endpoint"),
            }
        }
        Err(error) => json_error(400, "Bad Request", &error.to_string()),
    }
}

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

struct HttpRequest {
    method: String,
    path: String,
    origin: Option<String>,
    body: Vec<u8>,
}

fn read_http_request(stream: &mut TcpStream) -> Result<HttpRequest, WorkbenchError> {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| WorkbenchError::new(error.to_string()))?;
    let mut bytes = Vec::new();
    let header_end = loop {
        if bytes.len() >= MAX_HTTP_HEADER {
            return Err(WorkbenchError::new("HTTP header is too large"));
        }
        let mut chunk = [0_u8; 4096];
        let count = stream
            .read(&mut chunk)
            .map_err(|error| WorkbenchError::new(format!("cannot read HTTP request: {error}")))?;
        if count == 0 {
            return Err(WorkbenchError::new("incomplete HTTP request"));
        }
        bytes.extend_from_slice(&chunk[..count]);
        if let Some(index) = find_bytes(&bytes, b"\r\n\r\n") {
            break index + 4;
        }
    };
    let header = std::str::from_utf8(&bytes[..header_end])
        .map_err(|_| WorkbenchError::new("HTTP header is not UTF-8"))?;
    let mut lines = header.split("\r\n");
    let mut request_line = lines
        .next()
        .ok_or_else(|| WorkbenchError::new("missing HTTP request line"))?
        .split_whitespace();
    let method = request_line.next().unwrap_or_default().to_string();
    let path = request_line.next().unwrap_or_default().to_string();
    if request_line.next() != Some("HTTP/1.1") || request_line.next().is_some() {
        return Err(WorkbenchError::new("invalid HTTP/1.1 request line"));
    }
    let mut content_length = 0_usize;
    let mut origin = None;
    for line in lines.filter(|line| !line.is_empty()) {
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| WorkbenchError::new("malformed HTTP header"))?;
        match name.trim().to_ascii_lowercase().as_str() {
            "content-length" => {
                content_length = value
                    .trim()
                    .parse()
                    .map_err(|_| WorkbenchError::new("invalid Content-Length"))?;
            }
            "origin" => origin = Some(value.trim().to_string()),
            _ => {}
        }
    }
    if content_length > MAX_HTTP_BODY {
        return Err(WorkbenchError::new("HTTP body is too large"));
    }
    while bytes.len() - header_end < content_length {
        let mut chunk = [0_u8; 4096];
        let count = stream
            .read(&mut chunk)
            .map_err(|error| WorkbenchError::new(format!("cannot read HTTP body: {error}")))?;
        if count == 0 {
            return Err(WorkbenchError::new("incomplete HTTP body"));
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
    Ok(HttpRequest {
        method,
        path,
        origin,
        body: bytes[header_end..header_end + content_length].to_vec(),
    })
}

fn origin_allowed(origin: Option<&str>, server: SocketAddr) -> bool {
    let Some(origin) = origin else {
        return true;
    };
    let port = server.port();
    let allowed = match server.ip() {
        IpAddr::V4(ip) => vec![
            format!("http://{ip}:{port}"),
            format!("http://localhost:{port}"),
        ],
        IpAddr::V6(ip) => vec![
            format!("http://[{ip}]:{port}"),
            format!("http://localhost:{port}"),
        ],
    };
    allowed.iter().any(|candidate| candidate == origin)
}

fn json_response(value: &impl Serialize) -> Result<HttpResponse, WorkbenchError> {
    Ok(HttpResponse {
        status: 200,
        reason: "OK",
        content_type: "application/json; charset=utf-8",
        body: serde_json::to_vec(value).map_err(|error| WorkbenchError::new(error.to_string()))?,
    })
}

fn html_response(body: &'static [u8]) -> HttpResponse {
    HttpResponse {
        status: 200,
        reason: "OK",
        content_type: "text/html; charset=utf-8",
        body: body.to_vec(),
    }
}

fn json_error(status: u16, reason: &'static str, message: &str) -> HttpResponse {
    #[derive(Serialize)]
    struct ErrorBody<'a> {
        error: &'a str,
    }
    HttpResponse {
        status,
        reason,
        content_type: "application/json; charset=utf-8",
        body: serde_json::to_vec(&ErrorBody { error: message }).unwrap_or_default(),
    }
}

fn write_http_response(stream: &mut TcpStream, response: HttpResponse) -> std::io::Result<()> {
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    write!(
        stream,
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
        response.status,
        response.reason,
        response.content_type,
        response.body.len()
    )?;
    stream.write_all(&response.body)
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tape::{InputFrame, RawPadState};

    fn temporary_root(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("huntctl-workbench-{name}-{nonce}"));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn write_tape(root: &Path, name: &str, values: &[i8]) {
        let tape = InputTape {
            frames: values
                .iter()
                .map(|value| InputFrame {
                    owned_ports: 0x0f,
                    pads: [
                        RawPadState {
                            stick_x: *value,
                            ..RawPadState::default()
                        },
                        RawPadState::default(),
                        RawPadState::default(),
                        RawPadState::default(),
                    ],
                    ..InputFrame::default()
                })
                .collect(),
            ..InputTape::default()
        };
        fs::write(root.join(name), tape.encode().unwrap()).unwrap();
    }

    fn timeline() -> Timeline {
        Timeline::parse(
            r#"
timeline test
milestone boot
milestone control
milestone exit
segment boot_link from boot to control profile boot_to_fsp103
segment link_exit from control to exit profile fsp103_to_fsp104
variant boot_link.one incumbent uses tape first.tape starts clean produces aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa ticks 2
variant link_exit.one incumbent uses tape second.tape starts aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa produces bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb ticks 1
continuation main starts root@clean
continue main with boot_link.one after root@clean
continue main with link_exit.one after boot_link.one@aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
"#,
        )
        .unwrap()
    }

    #[test]
    fn graph_exposes_timeline_shape_and_scrub_ranges() {
        let root = temporary_root("graph");
        write_tape(&root, "first.tape", &[1, 2, 3, 4]);
        write_tape(&root, "second.tape", &[5, 6, 7]);
        let graph = graph_from_timeline(&timeline(), &root).unwrap();
        assert_eq!(graph.schema, "dusklight.route-workbench.graph.v2");
        assert_eq!(graph.milestones.len(), 3);
        assert_eq!(graph.segments.len(), 2);
        assert!(graph.variants.iter().all(|variant| variant.playable));
        assert!(graph.variants.iter().all(|variant| variant.recordable));
        assert_eq!(graph.lineages[0].frame_count, Some(7));
        assert_eq!(graph.lineages[0].steps[1].chain_start_frame, Some(4));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn materializes_milestone_and_inclusive_segment_frame() {
        let root = temporary_root("materialize");
        write_tape(&root, "first.tape", &[1, 2, 3, 4]);
        write_tape(&root, "second.tape", &[5, 6, 7]);
        let route = timeline();
        let milestone = materialize_lineage(
            &route,
            &root,
            "main",
            MaterializeTarget::ThroughMilestone("control".into()),
        )
        .unwrap();
        assert_eq!(milestone.tape.frames.len(), 4);
        let scrubbed = materialize_lineage(
            &route,
            &root,
            "main",
            MaterializeTarget::ThroughSegmentFrame {
                segment: "link_exit".into(),
                frame: 0,
            },
        )
        .unwrap();
        assert_eq!(scrubbed.tape.frames.len(), 5);
        assert_eq!(scrubbed.tape.frames[4].pads[0].stick_x, 5);
        assert_eq!(scrubbed.steps[1].chain_start_frame, 4);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_selectors_off_lineage_and_artifact_escape() {
        let root = temporary_root("guardrails");
        write_tape(&root, "first.tape", &[1, 2, 3, 4]);
        write_tape(&root, "second.tape", &[5, 6, 7]);
        let error = materialize_lineage(
            &timeline(),
            &root,
            "main",
            MaterializeTarget::ThroughMilestone("missing".into()),
        )
        .unwrap_err();
        assert!(error.to_string().contains("not on lineage"));
        assert!(checked_artifact_path(&root, Path::new("../outside.tape")).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn play_request_requires_unambiguous_scrub_target() {
        let request = PlayRequest {
            lineage: Some("main".into()),
            variant: None,
            through_milestone: Some("control".into()),
            segment: Some("boot_link".into()),
            frame: Some(1),
            takeover: true,
        };
        assert!(validate_play_request(&request).is_err());
    }

    #[test]
    fn origin_policy_is_same_origin_or_non_browser() {
        let address: SocketAddr = "127.0.0.1:43123".parse().unwrap();
        assert!(origin_allowed(None, address));
        assert!(origin_allowed(Some("http://127.0.0.1:43123"), address));
        assert!(origin_allowed(Some("http://localhost:43123"), address));
        assert!(!origin_allowed(Some("https://hostile.example"), address));
    }

    #[test]
    fn browser_segment_selects_incumbent_and_uses_local_tape_frame() {
        let request = BrowserPlayRequest {
            selection: BrowserSelection::Segment {
                id: "link_exit".into(),
            },
            stop: BrowserStop::Tick { tick: 1 },
            handoff: true,
        };
        let adapted = adapt_browser_request(&timeline(), request).unwrap();
        assert_eq!(adapted.variant.as_deref(), Some("link_exit.one"));
        assert_eq!(adapted.frame, Some(1));
        assert!(adapted.takeover);
    }

    #[test]
    fn compiles_checked_in_tas_artifacts() {
        let root = temporary_root("tas");
        fs::write(
            root.join("boot.tas"),
            "dusktape 1\nrate 30/1\nports 0x0f\nstate neutral {}\nframe neutral\n",
        )
        .unwrap();
        let route = Timeline::parse(
            r#"
timeline tas
milestone boot
milestone control
segment boot_link from boot to control profile boot_to_fsp103
variant boot_link.tas incumbent uses tas boot.tas starts clean produces control
continuation main starts root@clean
continue main with boot_link.tas after root@clean
"#,
        )
        .unwrap();
        let tape = load_variant_tape(&route, &route.variants["boot_link.tas"], &root).unwrap();
        assert_eq!(tape.frames.len(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn checked_in_intro_exposes_exact_main_record_anchor() {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        let timeline_path = repository.join("routes/intro.timeline");
        let route = load_authoritative_timeline(&timeline_path).unwrap();
        let graph = graph_from_timeline(&route, timeline_path.parent().unwrap()).unwrap();
        let variant = graph
            .variants
            .iter()
            .find(|variant| variant.id == "boot_to_link.golf439")
            .unwrap();
        assert!(variant.recordable);
        assert_eq!(variant.record_anchors.len(), 1);
        assert_eq!(variant.record_anchors[0].lineage, "main");
        assert_eq!(variant.record_anchors[0].step_index, 0);
        assert_eq!(variant.record_anchors[0].terminal_milestone, "link_control");
    }

    fn install_ready_draft(
        repository_root: &Path,
        state_root: &Path,
        id: &str,
        values: &[i8],
    ) -> DraftManifest {
        let route = timeline();
        let parent = materialize_lineage(
            &route,
            repository_root,
            "main",
            MaterializeTarget::ThroughMilestone("exit".into()),
        )
        .unwrap()
        .tape;
        let continuation = InputTape {
            frames: values
                .iter()
                .map(|value| InputFrame {
                    owned_ports: 0x0f,
                    pads: [
                        RawPadState {
                            stick_x: *value,
                            ..RawPadState::default()
                        },
                        RawPadState::default(),
                        RawPadState::default(),
                        RawPadState::default(),
                    ],
                    ..InputFrame::default()
                })
                .collect(),
            ..InputTape::default()
        };
        let continuation_bytes = continuation.encode().unwrap();
        let result = concatenate(vec![
            ChainSegment::all(parent.clone()),
            ChainSegment::all(continuation),
        ])
        .unwrap()
        .tape;
        let directory = drafts_root(state_root).unwrap().join(id);
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join(DRAFT_TAPE), &continuation_bytes).unwrap();
        fs::write(directory.join("playback.tape"), parent.encode().unwrap()).unwrap();
        let manifest = DraftManifest {
            schema: DRAFT_SCHEMA.into(),
            id: id.into(),
            label: "Test branch".into(),
            parent: DraftParent::Variant {
                id: "link_exit.one".into(),
                lineage: "main".into(),
                step_index: 1,
                terminal_milestone: "exit".into(),
                boundary_fingerprint: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            parent_tape_sha256: tape_digest(&parent).unwrap(),
            created_unix_ms: 1,
            session_token: "00112233445566778899aabbccddeeff".into(),
            expected_start_milestone: Some("entered-f-sp104".into()),
            expected_start_fingerprint: Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()),
            tape: DRAFT_TAPE.into(),
            status: DraftStatus::Ready,
            endpoint_kind: "manual_stop".into(),
            verification: "unverified".into(),
            start_boundary_verified: true,
            parent_frames: parent.frames.len() as u64,
            tape_sha256: Some(format!("{:x}", Sha256::digest(&continuation_bytes))),
            tape_bytes: Some(continuation_bytes.len() as u64),
            result_tape_sha256: Some(tape_digest(&result).unwrap()),
            frames: Some(values.len() as u64),
            error: None,
        };
        write_draft_manifest(&directory, &manifest, true).unwrap();
        manifest
    }

    fn write_success_status(directory: &Path, manifest: &DraftManifest, frame_count: u64) {
        let status = serde_json::json!({
            "schema": "dusklight.input-recording/v2",
            "status": "success",
            "tape": fs::canonicalize(directory.join(DRAFT_TAPE)).unwrap(),
            "frame_count": frame_count,
            "frame_capacity": 1080000,
            "handoff_reached": true,
            "capacity_exhausted": false,
            "error": null,
            "process_success": true,
            "session_token": manifest.session_token,
            "start_milestone": manifest.expected_start_milestone,
            "start_fingerprint": manifest.expected_start_fingerprint,
            "start_tape_frame": manifest.parent_frames - 1
        });
        fs::write(
            directory.join(format!("{DRAFT_TAPE}.status.json")),
            serde_json::to_vec(&status).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn draft_suffix_composes_after_exact_two_segment_lineage() {
        let root = temporary_root("draft-chain");
        write_tape(&root, "first.tape", &[1, 2, 3, 4]);
        write_tape(&root, "second.tape", &[5, 6, 7]);
        let state = root.join("state");
        let id = "draft-test-chain";
        install_ready_draft(&root, &state, id, &[8, 9]);
        let materialized = materialize_draft(&timeline(), &root, &state, id).unwrap();
        assert_eq!(
            materialized
                .tape
                .frames
                .iter()
                .map(|frame| frame.pads[0].stick_x)
                .collect::<Vec<_>>(),
            [1, 2, 3, 4, 5, 6, 7, 8, 9]
        );
        let graph = graph_with_drafts(&timeline(), &root, &state).unwrap();
        assert!(graph.drafts[0].playable);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn tampered_continuation_is_neither_playable_nor_loadable() {
        let root = temporary_root("draft-tamper");
        write_tape(&root, "first.tape", &[1, 2, 3, 4]);
        write_tape(&root, "second.tape", &[5, 6, 7]);
        let state = root.join("state");
        let id = "draft-test-tamper";
        install_ready_draft(&root, &state, id, &[8, 9]);
        let tape = drafts_root(&state).unwrap().join(id).join(DRAFT_TAPE);
        let mut bytes = fs::read(&tape).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        fs::write(&tape, bytes).unwrap();
        let graph = graph_with_drafts(&timeline(), &root, &state).unwrap();
        assert!(!graph.drafts[0].playable);
        assert!(
            graph.drafts[0]
                .error
                .as_deref()
                .unwrap()
                .contains("tampered")
        );
        assert!(materialize_draft(&timeline(), &root, &state, id).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn capacity_exhausted_draft_is_visible_but_not_branchable() {
        let root = temporary_root("draft-capacity");
        write_tape(&root, "first.tape", &[1, 2, 3, 4]);
        write_tape(&root, "second.tape", &[5, 6, 7]);
        let state = root.join("state");
        let id = "draft-test-capacity";
        let mut manifest = install_ready_draft(&root, &state, id, &[8]);
        manifest.status = DraftStatus::CapacityExhausted;
        let directory = drafts_root(&state).unwrap().join(id);
        fs::remove_file(directory.join(DRAFT_FINAL_MANIFEST)).unwrap();
        write_draft_manifest(&directory, &manifest, true).unwrap();
        let graph = graph_with_drafts(&timeline(), &root, &state).unwrap();
        assert!(!graph.drafts[0].playable);
        assert!(materialize_draft(&timeline(), &root, &state, id).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn native_status_token_and_start_frame_must_match_before_ready() {
        let root = temporary_root("draft-status");
        write_tape(&root, "first.tape", &[1, 2, 3, 4]);
        write_tape(&root, "second.tape", &[5, 6, 7]);
        let state = root.join("state");
        let id = "draft-test-status";
        let mut manifest = install_ready_draft(&root, &state, id, &[8, 9]);
        let directory = drafts_root(&state).unwrap().join(id);
        manifest.status = DraftStatus::Recording;
        manifest.start_boundary_verified = false;
        manifest.tape_sha256 = None;
        manifest.tape_bytes = None;
        manifest.result_tape_sha256 = None;
        let status_path = directory.join(format!("{DRAFT_TAPE}.status.json"));
        let status = |token: &str, frame: u64| {
            serde_json::json!({
                "schema": "dusklight.input-recording/v2",
                "status": "success",
                "tape": fs::canonicalize(directory.join(DRAFT_TAPE)).unwrap(),
                "frame_count": 2,
                "frame_capacity": 1080000,
                "handoff_reached": true,
                "capacity_exhausted": false,
                "error": null,
                "process_success": true,
                "session_token": token,
                "start_milestone": "entered-f-sp104",
                "start_fingerprint": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "start_tape_frame": frame
            })
        };

        fs::write(
            &status_path,
            serde_json::to_vec(&status("ffffffffffffffffffffffffffffffff", 6)).unwrap(),
        )
        .unwrap();
        let mut rejected = manifest.clone();
        finalize_recording(&directory, &mut rejected, Some(true));
        assert_eq!(rejected.status, DraftStatus::ProcessFailure);

        fs::write(
            &status_path,
            serde_json::to_vec(&status(&manifest.session_token, 5)).unwrap(),
        )
        .unwrap();
        let mut wrong_frame = manifest.clone();
        finalize_recording(&directory, &mut wrong_frame, Some(true));
        assert_eq!(wrong_frame.status, DraftStatus::ProcessFailure);

        let mut process_failed_status = status(&manifest.session_token, 6);
        process_failed_status["process_success"] = serde_json::json!(false);
        fs::write(
            &status_path,
            serde_json::to_vec(&process_failed_status).unwrap(),
        )
        .unwrap();
        let mut native_failed = manifest.clone();
        finalize_recording(&directory, &mut native_failed, None);
        assert_eq!(native_failed.status, DraftStatus::ProcessFailure);

        fs::write(
            &status_path,
            serde_json::to_vec(&status(&manifest.session_token, 6)).unwrap(),
        )
        .unwrap();
        let mut exit_disagreed = manifest.clone();
        finalize_recording(&directory, &mut exit_disagreed, Some(false));
        assert_eq!(exit_disagreed.status, DraftStatus::ProcessFailure);

        finalize_recording(&directory, &mut manifest, Some(true));
        assert_eq!(manifest.status, DraftStatus::Ready);
        assert!(manifest.start_boundary_verified);
        assert!(manifest.tape_sha256.as_deref().is_some_and(valid_sha256));
        assert!(
            manifest
                .result_tape_sha256
                .as_deref()
                .is_some_and(valid_sha256)
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cyclic_ready_drafts_are_structurally_nonplayable() {
        let root = temporary_root("draft-cycle");
        write_tape(&root, "first.tape", &[1, 2, 3, 4]);
        write_tape(&root, "second.tape", &[5, 6, 7]);
        let state = root.join("state");
        let mut left = install_ready_draft(&root, &state, "draft-cycle-left", &[8]);
        let mut right = install_ready_draft(&root, &state, "draft-cycle-right", &[9]);
        left.parent = DraftParent::Draft {
            id: right.id.clone(),
            parent_tape_sha256: right.result_tape_sha256.clone().unwrap(),
        };
        left.parent_tape_sha256 = right.result_tape_sha256.clone().unwrap();
        right.parent = DraftParent::Draft {
            id: left.id.clone(),
            parent_tape_sha256: left.result_tape_sha256.clone().unwrap(),
        };
        right.parent_tape_sha256 = left.result_tape_sha256.clone().unwrap();
        for manifest in [&left, &right] {
            let directory = drafts_root(&state).unwrap().join(&manifest.id);
            fs::remove_file(directory.join(DRAFT_FINAL_MANIFEST)).unwrap();
            write_draft_manifest(&directory, manifest, true).unwrap();
        }
        let graph = graph_with_drafts(&timeline(), &root, &state).unwrap();
        assert_eq!(graph.drafts.len(), 2);
        assert!(graph.drafts.iter().all(|draft| !draft.playable));
        assert!(
            graph
                .drafts
                .iter()
                .all(|draft| draft.error.as_deref().unwrap().contains("cycle"))
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_and_nonready_draft_parents_block_children() {
        let root = temporary_root("draft-parent-state");
        write_tape(&root, "first.tape", &[1, 2, 3, 4]);
        write_tape(&root, "second.tape", &[5, 6, 7]);
        let state = root.join("state");
        let mut missing = install_ready_draft(&root, &state, "draft-missing-child", &[8]);
        missing.parent = DraftParent::Draft {
            id: "draft-does-not-exist".into(),
            parent_tape_sha256: "11".repeat(32),
        };
        missing.parent_tape_sha256 = "11".repeat(32);

        let mut parent = install_ready_draft(&root, &state, "draft-nonready-parent", &[9]);
        parent.status = DraftStatus::CapacityExhausted;
        let mut child = install_ready_draft(&root, &state, "draft-nonready-child", &[10]);
        child.parent = DraftParent::Draft {
            id: parent.id.clone(),
            parent_tape_sha256: parent.result_tape_sha256.clone().unwrap(),
        };
        child.parent_tape_sha256 = parent.result_tape_sha256.clone().unwrap();

        for manifest in [&missing, &parent, &child] {
            let directory = drafts_root(&state).unwrap().join(&manifest.id);
            fs::remove_file(directory.join(DRAFT_FINAL_MANIFEST)).unwrap();
            write_draft_manifest(&directory, manifest, true).unwrap();
        }
        let graph = graph_with_drafts(&timeline(), &root, &state).unwrap();
        let by_id = graph
            .drafts
            .iter()
            .map(|draft| (draft.id.as_str(), draft))
            .collect::<BTreeMap<_, _>>();
        assert!(!by_id["draft-missing-child"].playable);
        assert!(
            by_id["draft-missing-child"]
                .error
                .as_deref()
                .unwrap()
                .contains("missing")
        );
        assert!(!by_id["draft-nonready-parent"].playable);
        assert!(!by_id["draft-nonready-child"].playable);
        assert!(
            by_id["draft-nonready-child"]
                .error
                .as_deref()
                .unwrap()
                .contains("not ready")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn interrupted_final_manifest_write_never_exposes_false_ready() {
        let root = temporary_root("draft-interrupted-final");
        write_tape(&root, "first.tape", &[1, 2, 3, 4]);
        write_tape(&root, "second.tape", &[5, 6, 7]);
        let state = root.join("state");
        let id = "draft-interrupted-final";
        let mut manifest = install_ready_draft(&root, &state, id, &[8]);
        let directory = drafts_root(&state).unwrap().join(id);
        fs::remove_file(directory.join(DRAFT_FINAL_MANIFEST)).unwrap();
        manifest.status = DraftStatus::Recording;
        manifest.tape_sha256 = None;
        manifest.tape_bytes = None;
        manifest.result_tape_sha256 = None;
        write_draft_manifest(&directory, &manifest, false).unwrap();
        fs::write(directory.join(".draft-interrupted.tmp"), b"{\"status\":").unwrap();
        let graph = graph_with_drafts(&timeline(), &root, &state).unwrap();
        assert_eq!(graph.drafts.len(), 1);
        assert_eq!(graph.drafts[0].status, DraftStatus::Orphaned);
        assert!(!graph.drafts[0].playable);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn orphaned_descriptor_recovers_from_late_token_bound_status() {
        let root = temporary_root("draft-orphan-recovery");
        write_tape(&root, "first.tape", &[1, 2, 3, 4]);
        write_tape(&root, "second.tape", &[5, 6, 7]);
        let state = root.join("state");
        let id = "draft-orphan-recovery";
        let mut manifest = install_ready_draft(&root, &state, id, &[8]);
        let directory = drafts_root(&state).unwrap().join(id);
        fs::remove_file(directory.join(DRAFT_FINAL_MANIFEST)).unwrap();
        manifest.status = DraftStatus::Preparing;
        manifest.start_boundary_verified = false;
        manifest.tape_sha256 = None;
        manifest.tape_bytes = None;
        manifest.result_tape_sha256 = None;
        write_draft_manifest(&directory, &manifest, false).unwrap();

        let orphaned = graph_with_drafts(&timeline(), &root, &state).unwrap();
        assert_eq!(orphaned.drafts[0].status, DraftStatus::Orphaned);
        assert!(!directory.join(DRAFT_FINAL_MANIFEST).exists());

        write_success_status(&directory, &manifest, 1);
        let recovered = graph_with_drafts(&timeline(), &root, &state).unwrap();
        assert_eq!(recovered.drafts[0].status, DraftStatus::Ready);
        assert!(recovered.drafts[0].playable);
        assert!(directory.join(DRAFT_FINAL_MANIFEST).is_file());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn graph_poll_does_not_promote_while_recording_process_is_live() {
        let root = temporary_root("draft-live-race");
        write_tape(&root, "first.tape", &[1, 2, 3, 4]);
        write_tape(&root, "second.tape", &[5, 6, 7]);
        let state = root.join("state");
        let id = "draft-live-race";
        let mut manifest = install_ready_draft(&root, &state, id, &[8]);
        let directory = drafts_root(&state).unwrap().join(id);
        fs::remove_file(directory.join(DRAFT_FINAL_MANIFEST)).unwrap();
        manifest.status = DraftStatus::Preparing;
        manifest.start_boundary_verified = false;
        manifest.tape_sha256 = None;
        manifest.tape_bytes = None;
        manifest.result_tape_sha256 = None;
        write_draft_manifest(&directory, &manifest, false).unwrap();
        write_draft_launch(
            &directory,
            &DraftLaunch {
                schema: "dusklight.route-workbench.launch.v1".into(),
                id: id.into(),
                pid: std::process::id(),
                session_token: manifest.session_token.clone(),
            },
        )
        .unwrap();
        write_success_status(&directory, &manifest, 1);

        let live = graph_with_drafts(&timeline(), &root, &state).unwrap();
        assert_eq!(live.drafts[0].status, DraftStatus::Recording);
        assert!(!live.drafts[0].playable);
        assert!(!directory.join(DRAFT_FINAL_MANIFEST).exists());

        fs::remove_file(directory.join(DRAFT_LAUNCH)).unwrap();
        write_draft_launch(
            &directory,
            &DraftLaunch {
                schema: "dusklight.route-workbench.launch.v1".into(),
                id: id.into(),
                pid: u32::MAX,
                session_token: manifest.session_token.clone(),
            },
        )
        .unwrap();
        active_recordings().lock().unwrap().insert(id.into());
        let awaiting_monitor = graph_with_drafts(&timeline(), &root, &state).unwrap();
        assert_eq!(awaiting_monitor.drafts[0].status, DraftStatus::Recording);
        assert!(!directory.join(DRAFT_FINAL_MANIFEST).exists());
        active_recordings().lock().unwrap().remove(id);
        let exited = graph_with_drafts(&timeline(), &root, &state).unwrap();
        assert_eq!(exited.drafts[0].status, DraftStatus::Ready);
        assert!(exited.drafts[0].playable);
        fs::remove_dir_all(root).unwrap();
    }
}
