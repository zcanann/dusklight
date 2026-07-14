//! Content-addressed authority for immutable route artifacts and named heads.

use crate::search::Candidate;
use crate::tape::InputTape;
use crate::timeline::{ArtifactSource, ResolvedLineage, Timeline, TimelineError};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::error::Error;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const ROUTE_OBJECT_SCHEMA: &str = "dusklight-route-object/v1";

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct StoredObject {
    pub schema: String,
    #[serde(flatten)]
    pub value: RouteObject,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "object_type", rename_all = "snake_case")]
pub enum RouteObject {
    Program {
        format: String,
        source: String,
    },
    Tape {
        bytes_hex: String,
    },
    Boundary {
        milestone: String,
        fingerprint: String,
    },
    Evaluation {
        variant: String,
        artifact: ObjectId,
        boundary: ObjectId,
        success: bool,
        first_hit_tick: Option<u64>,
        raw: serde_json::Value,
    },
    Lineage {
        timeline: String,
        name: String,
        parent_lineage: Option<ObjectId>,
        root_fingerprint: String,
        steps: Vec<StoredLineageStep>,
    },
    Snapshot {
        timeline: String,
        lineages: BTreeMap<String, ObjectId>,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct StoredLineageStep {
    pub variant: String,
    pub program: Option<ObjectId>,
    pub tape: ObjectId,
    pub start_boundary: ObjectId,
    pub end_boundary: ObjectId,
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ObjectId(pub String);

impl ObjectId {
    fn parse(value: impl Into<String>) -> Result<Self, StoreError> {
        let value = value.into();
        if value.len() != 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(StoreError::InvalidObjectId(value));
        }
        Ok(Self(value))
    }
}

impl fmt::Display for ObjectId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ImportResult {
    pub snapshot: ObjectId,
    pub reference: String,
    pub lineages: BTreeMap<String, ObjectId>,
}

#[derive(Clone, Debug, Serialize)]
pub struct GcReport {
    pub reachable: usize,
    pub unreachable: Vec<ObjectId>,
    pub removed: usize,
}

#[derive(Clone, Debug)]
pub struct RouteStore {
    root: PathBuf,
}

impl RouteStore {
    pub fn initialize(root: impl Into<PathBuf>) -> Result<Self, StoreError> {
        let store = Self { root: root.into() };
        fs::create_dir_all(store.objects_dir())?;
        fs::create_dir_all(store.refs_dir())?;
        fs::create_dir_all(store.root.join("tmp"))?;
        Ok(store)
    }

    pub fn open(root: impl Into<PathBuf>) -> Result<Self, StoreError> {
        let store = Self { root: root.into() };
        if !store.objects_dir().is_dir() || !store.refs_dir().is_dir() {
            return Err(StoreError::NotInitialized(store.root));
        }
        Ok(store)
    }

    pub fn put(&self, value: RouteObject) -> Result<ObjectId, StoreError> {
        let object = StoredObject {
            schema: ROUTE_OBJECT_SCHEMA.into(),
            value,
        };
        let bytes = serde_json::to_vec(&object)?;
        let id = ObjectId(format!("{:x}", Sha256::digest(&bytes)));
        let destination = self.object_path(&id);
        if destination.exists() {
            self.read(&id)?;
            return Ok(id);
        }
        let temporary = self
            .root
            .join("tmp")
            .join(format!("{}-{}.tmp", id.0, std::process::id()));
        {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
        }
        match fs::rename(&temporary, &destination) {
            Ok(()) => {}
            Err(_error) if destination.exists() => {
                let _ = fs::remove_file(&temporary);
                self.read(&id)?;
            }
            Err(error) => {
                let _ = fs::remove_file(&temporary);
                return Err(error.into());
            }
        }
        Ok(id)
    }

    pub fn read(&self, id: &ObjectId) -> Result<StoredObject, StoreError> {
        ObjectId::parse(id.0.clone())?;
        let bytes = fs::read(self.object_path(id))?;
        let actual = format!("{:x}", Sha256::digest(&bytes));
        if actual != id.0 {
            return Err(StoreError::HashMismatch {
                expected: id.clone(),
                actual,
            });
        }
        let object: StoredObject = serde_json::from_slice(&bytes)?;
        if object.schema != ROUTE_OBJECT_SCHEMA {
            return Err(StoreError::InvalidSchema(object.schema));
        }
        object.validate()?;
        Ok(object)
    }

    pub fn resolve_ref(&self, name: &str) -> Result<ObjectId, StoreError> {
        let directory = self.ref_dir(name)?;
        let mut events = fs::read_dir(&directory)?.collect::<Result<Vec<_>, _>>()?;
        events.sort_by_key(|event| event.file_name());
        let event = events
            .last()
            .ok_or_else(|| StoreError::UnknownRef(name.into()))?;
        ObjectId::parse(fs::read_to_string(event.path())?.trim())
    }

    /// Ref updates are append-only event creation. Readers choose the largest
    /// sequence, so prior heads remain recoverable and no mutable HEAD file can
    /// be torn.
    pub fn promote(&self, name: &str, id: &ObjectId) -> Result<(), StoreError> {
        let mut reachable = HashSet::new();
        let mut active = HashSet::new();
        self.mark_reachable(id, &mut reachable, &mut active)?;
        let directory = self.ref_dir(name)?;
        fs::create_dir_all(&directory)?;
        loop {
            let next = fs::read_dir(&directory)?.count();
            let event = directory.join(format!("{next:020}.ref"));
            match OpenOptions::new().create_new(true).write(true).open(event) {
                Ok(mut file) => {
                    writeln!(file, "{id}")?;
                    file.sync_all()?;
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    pub fn fork(&self, from: &str, to: &str) -> Result<ObjectId, StoreError> {
        let id = self.resolve_ref(from)?;
        self.promote(to, &id)?;
        Ok(id)
    }

    pub fn fork_lineage(
        &self,
        from: &str,
        lineage: &str,
        to: &str,
    ) -> Result<ObjectId, StoreError> {
        let snapshot = self.resolve_ref(from)?;
        let object = self.read(&snapshot)?;
        let RouteObject::Snapshot { lineages, .. } = object.value else {
            return Err(StoreError::RefNotSnapshot(from.into()));
        };
        let id = lineages
            .get(lineage)
            .cloned()
            .ok_or_else(|| StoreError::UnknownLineage(lineage.into()))?;
        self.promote(to, &id)?;
        Ok(id)
    }

    pub fn import_timeline(
        &self,
        timeline: &Timeline,
        source_root: &Path,
        reference: &str,
    ) -> Result<ImportResult, StoreError> {
        timeline.validate_artifacts(Some(source_root))?;
        let inspection = timeline.inspect()?;
        let artifacts = self.import_variant_artifacts(timeline, source_root)?;
        let boundaries = self.import_boundaries(timeline)?;
        let mut lineages = BTreeMap::new();
        for lineage in &inspection.lineages {
            let id = self.store_lineage(timeline, lineage, None, &artifacts, &boundaries)?;
            lineages.insert(lineage.name.clone(), id);
        }
        let snapshot = self.put(RouteObject::Snapshot {
            timeline: timeline.name.clone(),
            lineages: lineages.clone(),
        })?;
        self.promote(reference, &snapshot)?;
        Ok(ImportResult {
            snapshot,
            reference: reference.into(),
            lineages,
        })
    }

    pub fn import_evaluation(
        &self,
        path: &Path,
        milestone: &str,
        fingerprint: &str,
        reference: Option<&str>,
    ) -> Result<ObjectId, StoreError> {
        let raw: serde_json::Value = serde_json::from_slice(&fs::read(path)?)?;
        if raw["schema_version"].as_u64() != Some(1) {
            return Err(StoreError::InvalidObject(
                "unsupported evaluation schema".into(),
            ));
        }
        let variant = raw["candidate_id"]
            .as_str()
            .ok_or_else(|| StoreError::InvalidObject("evaluation has no candidate_id".into()))?;
        let tape_path = raw["tape"]
            .as_str()
            .ok_or_else(|| StoreError::InvalidObject("evaluation has no tape path".into()))?;
        let tape_path = {
            let tape = PathBuf::from(tape_path);
            if tape.is_absolute() {
                tape
            } else {
                path.parent().unwrap_or_else(|| Path::new(".")).join(tape)
            }
        };
        let tape_bytes = fs::read(&tape_path)?;
        InputTape::decode(&tape_bytes)?;
        let artifact = self.put(RouteObject::Tape {
            bytes_hex: encode_hex(&tape_bytes),
        })?;
        let boundary = self.put(RouteObject::Boundary {
            milestone: milestone.into(),
            fingerprint: fingerprint.into(),
        })?;
        let id = self.put(RouteObject::Evaluation {
            variant: variant.into(),
            artifact,
            boundary,
            success: raw["success"].as_bool().unwrap_or(false),
            first_hit_tick: raw["first_hit_tick"].as_u64(),
            raw,
        })?;
        if let Some(reference) = reference {
            self.promote(reference, &id)?;
        }
        Ok(id)
    }

    pub fn append_lineage(
        &self,
        reference: &str,
        timeline: &Timeline,
        lineage_name: &str,
        source_root: &Path,
    ) -> Result<ObjectId, StoreError> {
        let parent = self.resolve_ref(reference)?;
        if !matches!(self.read(&parent)?.value, RouteObject::Lineage { .. }) {
            return Err(StoreError::RefNotLineage(reference.into()));
        }
        let inspection = timeline.inspect()?;
        let lineage = inspection
            .lineages
            .iter()
            .find(|lineage| lineage.name == lineage_name)
            .ok_or_else(|| StoreError::UnknownLineage(lineage_name.into()))?;
        let artifacts = self.import_variant_artifacts(timeline, source_root)?;
        let boundaries = self.import_boundaries(timeline)?;
        let id = self.store_lineage(timeline, lineage, Some(parent), &artifacts, &boundaries)?;
        self.promote(reference, &id)?;
        Ok(id)
    }

    pub fn replay_repair(
        &self,
        from_reference: &str,
        to_reference: &str,
        timeline: &Timeline,
        lineage_name: &str,
        source_root: &Path,
    ) -> Result<ObjectId, StoreError> {
        let parent = self.resolve_ref(from_reference)?;
        if !matches!(self.read(&parent)?.value, RouteObject::Lineage { .. }) {
            return Err(StoreError::RefNotLineage(from_reference.into()));
        }
        let inspection = timeline.inspect()?;
        let lineage = inspection
            .lineages
            .iter()
            .find(|lineage| lineage.name == lineage_name)
            .ok_or_else(|| StoreError::UnknownLineage(lineage_name.into()))?;
        let artifacts = self.import_variant_artifacts(timeline, source_root)?;
        let boundaries = self.import_boundaries(timeline)?;
        let id = self.store_lineage(timeline, lineage, Some(parent), &artifacts, &boundaries)?;
        self.promote(to_reference, &id)?;
        Ok(id)
    }

    pub fn gc(&self, apply: bool) -> Result<GcReport, StoreError> {
        let mut roots = Vec::new();
        for event in head_ref_files(&self.refs_dir())? {
            roots.push(ObjectId::parse(fs::read_to_string(event)?.trim())?);
        }
        let mut reachable = HashSet::new();
        let mut active = HashSet::new();
        for root in roots {
            self.mark_reachable(&root, &mut reachable, &mut active)?;
        }
        let mut unreachable = Vec::new();
        for entry in fs::read_dir(self.objects_dir())? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let id = ObjectId::parse(entry.file_name().to_string_lossy())?;
            if !reachable.contains(&id) {
                unreachable.push(id);
            }
        }
        unreachable.sort();
        let removed = if apply {
            for id in &unreachable {
                fs::remove_file(self.object_path(id))?;
            }
            unreachable.len()
        } else {
            0
        };
        Ok(GcReport {
            reachable: reachable.len(),
            unreachable,
            removed,
        })
    }

    pub fn verify(&self) -> Result<usize, StoreError> {
        let mut count = 0;
        for entry in fs::read_dir(self.objects_dir())? {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                let id = ObjectId::parse(entry.file_name().to_string_lossy())?;
                self.read(&id)?;
                count += 1;
            }
        }
        for event in head_ref_files(&self.refs_dir())? {
            let id = ObjectId::parse(fs::read_to_string(event)?.trim())?;
            let mut reachable = HashSet::new();
            let mut active = HashSet::new();
            self.mark_reachable(&id, &mut reachable, &mut active)?;
        }
        Ok(count)
    }

    fn import_variant_artifacts(
        &self,
        timeline: &Timeline,
        root: &Path,
    ) -> Result<BTreeMap<String, (Option<ObjectId>, ObjectId)>, StoreError> {
        let mut output = BTreeMap::new();
        for variant in timeline.variants.values() {
            let (program, tape_bytes) = match &variant.artifact {
                ArtifactSource::Baseline(profile) => {
                    let candidate = Candidate::baseline(*profile);
                    (
                        Some(serde_json::to_string_pretty(&candidate)?),
                        candidate.compile()?.encode()?,
                    )
                }
                ArtifactSource::Candidate(path) => {
                    let source = fs::read_to_string(root.join(path))?;
                    let candidate: Candidate = serde_json::from_str(&source)?;
                    (Some(source), candidate.compile()?.encode()?)
                }
                ArtifactSource::Tape(path) => {
                    let bytes = fs::read(root.join(path))?;
                    InputTape::decode(&bytes)?;
                    (None, bytes)
                }
            };
            let program = program
                .map(|source| {
                    self.put(RouteObject::Program {
                        format: "dusklight-search-candidate/v1".into(),
                        source,
                    })
                })
                .transpose()?;
            let tape = self.put(RouteObject::Tape {
                bytes_hex: encode_hex(&tape_bytes),
            })?;
            output.insert(variant.id.clone(), (program, tape));
        }
        Ok(output)
    }

    fn import_boundaries(
        &self,
        timeline: &Timeline,
    ) -> Result<BTreeMap<(String, String), ObjectId>, StoreError> {
        let mut output = BTreeMap::new();
        for variant in timeline.variants.values() {
            let segment = &timeline.segments[&variant.segment];
            for (milestone, fingerprint) in [
                (&segment.from, &variant.start_fingerprint),
                (&segment.to, &variant.boundary_fingerprint),
            ] {
                let key = (milestone.clone(), fingerprint.clone());
                if let std::collections::btree_map::Entry::Vacant(entry) = output.entry(key) {
                    let id = self.put(RouteObject::Boundary {
                        milestone: milestone.clone(),
                        fingerprint: fingerprint.clone(),
                    })?;
                    entry.insert(id);
                }
            }
        }
        Ok(output)
    }

    fn store_lineage(
        &self,
        timeline: &Timeline,
        lineage: &ResolvedLineage,
        parent_lineage: Option<ObjectId>,
        artifacts: &BTreeMap<String, (Option<ObjectId>, ObjectId)>,
        boundaries: &BTreeMap<(String, String), ObjectId>,
    ) -> Result<ObjectId, StoreError> {
        let mut steps = Vec::new();
        for step in &lineage.steps {
            let variant = &timeline.variants[&step.variant];
            let segment = &timeline.segments[&variant.segment];
            let (program, tape) = &artifacts[&variant.id];
            steps.push(StoredLineageStep {
                variant: variant.id.clone(),
                program: program.clone(),
                tape: tape.clone(),
                start_boundary: boundaries
                    [&(segment.from.clone(), variant.start_fingerprint.clone())]
                    .clone(),
                end_boundary: boundaries
                    [&(segment.to.clone(), variant.boundary_fingerprint.clone())]
                    .clone(),
            });
        }
        self.put(RouteObject::Lineage {
            timeline: timeline.name.clone(),
            name: lineage.name.clone(),
            parent_lineage,
            root_fingerprint: lineage.root_fingerprint.clone(),
            steps,
        })
    }

    fn mark_reachable(
        &self,
        id: &ObjectId,
        reachable: &mut HashSet<ObjectId>,
        active: &mut HashSet<ObjectId>,
    ) -> Result<(), StoreError> {
        if reachable.contains(id) {
            return Ok(());
        }
        if !active.insert(id.clone()) {
            return Err(StoreError::ObjectCycle(id.clone()));
        }
        let object = self.read(id)?;
        for reference in object.references() {
            self.mark_reachable(&reference, reachable, active)?;
        }
        active.remove(id);
        reachable.insert(id.clone());
        Ok(())
    }

    fn object_path(&self, id: &ObjectId) -> PathBuf {
        self.objects_dir().join(&id.0)
    }

    fn objects_dir(&self) -> PathBuf {
        self.root.join("objects")
    }

    fn refs_dir(&self) -> PathBuf {
        self.root.join("refs")
    }

    fn ref_dir(&self, name: &str) -> Result<PathBuf, StoreError> {
        let path = Path::new(name);
        if name.is_empty()
            || path.is_absolute()
            || path
                .components()
                .any(|component| !matches!(component, std::path::Component::Normal(_)))
        {
            return Err(StoreError::InvalidRef(name.into()));
        }
        Ok(self.refs_dir().join(path))
    }
}

impl StoredObject {
    fn validate(&self) -> Result<(), StoreError> {
        match &self.value {
            RouteObject::Program { format, .. } if format.is_empty() => {
                Err(StoreError::InvalidObject("empty program format".into()))
            }
            RouteObject::Tape { bytes_hex } => {
                let bytes = decode_hex(bytes_hex)?;
                InputTape::decode(&bytes)?;
                Ok(())
            }
            RouteObject::Boundary {
                milestone,
                fingerprint,
            } if milestone.is_empty() || fingerprint.is_empty() => {
                Err(StoreError::InvalidObject("empty boundary field".into()))
            }
            RouteObject::Lineage { steps, .. } if steps.is_empty() => {
                Err(StoreError::InvalidObject("empty lineage".into()))
            }
            RouteObject::Snapshot { lineages, .. } if lineages.is_empty() => {
                Err(StoreError::InvalidObject("empty snapshot".into()))
            }
            _ => Ok(()),
        }
    }

    fn references(&self) -> Vec<ObjectId> {
        match &self.value {
            RouteObject::Evaluation {
                artifact, boundary, ..
            } => vec![artifact.clone(), boundary.clone()],
            RouteObject::Lineage {
                parent_lineage,
                steps,
                ..
            } => parent_lineage
                .iter()
                .cloned()
                .chain(steps.iter().flat_map(|step| {
                    step.program.iter().cloned().chain([
                        step.tape.clone(),
                        step.start_boundary.clone(),
                        step.end_boundary.clone(),
                    ])
                }))
                .collect(),
            RouteObject::Snapshot { lineages, .. } => lineages.values().cloned().collect(),
            _ => Vec::new(),
        }
    }
}

fn recursive_files(root: &Path) -> Result<Vec<PathBuf>, std::io::Error> {
    let mut output = Vec::new();
    if !root.exists() {
        return Ok(output);
    }
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                pending.push(entry.path());
            } else {
                output.push(entry.path());
            }
        }
    }
    Ok(output)
}

fn head_ref_files(root: &Path) -> Result<Vec<PathBuf>, std::io::Error> {
    let mut heads: BTreeMap<PathBuf, PathBuf> = BTreeMap::new();
    for path in recursive_files(root)? {
        if path.extension().and_then(|value| value.to_str()) != Some("ref") {
            continue;
        }
        let directory = path.parent().unwrap_or(root).to_path_buf();
        let replace = heads
            .get(&directory)
            .is_none_or(|current| path.file_name() > current.file_name());
        if replace {
            heads.insert(directory, path);
        }
    }
    Ok(heads.into_values().collect())
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn decode_hex(value: &str) -> Result<Vec<u8>, StoreError> {
    if !value.len().is_multiple_of(2) {
        return Err(StoreError::InvalidObject("odd-length hex tape".into()));
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let digit = |byte: u8| match byte {
                b'0'..=b'9' => Some(byte - b'0'),
                b'a'..=b'f' => Some(byte - b'a' + 10),
                _ => None,
            };
            let high = digit(pair[0])
                .ok_or_else(|| StoreError::InvalidObject("invalid tape hex".into()))?;
            let low = digit(pair[1])
                .ok_or_else(|| StoreError::InvalidObject("invalid tape hex".into()))?;
            Ok((high << 4) | low)
        })
        .collect()
}

#[derive(Debug)]
pub enum StoreError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Timeline(TimelineError),
    Search(crate::search::SearchError),
    Tape(crate::tape::TapeError),
    NotInitialized(PathBuf),
    InvalidSchema(String),
    InvalidObjectId(String),
    InvalidObject(String),
    InvalidRef(String),
    UnknownRef(String),
    UnknownLineage(String),
    RefNotLineage(String),
    RefNotSnapshot(String),
    HashMismatch { expected: ObjectId, actual: String },
    ObjectCycle(ObjectId),
}

