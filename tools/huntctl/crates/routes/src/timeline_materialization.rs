//! Exact materialization of immutable timeline segment ancestry.

use crate::tape::InputTape;
use crate::timeline::{ArtifactSource, Segment, Timeline};
use dusklight_control::tape_chain::{ChainSegment, concatenate};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaterializedSegmentChain {
    pub tape: InputTape,
    pub steps: Vec<MaterializedSegmentStep>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaterializedSegmentStep {
    pub segment: String,
    pub chain_start_frame: u64,
    pub chain_end_frame: u64,
}

/// Materializes the unique Boot-rooted ancestry of a canonical timeline segment.
///
/// Baseline and candidate sources are evaluation programs rather than immutable
/// continuation payloads, so they are deliberately rejected here.
pub fn materialize_segment_chain(
    timeline: &Timeline,
    artifact_root: &Path,
    segment_id: &str,
) -> Result<MaterializedSegmentChain, TimelineMaterializationError> {
    let mut reversed = Vec::new();
    let mut cursor = segment_id;
    let mut seen = BTreeSet::new();
    loop {
        if !seen.insert(cursor.to_owned()) {
            return Err(materialization_error(format!(
                "segment ancestry contains a cycle at {cursor:?}"
            )));
        }
        let segment = timeline
            .segments
            .get(cursor)
            .ok_or_else(|| materialization_error(format!("unknown segment {cursor:?}")))?;
        validate_segment_identity(segment)?;
        reversed.push(segment);
        let Some(parent_id) = segment.parent.as_deref() else {
            break;
        };
        let parent = timeline.segments.get(parent_id).ok_or_else(|| {
            materialization_error(format!(
                "segment {:?} references missing parent {parent_id:?}",
                segment.id
            ))
        })?;
        if parent.end_fingerprint != segment.start_fingerprint {
            return Err(materialization_error(format!(
                "segment {:?} starts at {}, but parent {parent_id:?} ends at {}",
                segment.id, segment.start_fingerprint, parent.end_fingerprint
            )));
        }
        cursor = parent_id;
    }
    reversed.reverse();

    let chain = reversed
        .iter()
        .map(|segment| {
            Ok(
                ChainSegment::all(load_segment_tape(segment, artifact_root)?)
                    .named(segment.id.clone()),
            )
        })
        .collect::<Result<Vec<_>, TimelineMaterializationError>>()?;
    let chained = concatenate(chain).map_err(materialization_error)?;
    let steps = chained
        .segments
        .iter()
        .map(|boundary| MaterializedSegmentStep {
            segment: boundary
                .segment_name
                .clone()
                .expect("timeline materialization names every segment"),
            chain_start_frame: boundary.chain_start_tick,
            chain_end_frame: boundary.chain_end_tick,
        })
        .collect();
    Ok(MaterializedSegmentChain {
        tape: chained.tape,
        steps,
    })
}

fn validate_segment_identity(segment: &Segment) -> Result<(), TimelineMaterializationError> {
    if !matches!(
        segment.artifact,
        ArtifactSource::Tas(_) | ArtifactSource::Tape(_)
    ) {
        return Err(materialization_error(format!(
            "segment {} is not an immutable canonical continuation payload",
            segment.id
        )));
    }
    if placeholder(&segment.start_fingerprint) || placeholder(&segment.end_fingerprint) {
        return Err(materialization_error(format!(
            "segment {} uses placeholder fingerprints",
            segment.id
        )));
    }
    Ok(())
}

fn load_segment_tape(
    segment: &Segment,
    artifact_root: &Path,
) -> Result<InputTape, TimelineMaterializationError> {
    match &segment.artifact {
        ArtifactSource::Tas(relative) => {
            let path = checked_artifact_path(artifact_root, relative)?;
            let source = fs::read_to_string(&path).map_err(materialization_error)?;
            crate::tape_dsl::parse(&source)
                .map_err(materialization_error)?
                .compile()
                .map(|compiled| compiled.tape)
                .map_err(materialization_error)
        }
        ArtifactSource::Tape(relative) => {
            let path = checked_artifact_path(artifact_root, relative)?;
            InputTape::decode(&fs::read(path).map_err(materialization_error)?)
                .map(|decoded| decoded.tape)
                .map_err(materialization_error)
        }
        ArtifactSource::Candidate(_) => Err(materialization_error(format!(
            "segment {} candidate is not a canonical continuation payload",
            segment.id
        ))),
        ArtifactSource::Baseline(_) => Err(materialization_error(format!(
            "segment {} baseline is not a canonical continuation payload",
            segment.id
        ))),
    }
}

fn checked_artifact_path(
    root: &Path,
    relative: &Path,
) -> Result<PathBuf, TimelineMaterializationError> {
    if relative.is_absolute() {
        return Err(materialization_error("timeline artifact path is absolute"));
    }
    let root = root.canonicalize().map_err(materialization_error)?;
    let path = root
        .join(relative)
        .canonicalize()
        .map_err(materialization_error)?;
    if !path.starts_with(&root) || !path.is_file() {
        return Err(materialization_error(
            "timeline artifact is outside its root or is not a file",
        ));
    }
    Ok(path)
}

fn placeholder(value: &str) -> bool {
    value.trim().is_empty() || value.to_ascii_lowercase().contains("unknown")
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimelineMaterializationError(String);

fn materialization_error(error: impl fmt::Display) -> TimelineMaterializationError {
    TimelineMaterializationError(error.to_string())
}

impl fmt::Display for TimelineMaterializationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for TimelineMaterializationError {}
