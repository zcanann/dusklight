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
use std::error::Error;
use std::fmt;
use std::fs;
use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const GRAPH_SCHEMA: &str = "dusklight.route-workbench.graph.v1";
const MAX_HTTP_HEADER: usize = 64 * 1024;
const MAX_HTTP_BODY: usize = 1024 * 1024;

#[derive(Clone, Debug, Serialize)]
pub struct WorkbenchGraph {
    pub schema: String,
    pub timeline: String,
    pub milestones: Vec<GraphMilestone>,
    pub segments: Vec<GraphSegment>,
    pub variants: Vec<GraphVariant>,
    pub lineages: Vec<GraphLineage>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
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
    Segment { id: String },
    Milestone { id: String },
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
    })
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
    let end = if request.takeover { "release" } else { "hold" };
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
                        graph_from_timeline(&timeline, &artifact_root)
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
                            let play_request = adapt_browser_request(&timeline, browser_request)?;
                            let (response, _child) = play(&timeline, config, &play_request)?;
                            Ok(response)
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
variant boot_link.one incumbent uses tape first.tape starts clean produces control-a ticks 2
variant link_exit.one incumbent uses tape second.tape starts control-a produces exit-a ticks 1
continuation main starts root@clean
continue main with boot_link.one after root@clean
continue main with link_exit.one after boot_link.one@control-a
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
        assert_eq!(graph.milestones.len(), 3);
        assert_eq!(graph.segments.len(), 2);
        assert!(graph.variants.iter().all(|variant| variant.playable));
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
}