impl fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "route store I/O failed: {error}"),
            Self::Json(error) => write!(formatter, "invalid route store JSON: {error}"),
            Self::Timeline(error) => error.fmt(formatter),
            Self::Search(error) => error.fmt(formatter),
            Self::Tape(error) => error.fmt(formatter),
            Self::NotInitialized(path) => {
                write!(
                    formatter,
                    "route store {} is not initialized",
                    path.display()
                )
            }
            Self::InvalidSchema(schema) => {
                write!(formatter, "unsupported route object schema {schema:?}")
            }
            Self::InvalidObjectId(id) => write!(formatter, "invalid route object ID {id:?}"),
            Self::InvalidObject(message) => write!(formatter, "invalid route object: {message}"),
            Self::InvalidRef(name) => write!(formatter, "invalid route ref {name:?}"),
            Self::UnknownRef(name) => write!(formatter, "unknown route ref {name:?}"),
            Self::UnknownLineage(name) => write!(formatter, "unknown lineage {name:?}"),
            Self::RefNotLineage(name) => {
                write!(formatter, "route ref {name:?} does not point to a lineage")
            }
            Self::RefNotSnapshot(name) => write!(
                formatter,
                "route ref {name:?} does not point to a timeline snapshot"
            ),
            Self::HashMismatch { expected, actual } => {
                write!(formatter, "route object {expected} hashes to {actual}")
            }
            Self::ObjectCycle(id) => {
                write!(formatter, "route object graph contains a cycle at {id}")
            }
        }
    }
}

impl Error for StoreError {}

impl From<std::io::Error> for StoreError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}
impl From<serde_json::Error> for StoreError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}
impl From<TimelineError> for StoreError {
    fn from(value: TimelineError) -> Self {
        Self::Timeline(value)
    }
}
impl From<crate::search::SearchError> for StoreError {
    fn from(value: crate::search::SearchError) -> Self {
        Self::Search(value)
    }
}
impl From<crate::tape::TapeError> for StoreError {
    fn from(value: crate::tape::TapeError) -> Self {
        Self::Tape(value)
    }
}

impl std::str::FromStr for ObjectId {
    type Err = StoreError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = r#"
timeline test
milestone boot
milestone control
milestone map
segment boot_link from boot to control profile boot_to_fsp103
segment exit from control to map profile fsp103_to_fsp104
variant boot_link.safe incumbent uses baseline boot_to_fsp103 starts clean produces control-v1
variant exit.safe incumbent uses baseline fsp103_to_fsp104 starts control-v1 produces map-v1
continuation main starts root@clean
continue main with boot_link.safe after root@clean
continue main with exit.safe after boot_link.safe@control-v1
"#;

    #[test]
    fn store_import_fork_append_repair_and_gc_are_structural() {
        let root = std::env::temp_dir().join(format!("huntctl-route-store-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let store = RouteStore::initialize(&root).unwrap();
        let timeline = Timeline::parse(SOURCE).unwrap();
        let imported = store
            .import_timeline(&timeline, Path::new("."), "routes/main")
            .unwrap();
        assert_eq!(store.resolve_ref("routes/main").unwrap(), imported.snapshot);
        let main = store
            .fork_lineage("routes/main", "main", "experiments/main")
            .unwrap();
        assert_eq!(store.resolve_ref("experiments/main").unwrap(), main);

        let appended = store
            .append_lineage("experiments/main", &timeline, "main", Path::new("."))
            .unwrap();
        assert_ne!(appended, main);
        let repaired = store
            .replay_repair(
                "experiments/main",
                "experiments/repaired",
                &timeline,
                "main",
                Path::new("."),
            )
            .unwrap();
        assert_eq!(store.resolve_ref("experiments/repaired").unwrap(), repaired);
        assert!(store.verify().unwrap() > 0);

        let tape_path = root.join("evaluation.tape");
        fs::write(
            &tape_path,
            Candidate::baseline(crate::search::SegmentProfile::Fsp103ToFsp104)
                .compile()
                .unwrap()
                .encode()
                .unwrap(),
        )
        .unwrap();
        let evaluation_path = root.join("evaluation.json");
        fs::write(
            &evaluation_path,
            serde_json::to_vec(&serde_json::json!({
                "schema_version": 1,
                "candidate_id": "candidate-1",
                "tape": tape_path,
                "success": true,
                "first_hit_tick": 603
            }))
            .unwrap(),
        )
        .unwrap();
        let evaluation = store
            .import_evaluation(
                &evaluation_path,
                "f_sp104",
                "map-v1",
                Some("evaluations/candidate-1"),
            )
            .unwrap();
        assert_eq!(
            store.resolve_ref("evaluations/candidate-1").unwrap(),
            evaluation
        );

        let unreachable = store
            .put(RouteObject::Boundary {
                milestone: "unused".into(),
                fingerprint: "unused-v1".into(),
            })
            .unwrap();
        let report = store.gc(false).unwrap();
        assert!(report.unreachable.contains(&unreachable));
        assert_eq!(report.removed, 0);
        let report = store.gc(true).unwrap();
        assert!(report.removed >= 1);
        assert!(!store.object_path(&unreachable).exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn object_hash_tampering_is_rejected() {
        let root =
            std::env::temp_dir().join(format!("huntctl-route-tamper-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let store = RouteStore::initialize(&root).unwrap();
        let id = store
            .put(RouteObject::Boundary {
                milestone: "m".into(),
                fingerprint: "f".into(),
            })
            .unwrap();
        fs::write(store.object_path(&id), b"{}").unwrap();
        assert!(matches!(
            store.read(&id),
            Err(StoreError::HashMismatch { .. })
        ));
        fs::remove_dir_all(root).unwrap();
    }
}
