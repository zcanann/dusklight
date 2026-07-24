//! Deterministic discovery and extraction from a user-supplied retail `orig/` tree.
//!
//! The public artifacts contain normalized relative paths and digests, never
//! host paths or original game bytes. Exact identities come from either the
//! bundled audited registry or a caller-supplied registry; discovery verifies
//! every fingerprint field instead of trusting a friendly build label.

use crate::artifact::Digest;
use crate::identity::{ContentFingerprint, ContentIdentity, GamePlatform, GameRegion};
use crate::orig_extraction::{
    ExtractedMessageFlow, ExtractedStageData, extract_unique_rarc_resource,
    list_rarc_resource_names, parse_message_flow, parse_stage_data,
};
use crate::{PlannerContractError, canonical_json, require_canonical_json_bytes};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::{Component, Path, PathBuf};

pub const ORIG_INPUT_SCAN_SCHEMA: &str = "dusklight.route-planner.orig-input-scan/v1";
pub const EXTRACTED_ORIG_BUNDLE_SCHEMA: &str = "dusklight.route-planner.extracted-orig-bundle/v8";
pub const SUPPORTED_BUILD_REGISTRY_SCHEMA: &str =
    "dusklight.route-planner.supported-build-registry/v1";
pub const ORIG_IDENTIFICATION_SCHEMA: &str = "dusklight.route-planner.orig-identification/v1";
const BUNDLED_SUPPORTED_BUILD_REGISTRY: &[u8] = include_bytes!("../data/supported-builds.json");
const ORIG_FILE_MANIFEST_SCHEMA: &str = "dusklight.route-planner.orig-file-manifest/v1";
const MAX_ORIG_FILES: usize = 100_000;
const MAX_ARCHIVE_INPUT_BYTES: u64 = 128 * 1024 * 1024;
const MAX_SUPPORTED_IDENTITIES: usize = 1024;

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OrigFileRecord {
    pub relative_path: String,
    pub bytes: u64,
    pub sha256: Digest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OrigInputScan {
    pub schema: String,
    pub fingerprint: ContentFingerprint,
    pub file_manifest_sha256: Digest,
    pub files: Vec<OrigFileRecord>,
    pub extractable_archive_paths: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractedOrigStageArchive {
    pub relative_path: String,
    pub archive_sha256: Digest,
    pub resource_name: String,
    pub resource_sha256: Digest,
    pub stage: ExtractedStageData,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractedOrigMessageArchive {
    pub relative_path: String,
    pub archive_sha256: Digest,
    pub locale_bundle: String,
    pub message_group: u16,
    pub resource_name: String,
    pub resource_sha256: Digest,
    pub flow: ExtractedMessageFlow,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IgnoredOrigArchiveReason {
    NoMessageFlowResource,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractedOrigIgnoredArchive {
    pub relative_path: String,
    pub archive_sha256: Digest,
    pub reason: IgnoredOrigArchiveReason,
    pub resource_names: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractedOrigBundle {
    pub schema: String,
    pub content: ContentIdentity,
    pub input_scan: OrigInputScan,
    pub stages: Vec<ExtractedOrigStageArchive>,
    pub message_flows: Vec<ExtractedOrigMessageArchive>,
    pub ignored_archives: Vec<ExtractedOrigIgnoredArchive>,
}

/// Exact identities the caller is prepared to support. Product IDs and friendly
/// names are never sufficient on their own.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SupportedBuildRegistry {
    pub schema: String,
    pub identities: Vec<ContentIdentity>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum OrigSupportStatus {
    Supported { content: ContentIdentity },
    Unsupported { fingerprint: ContentFingerprint },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OrigIdentification {
    pub schema: String,
    pub scan_sha256: Digest,
    pub support: OrigSupportStatus,
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct OrigFileManifest<'a> {
    schema: &'static str,
    product_id: &'a str,
    files: &'a [OrigFileRecord],
}

struct DiscoveredOrig {
    game_root: PathBuf,
    scan: OrigInputScan,
}

impl OrigInputScan {
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PlannerContractError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn digest(&self) -> Result<Digest, PlannerContractError> {
        Ok(Digest(Sha256::digest(self.canonical_bytes()?).into()))
    }

    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != ORIG_INPUT_SCAN_SCHEMA {
            return Err(PlannerContractError::new(
                "orig.scan.schema",
                "is unsupported",
            ));
        }
        self.fingerprint.validate()?;
        validate_file_records(&self.files)?;
        if self.extractable_archive_paths.is_empty() {
            return Err(PlannerContractError::new(
                "orig.scan.extractable_archive_paths",
                "must contain at least one recognized stage or message archive",
            ));
        }
        if !is_strictly_sorted(&self.extractable_archive_paths) {
            return Err(PlannerContractError::new(
                "orig.scan.extractable_archive_paths",
                "must be unique and sorted",
            ));
        }
        for path in &self.extractable_archive_paths {
            validate_relative_path(path)?;
            if self
                .files
                .binary_search_by(|record| record.relative_path.cmp(path))
                .is_err()
            {
                return Err(PlannerContractError::new(
                    "orig.scan.extractable_archive_paths",
                    "references a file absent from the sealed manifest",
                ));
            }
        }
        let manifest_sha256 = digest_file_manifest(&self.fingerprint.product_id, &self.files)?;
        if manifest_sha256 != self.file_manifest_sha256
            || manifest_sha256 != self.fingerprint.game_data_sha256
        {
            return Err(PlannerContractError::new(
                "orig.scan.file_manifest_sha256",
                "does not match the canonical file manifest and content fingerprint",
            ));
        }
        let executable = self
            .files
            .iter()
            .find(|record| record.relative_path == "sys/main.dol")
            .ok_or_else(|| PlannerContractError::new("orig.scan", "is missing sys/main.dol"))?;
        if executable.sha256 != self.fingerprint.executable_sha256 {
            return Err(PlannerContractError::new(
                "orig.scan.fingerprint.executable_sha256",
                "does not match sys/main.dol in the sealed manifest",
            ));
        }
        let resources = self
            .files
            .iter()
            .filter(|record| {
                record.relative_path.starts_with("files/res/")
                    && record.relative_path.ends_with(".arc")
            })
            .cloned()
            .collect::<Vec<_>>();
        if resources.is_empty()
            || digest_file_manifest(&self.fingerprint.product_id, &resources)?
                != self.fingerprint.resource_manifest_sha256
        {
            return Err(PlannerContractError::new(
                "orig.scan.fingerprint.resource_manifest_sha256",
                "does not match the resource archives in the sealed manifest",
            ));
        }
        Ok(())
    }
}

impl ExtractedOrigBundle {
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PlannerContractError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn digest(&self) -> Result<Digest, PlannerContractError> {
        Ok(Digest(Sha256::digest(self.canonical_bytes()?).into()))
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, PlannerContractError> {
        let bundle: Self = serde_json::from_slice(bytes)?;
        bundle.validate()?;
        if bundle.canonical_bytes()? != bytes {
            return Err(PlannerContractError::new(
                "orig.bundle",
                "is not canonical JSON",
            ));
        }
        Ok(bundle)
    }

    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != EXTRACTED_ORIG_BUNDLE_SCHEMA {
            return Err(PlannerContractError::new(
                "orig.bundle.schema",
                "is unsupported",
            ));
        }
        self.content.validate()?;
        self.input_scan.validate()?;
        self.content.verify_detected(&self.input_scan.fingerprint)?;
        if self.stages.is_empty() && self.message_flows.is_empty() {
            return Err(PlannerContractError::new(
                "orig.bundle",
                "must contain at least one decoded stage or message archive",
            ));
        }
        if !is_sorted_by(&self.stages, |record| record.relative_path.as_str())
            || !is_sorted_by(&self.message_flows, |record| record.relative_path.as_str())
            || !is_sorted_by(&self.ignored_archives, |record| {
                record.relative_path.as_str()
            })
        {
            return Err(PlannerContractError::new(
                "orig.bundle",
                "decoded archives must be unique and sorted by relative path",
            ));
        }
        let mut classified_paths = BTreeSet::new();
        for record in &self.stages {
            validate_relative_path(&record.relative_path)?;
            if !classified_paths.insert(record.relative_path.as_str()) {
                return Err(PlannerContractError::new(
                    "orig.bundle",
                    "classifies one archive more than once",
                ));
            }
            require_source_digest(
                &self.input_scan.files,
                &record.relative_path,
                record.archive_sha256,
            )?;
            if !matches!(record.resource_name.as_str(), "stage.dzs" | "room.dzr") {
                return Err(PlannerContractError::new(
                    "orig.bundle.stages.resource_name",
                    "must be stage.dzs or room.dzr",
                ));
            }
            require_nonzero_digest("orig.bundle.stages.resource_sha256", record.resource_sha256)?;
        }
        for record in &self.message_flows {
            validate_relative_path(&record.relative_path)?;
            if !classified_paths.insert(record.relative_path.as_str()) {
                return Err(PlannerContractError::new(
                    "orig.bundle",
                    "classifies one archive more than once",
                ));
            }
            require_source_digest(
                &self.input_scan.files,
                &record.relative_path,
                record.archive_sha256,
            )?;
            if record.locale_bundle.is_empty()
                || !record
                    .locale_bundle
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric())
            {
                return Err(PlannerContractError::new(
                    "orig.bundle.message_flows.locale_bundle",
                    "must be nonempty ASCII letters or digits",
                ));
            }
            let expected = format!("zel_{:02}.bmg", record.message_group);
            if record.resource_name != expected {
                return Err(PlannerContractError::new(
                    "orig.bundle.message_flows.resource_name",
                    "does not match its message group",
                ));
            }
            require_nonzero_digest(
                "orig.bundle.message_flows.resource_sha256",
                record.resource_sha256,
            )?;
        }
        for record in &self.ignored_archives {
            validate_relative_path(&record.relative_path)?;
            if !classified_paths.insert(record.relative_path.as_str()) {
                return Err(PlannerContractError::new(
                    "orig.bundle",
                    "classifies one archive more than once",
                ));
            }
            require_source_digest(
                &self.input_scan.files,
                &record.relative_path,
                record.archive_sha256,
            )?;
            if !record
                .resource_names
                .windows(2)
                .all(|pair| pair[0] < pair[1])
            {
                return Err(PlannerContractError::new(
                    "orig.bundle.ignored_archives.resource_names",
                    "must be unique and sorted",
                ));
            }
            for name in &record.resource_names {
                if name.is_empty() || name.contains(['/', '\\', '\0']) {
                    return Err(PlannerContractError::new(
                        "orig.bundle.ignored_archives.resource_names",
                        "must contain valid basenames",
                    ));
                }
            }
        }
        Ok(())
    }
}

impl SupportedBuildRegistry {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != SUPPORTED_BUILD_REGISTRY_SCHEMA {
            return Err(PlannerContractError::new(
                "supported_build_registry.schema",
                "is unsupported",
            ));
        }
        if self.identities.len() > MAX_SUPPORTED_IDENTITIES {
            return Err(PlannerContractError::new(
                "supported_build_registry.identities",
                format!("must contain at most {MAX_SUPPORTED_IDENTITIES} entries"),
            ));
        }
        let mut previous_id = None;
        for identity in &self.identities {
            identity.validate()?;
            if previous_id.is_some_and(|id: &str| id >= identity.id.as_str()) {
                return Err(PlannerContractError::new(
                    "supported_build_registry.identities",
                    "must be unique and sorted by friendly content ID",
                ));
            }
            if self
                .identities
                .iter()
                .filter(|candidate| candidate.fingerprint == identity.fingerprint)
                .count()
                != 1
            {
                return Err(PlannerContractError::new(
                    "supported_build_registry.identities",
                    "must not assign two friendly IDs to one exact fingerprint",
                ));
            }
            previous_id = Some(identity.id.as_str());
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PlannerContractError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, PlannerContractError> {
        let registry: Self = serde_json::from_slice(bytes)?;
        registry.validate()?;
        require_canonical_json_bytes(
            "supported_build_registry",
            bytes,
            &registry.canonical_bytes()?,
        )?;
        Ok(registry)
    }

    pub fn identify(
        &self,
        scan: &OrigInputScan,
        requested_content_id: Option<&str>,
    ) -> Result<OrigIdentification, PlannerContractError> {
        self.validate()?;
        scan.validate()?;
        let exact_match = self
            .identities
            .iter()
            .find(|identity| identity.fingerprint == scan.fingerprint);
        let support = if let Some(requested_id) = requested_content_id {
            let requested = self
                .identities
                .iter()
                .find(|identity| identity.id == requested_id)
                .ok_or_else(|| {
                    PlannerContractError::new(
                        "requested_content_id",
                        "is absent from the supported-build registry",
                    )
                })?;
            requested.verify_detected(&scan.fingerprint)?;
            OrigSupportStatus::Supported {
                content: requested.clone(),
            }
        } else if let Some(identity) = exact_match {
            OrigSupportStatus::Supported {
                content: identity.clone(),
            }
        } else {
            OrigSupportStatus::Unsupported {
                fingerprint: scan.fingerprint.clone(),
            }
        };
        Ok(OrigIdentification {
            schema: ORIG_IDENTIFICATION_SCHEMA.into(),
            scan_sha256: scan.digest()?,
            support,
        })
    }
}

pub fn bundled_supported_build_registry() -> Result<SupportedBuildRegistry, PlannerContractError> {
    SupportedBuildRegistry::decode_canonical(BUNDLED_SUPPORTED_BUILD_REGISTRY)
}

impl OrigIdentification {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != ORIG_IDENTIFICATION_SCHEMA || self.scan_sha256 == Digest::ZERO {
            return Err(PlannerContractError::new(
                "orig_identification",
                "has an unsupported schema or zero scan digest",
            ));
        }
        match &self.support {
            OrigSupportStatus::Supported { content } => content.validate(),
            OrigSupportStatus::Unsupported { fingerprint } => fingerprint.validate(),
        }
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PlannerContractError> {
        self.validate()?;
        canonical_json(self)
    }
}

pub fn scan_orig_tree(
    supplied_root: &Path,
    expected_product_id: Option<&str>,
) -> Result<OrigInputScan, PlannerContractError> {
    let discovered = discover_orig_tree(supplied_root, expected_product_id)?;
    discovered.scan.validate()?;
    Ok(discovered.scan)
}

pub fn extract_orig_bundle(
    supplied_root: &Path,
    content: &ContentIdentity,
) -> Result<ExtractedOrigBundle, PlannerContractError> {
    content.validate()?;
    let discovered =
        discover_orig_tree(supplied_root, Some(content.fingerprint.product_id.as_str()))?;
    content.verify_detected(&discovered.scan.fingerprint)?;
    let mut stages = Vec::new();
    let mut message_flows = Vec::new();
    let mut ignored_archives = Vec::new();
    for relative_path in &discovered.scan.extractable_archive_paths {
        let path = discovered.game_root.join(relative_path);
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| io_error("orig.archive.metadata", error))?;
        if metadata.len() > MAX_ARCHIVE_INPUT_BYTES {
            return Err(PlannerContractError::new(
                "orig.archive.bytes",
                format!("{relative_path} exceeds {MAX_ARCHIVE_INPUT_BYTES} bytes"),
            ));
        }
        let archive = fs::read(&path).map_err(|error| io_error("orig.archive.read", error))?;
        let archive_sha256 = Digest(Sha256::digest(&archive).into());
        if let Some(resource_name) = stage_resource_name(relative_path) {
            let resource = extract_unique_rarc_resource(&archive, resource_name)?;
            stages.push(ExtractedOrigStageArchive {
                relative_path: relative_path.clone(),
                archive_sha256,
                resource_name: resource_name.into(),
                resource_sha256: Digest(Sha256::digest(&resource).into()),
                stage: parse_stage_data(&resource)?,
            });
        } else if let Some(locale_bundle) = message_archive_locale(relative_path) {
            let resource_names = list_rarc_resource_names(&archive)?;
            let flow_resources = resource_names
                .iter()
                .filter_map(|name| {
                    message_group_from_resource_name(name).map(|group| (name, group))
                })
                .collect::<Vec<_>>();
            let [(resource_name, message_group)] = flow_resources.as_slice() else {
                if flow_resources.is_empty() {
                    ignored_archives.push(ExtractedOrigIgnoredArchive {
                        relative_path: relative_path.clone(),
                        archive_sha256,
                        reason: IgnoredOrigArchiveReason::NoMessageFlowResource,
                        resource_names,
                    });
                    continue;
                }
                return Err(PlannerContractError::new(
                    "orig.message_archive",
                    format!("{relative_path} contains multiple numbered message-flow resources"),
                ));
            };
            let resource = extract_unique_rarc_resource(&archive, resource_name.as_str())?;
            message_flows.push(ExtractedOrigMessageArchive {
                relative_path: relative_path.clone(),
                archive_sha256,
                locale_bundle,
                message_group: *message_group,
                resource_name: resource_name.to_string(),
                resource_sha256: Digest(Sha256::digest(&resource).into()),
                flow: parse_message_flow(&resource)?,
            });
        }
    }
    let bundle = ExtractedOrigBundle {
        schema: EXTRACTED_ORIG_BUNDLE_SCHEMA.into(),
        content: content.clone(),
        input_scan: discovered.scan,
        stages,
        message_flows,
        ignored_archives,
    };
    bundle.validate()?;
    Ok(bundle)
}

fn discover_orig_tree(
    supplied_root: &Path,
    expected_product_id: Option<&str>,
) -> Result<DiscoveredOrig, PlannerContractError> {
    let game_root = locate_game_root(supplied_root, expected_product_id)?;
    let boot =
        fs::read(game_root.join("sys/boot.bin")).map_err(|error| io_error("orig.boot", error))?;
    if boot.len() < 8 {
        return Err(PlannerContractError::new(
            "orig.boot",
            "must contain at least the eight-byte disc identity header",
        ));
    }
    let product_id = std::str::from_utf8(&boot[..6])
        .map_err(|_| PlannerContractError::new("orig.product_id", "must be ASCII"))?
        .to_owned();
    if !product_id.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        return Err(PlannerContractError::new(
            "orig.product_id",
            "must contain six ASCII letters or digits",
        ));
    }
    if expected_product_id.is_some_and(|expected| expected != product_id) {
        return Err(PlannerContractError::new(
            "orig.product_id",
            "does not match the requested content identity",
        ));
    }
    let (platform, region) = decode_product_id(&product_id)?;
    let revision = format!("1.{}", boot[7]);
    let mut paths = Vec::new();
    for subtree in ["sys", "files"] {
        let directory = game_root.join(subtree);
        let metadata =
            fs::symlink_metadata(&directory).map_err(|error| io_error("orig.directory", error))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(PlannerContractError::new(
                "orig.directory",
                format!("{subtree} must be a real directory"),
            ));
        }
        collect_regular_files(&game_root, &directory, &mut paths)?;
    }
    if paths.len() > MAX_ORIG_FILES {
        return Err(PlannerContractError::new(
            "orig.files",
            format!("exceeds bounded limit {MAX_ORIG_FILES}"),
        ));
    }
    paths.sort();
    let mut files = Vec::with_capacity(paths.len());
    for relative_path in paths {
        let path = game_root.join(&relative_path);
        let metadata =
            fs::symlink_metadata(&path).map_err(|error| io_error("orig.file.metadata", error))?;
        files.push(OrigFileRecord {
            relative_path,
            bytes: metadata.len(),
            sha256: sha256_file(&path)?,
        });
    }
    validate_file_records(&files)?;
    let executable_sha256 = files
        .iter()
        .find(|record| record.relative_path == "sys/main.dol")
        .ok_or_else(|| {
            PlannerContractError::new("orig.executable", "missing required sys/main.dol")
        })?
        .sha256;
    let resources = files
        .iter()
        .filter(|record| {
            record.relative_path.starts_with("files/res/") && record.relative_path.ends_with(".arc")
        })
        .cloned()
        .collect::<Vec<_>>();
    if resources.is_empty() {
        return Err(PlannerContractError::new(
            "orig.resources",
            "contains no files/res/**/*.arc resources",
        ));
    }
    let resource_manifest_sha256 = digest_file_manifest(&product_id, &resources)?;
    let game_data_sha256 = digest_file_manifest(&product_id, &files)?;
    let extractable_archive_paths = files
        .iter()
        .filter(|record| {
            stage_resource_name(&record.relative_path).is_some()
                || message_archive_locale(&record.relative_path).is_some()
        })
        .map(|record| record.relative_path.clone())
        .collect::<Vec<_>>();
    let scan = OrigInputScan {
        schema: ORIG_INPUT_SCAN_SCHEMA.into(),
        fingerprint: ContentFingerprint {
            platform,
            region,
            revision,
            product_id,
            executable_sha256,
            game_data_sha256,
            resource_manifest_sha256,
        },
        file_manifest_sha256: game_data_sha256,
        files,
        extractable_archive_paths,
    };
    Ok(DiscoveredOrig { game_root, scan })
}

fn locate_game_root(
    supplied_root: &Path,
    expected_product_id: Option<&str>,
) -> Result<PathBuf, PlannerContractError> {
    let supplied_metadata =
        fs::symlink_metadata(supplied_root).map_err(|error| io_error("orig.root", error))?;
    if supplied_metadata.file_type().is_symlink() || !supplied_metadata.is_dir() {
        return Err(PlannerContractError::new(
            "orig.root",
            "must be a real directory rather than a symlink",
        ));
    }
    if supplied_root.join("sys/boot.bin").is_file() {
        return Ok(supplied_root.to_path_buf());
    }
    let entries = fs::read_dir(supplied_root)
        .map_err(|error| io_error("orig.root", error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| io_error("orig.root", error))?;
    let mut candidates = Vec::new();
    for entry in entries {
        let file_type = entry
            .file_type()
            .map_err(|error| io_error("orig.root", error))?;
        if file_type.is_symlink() || !file_type.is_dir() {
            continue;
        }
        let candidate = entry.path();
        let boot_path = candidate.join("sys/boot.bin");
        if !boot_path.is_file() {
            continue;
        }
        if let Some(expected) = expected_product_id {
            let boot = fs::read(&boot_path).map_err(|error| io_error("orig.boot", error))?;
            if boot.get(..6) != Some(expected.as_bytes()) {
                continue;
            }
        }
        candidates.push(candidate);
    }
    candidates.sort();
    match candidates.as_slice() {
        [candidate] => Ok(candidate.clone()),
        [] => Err(PlannerContractError::new(
            "orig.root",
            "does not contain an extracted game with sys/boot.bin",
        )),
        _ => Err(PlannerContractError::new(
            "orig.root",
            "contains multiple extracted games; supply an exact content identity or game root",
        )),
    }
}

fn collect_regular_files(
    game_root: &Path,
    directory: &Path,
    output: &mut Vec<String>,
) -> Result<(), PlannerContractError> {
    let mut entries = fs::read_dir(directory)
        .map_err(|error| io_error("orig.directory", error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| io_error("orig.directory", error))?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        let metadata =
            fs::symlink_metadata(&path).map_err(|error| io_error("orig.file.metadata", error))?;
        if metadata.file_type().is_symlink() {
            return Err(PlannerContractError::new(
                "orig.file",
                "symbolic links are not accepted in an immutable input tree",
            ));
        }
        if metadata.is_dir() {
            collect_regular_files(game_root, &path, output)?;
        } else if metadata.is_file() {
            if output.len() >= MAX_ORIG_FILES {
                return Err(PlannerContractError::new(
                    "orig.files",
                    format!("exceeds bounded limit {MAX_ORIG_FILES}"),
                ));
            }
            let relative = path
                .strip_prefix(game_root)
                .map_err(|_| PlannerContractError::new("orig.file", "escaped the game root"))?;
            output.push(normalized_relative_path(relative)?);
        }
    }
    Ok(())
}

fn normalized_relative_path(path: &Path) -> Result<String, PlannerContractError> {
    let mut parts = Vec::new();
    for component in path.components() {
        let Component::Normal(part) = component else {
            return Err(PlannerContractError::new(
                "orig.file.relative_path",
                "must contain only normal path components",
            ));
        };
        let part = part
            .to_str()
            .ok_or_else(|| PlannerContractError::new("orig.file.relative_path", "must be UTF-8"))?;
        if part.is_empty() || part.contains(['/', '\\', '\0']) {
            return Err(PlannerContractError::new(
                "orig.file.relative_path",
                "contains an invalid component",
            ));
        }
        parts.push(part);
    }
    if parts.is_empty() {
        return Err(PlannerContractError::new(
            "orig.file.relative_path",
            "must not be empty",
        ));
    }
    Ok(parts.join("/"))
}

fn validate_relative_path(path: &str) -> Result<(), PlannerContractError> {
    if path.is_empty()
        || path.starts_with('/')
        || path.ends_with('/')
        || path.contains('\0')
        || path
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
    {
        return Err(PlannerContractError::new(
            "orig.file.relative_path",
            "must be a normalized nonempty relative path",
        ));
    }
    Ok(())
}

fn decode_product_id(product_id: &str) -> Result<(GamePlatform, GameRegion), PlannerContractError> {
    let bytes = product_id.as_bytes();
    if bytes.len() != 6 {
        return Err(PlannerContractError::new(
            "orig.product_id",
            "must contain exactly six bytes",
        ));
    }
    let platform = match bytes[0] {
        b'G' => GamePlatform::GameCube,
        b'R' => GamePlatform::Wii,
        _ => {
            return Err(PlannerContractError::new(
                "orig.product_id",
                "does not identify a supported extracted GameCube or Wii disc",
            ));
        }
    };
    let region = match bytes[3] {
        b'E' => GameRegion::Usa,
        b'P' => GameRegion::Pal,
        b'J' => GameRegion::Japan,
        b'K' => GameRegion::Korea,
        b'C' => GameRegion::China,
        _ => {
            return Err(PlannerContractError::new(
                "orig.product_id",
                "contains an unsupported region code",
            ));
        }
    };
    Ok((platform, region))
}

fn stage_resource_name(relative_path: &str) -> Option<&'static str> {
    if !relative_path.starts_with("files/res/Stage/") || !relative_path.ends_with(".arc") {
        return None;
    }
    let file_name = relative_path.rsplit('/').next()?;
    if file_name == "STG_00.arc" {
        Some("stage.dzs")
    } else if file_name.starts_with('R') {
        Some("room.dzr")
    } else {
        None
    }
}

fn message_archive_locale(relative_path: &str) -> Option<String> {
    if !relative_path.starts_with("files/res/Msg") || !relative_path.ends_with(".arc") {
        return None;
    }
    let mut components = relative_path.split('/');
    if components.next()? != "files" || components.next()? != "res" {
        return None;
    }
    let locale = components.next()?.strip_prefix("Msg")?;
    let file_name = components.next()?;
    if components.next().is_some() {
        return None;
    }
    let group_hint = file_name.strip_prefix("bmgres")?.strip_suffix(".arc")?;
    if !group_hint.is_empty() && group_hint.parse::<u16>().is_err() {
        return None;
    }
    Some(locale.to_owned())
}

fn message_group_from_resource_name(resource_name: &str) -> Option<u16> {
    resource_name
        .strip_prefix("zel_")?
        .strip_suffix(".bmg")?
        .parse()
        .ok()
}

fn sha256_file(path: &Path) -> Result<Digest, PlannerContractError> {
    let file = File::open(path).map_err(|error| io_error("orig.file.read", error))?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|error| io_error("orig.file.read", error))?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(Digest(hasher.finalize().into()))
}

fn digest_file_manifest(
    product_id: &str,
    files: &[OrigFileRecord],
) -> Result<Digest, PlannerContractError> {
    let bytes = canonical_json(&OrigFileManifest {
        schema: ORIG_FILE_MANIFEST_SCHEMA,
        product_id,
        files,
    })?;
    Ok(Digest(Sha256::digest(bytes).into()))
}

fn validate_file_records(files: &[OrigFileRecord]) -> Result<(), PlannerContractError> {
    if files.is_empty() || files.len() > MAX_ORIG_FILES {
        return Err(PlannerContractError::new(
            "orig.files",
            format!("must contain between 1 and {MAX_ORIG_FILES} records"),
        ));
    }
    if !is_sorted_by(files, |record| record.relative_path.as_str()) {
        return Err(PlannerContractError::new(
            "orig.files",
            "must be unique and sorted by normalized relative path",
        ));
    }
    for record in files {
        validate_relative_path(&record.relative_path)?;
        require_nonzero_digest("orig.files.sha256", record.sha256)?;
    }
    Ok(())
}

fn require_source_digest(
    files: &[OrigFileRecord],
    relative_path: &str,
    digest: Digest,
) -> Result<(), PlannerContractError> {
    let source = files
        .binary_search_by(|record| record.relative_path.as_str().cmp(relative_path))
        .ok()
        .map(|index| &files[index])
        .ok_or_else(|| {
            PlannerContractError::new("orig.bundle.source", "is absent from the input scan")
        })?;
    if source.sha256 != digest {
        return Err(PlannerContractError::new(
            "orig.bundle.source",
            "digest disagrees with the input scan",
        ));
    }
    Ok(())
}

fn require_nonzero_digest(field: &str, digest: Digest) -> Result<(), PlannerContractError> {
    if digest == Digest::ZERO {
        Err(PlannerContractError::new(field, "must be nonzero"))
    } else {
        Ok(())
    }
}

fn is_strictly_sorted(values: &[String]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
}

fn is_sorted_by<T>(values: &[T], key: impl Fn(&T) -> &str) -> bool {
    values.windows(2).all(|pair| key(&pair[0]) < key(&pair[1]))
}

fn io_error(field: &str, error: std::io::Error) -> PlannerContractError {
    PlannerContractError::new(field, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

    struct FixtureRoot(PathBuf);

    impl FixtureRoot {
        fn new() -> Self {
            let id = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "dusklight-route-planner-orig-{}-{id}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for FixtureRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write(path: &Path, bytes: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn game_fixture(root: &Path, product_id: &str, revision: u8) -> PathBuf {
        let game = root.join(product_id);
        let mut boot = vec![0_u8; 8];
        boot[..6].copy_from_slice(product_id.as_bytes());
        boot[7] = revision;
        write(&game.join("sys/boot.bin"), &boot);
        write(&game.join("sys/main.dol"), b"synthetic executable");
        write(
            &game.join("files/res/Stage/TEST/STG_00.arc"),
            b"synthetic stage archive",
        );
        write(
            &game.join("files/res/Msgus/bmgres3.arc"),
            b"synthetic message archive",
        );
        game
    }

    fn rarc(resource_name: &str, resource: &[u8]) -> Vec<u8> {
        let data_base = 0x80_usize;
        let mut archive = vec![0_u8; data_base + resource.len()];
        archive[0..4].copy_from_slice(b"RARC");
        let archive_size = archive.len() as u32;
        archive[4..8].copy_from_slice(&archive_size.to_be_bytes());
        archive[12..16].copy_from_slice(&0x60_u32.to_be_bytes());
        archive[0x28..0x2c].copy_from_slice(&1_u32.to_be_bytes());
        archive[0x2c..0x30].copy_from_slice(&0x20_u32.to_be_bytes());
        archive[0x34..0x38].copy_from_slice(&0x34_u32.to_be_bytes());
        archive[0x44..0x46].copy_from_slice(&0x0100_u16.to_be_bytes());
        archive[0x48..0x4c].copy_from_slice(&0_u32.to_be_bytes());
        archive[0x4c..0x50].copy_from_slice(&(resource.len() as u32).to_be_bytes());
        archive[0x54..0x54 + resource_name.len()].copy_from_slice(resource_name.as_bytes());
        archive[0x54 + resource_name.len()] = 0;
        archive[data_base..].copy_from_slice(resource);
        archive
    }

    fn empty_rarc() -> Vec<u8> {
        let mut archive = rarc("unused", &[]);
        archive[0x28..0x2c].copy_from_slice(&0_u32.to_be_bytes());
        archive
    }

    fn minimal_bmg() -> Vec<u8> {
        let mut bmg = vec![0_u8; 0x40];
        bmg[0..8].copy_from_slice(b"MESGbmg1");
        bmg[8..12].copy_from_slice(&0x40_u32.to_be_bytes());
        bmg[12..16].copy_from_slice(&2_u32.to_be_bytes());
        bmg[0x20..0x24].copy_from_slice(b"FLW1");
        bmg[0x24..0x28].copy_from_slice(&0x10_u32.to_be_bytes());
        bmg[0x30..0x34].copy_from_slice(b"FLI1");
        bmg[0x34..0x38].copy_from_slice(&0x10_u32.to_be_bytes());
        bmg
    }

    #[test]
    fn discovers_parent_or_game_root_without_trusting_the_directory_label() {
        let fixture = FixtureRoot::new();
        let game = game_fixture(&fixture.0, "GZ2E01", 0);
        write(&game.join("GZ2E01.iso"), b"redundant container image");
        let parent_scan = scan_orig_tree(&fixture.0, Some("GZ2E01")).unwrap();
        let direct_scan = scan_orig_tree(&game, Some("GZ2E01")).unwrap();
        assert_eq!(parent_scan, direct_scan);
        assert_eq!(parent_scan.fingerprint.platform, GamePlatform::GameCube);
        assert_eq!(parent_scan.fingerprint.region, GameRegion::Usa);
        assert_eq!(parent_scan.fingerprint.revision, "1.0");
        assert!(
            parent_scan
                .files
                .iter()
                .all(|record| record.relative_path != "GZ2E01.iso")
        );
        assert_eq!(
            parent_scan.extractable_archive_paths,
            vec![
                "files/res/Msgus/bmgres3.arc",
                "files/res/Stage/TEST/STG_00.arc"
            ]
        );

        let misleading = fixture.0.join("not-the-product-label");
        fs::rename(&game, &misleading).unwrap();
        assert_eq!(
            scan_orig_tree(&misleading, Some("GZ2E01")).unwrap(),
            parent_scan
        );
    }

    #[test]
    fn product_mismatch_ambiguity_and_symlinks_fail_closed() {
        let fixture = FixtureRoot::new();
        let _game = game_fixture(&fixture.0, "GZ2E01", 0);
        assert!(scan_orig_tree(&fixture.0, Some("GZ2P01")).is_err());
        game_fixture(&fixture.0, "GZ2P01", 0);
        assert!(scan_orig_tree(&fixture.0, None).is_err());

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(
                _game.join("sys/main.dol"),
                _game.join("files/res/linked.arc"),
            )
            .unwrap();
            assert!(scan_orig_tree(&_game, Some("GZ2E01")).is_err());
        }
    }

    #[test]
    fn identity_verification_rejects_digest_or_friendly_label_disagreement() {
        let fixture = FixtureRoot::new();
        let game = game_fixture(&fixture.0, "GZ2E01", 0);
        let scan = scan_orig_tree(&game, Some("GZ2E01")).unwrap();
        let exact = ContentIdentity::new("gcn-us-1.0", scan.fingerprint.clone()).unwrap();
        exact.verify_detected(&scan.fingerprint).unwrap();

        let mut wrong = exact;
        wrong.fingerprint.resource_manifest_sha256 = Digest([0x55; 32]);
        assert!(wrong.verify_detected(&scan.fingerprint).is_err());
    }

    #[test]
    fn registry_classifies_only_exact_fingerprints_and_rejects_label_override() {
        let fixture = FixtureRoot::new();
        let game = game_fixture(&fixture.0, "GZ2E01", 0);
        let scan = scan_orig_tree(&game, Some("GZ2E01")).unwrap();
        let known = ContentIdentity::new("gcn-us-1.0", scan.fingerprint.clone()).unwrap();
        let registry = SupportedBuildRegistry {
            schema: SUPPORTED_BUILD_REGISTRY_SCHEMA.into(),
            identities: vec![known.clone()],
        };
        let identified = registry.identify(&scan, None).unwrap();
        assert_eq!(
            identified.support,
            OrigSupportStatus::Supported {
                content: known.clone()
            }
        );
        assert_eq!(
            SupportedBuildRegistry::decode_canonical(&registry.canonical_bytes().unwrap()).unwrap(),
            registry
        );

        let mut changed_scan = scan.clone();
        changed_scan.fingerprint.executable_sha256 = Digest([0x44; 32]);
        changed_scan
            .files
            .iter_mut()
            .find(|record| record.relative_path == "sys/main.dol")
            .unwrap()
            .sha256 = Digest([0x44; 32]);
        changed_scan.file_manifest_sha256 =
            digest_file_manifest(&changed_scan.fingerprint.product_id, &changed_scan.files)
                .unwrap();
        changed_scan.fingerprint.game_data_sha256 = changed_scan.file_manifest_sha256;
        let unsupported = registry.identify(&changed_scan, None).unwrap();
        assert!(matches!(
            unsupported.support,
            OrigSupportStatus::Unsupported { .. }
        ));
        assert!(
            registry
                .identify(&changed_scan, Some("gcn-us-1.0"))
                .is_err()
        );

        let duplicate = SupportedBuildRegistry {
            schema: SUPPORTED_BUILD_REGISTRY_SCHEMA.into(),
            identities: vec![
                ContentIdentity::new("gcn-us-1.0", scan.fingerprint.clone()).unwrap(),
                ContentIdentity::new("gcn-us-copy", scan.fingerprint.clone()).unwrap(),
            ],
        };
        assert!(duplicate.validate().is_err());
    }

    #[test]
    fn bundled_registry_is_canonical_and_names_every_audited_exact_identity() {
        let registry = bundled_supported_build_registry().unwrap();
        assert_eq!(registry.identities.len(), 3);
        assert_eq!(
            registry
                .identities
                .iter()
                .map(|identity| (
                    identity.id.as_str(),
                    identity.fingerprint.product_id.as_str(),
                    identity.fingerprint.revision.as_str(),
                ))
                .collect::<Vec<_>>(),
            vec![
                ("gcn-pal-1.0-gz2p01", "GZ2P01", "1.0"),
                ("gcn-us-1.0-gz2e01", "GZ2E01", "1.0"),
                ("wii-us-1.2-rzde01", "RZDE01", "1.2"),
            ]
        );
        assert_eq!(
            registry.canonical_bytes().unwrap(),
            BUNDLED_SUPPORTED_BUILD_REGISTRY
        );

        let crlf = BUNDLED_SUPPORTED_BUILD_REGISTRY
            .iter()
            .flat_map(|byte| {
                if *byte == b'\n' {
                    vec![b'\r', b'\n']
                } else {
                    vec![*byte]
                }
            })
            .collect::<Vec<_>>();
        let error = SupportedBuildRegistry::decode_canonical(&crlf).unwrap_err();
        assert_eq!(error.field(), "supported_build_registry");
        assert_eq!(
            error.detail(),
            "contains carriage returns; canonical JSON must use LF line endings"
        );
    }

    #[test]
    fn one_call_extracts_a_verified_sealed_bundle_and_detects_later_mutation() {
        let fixture = FixtureRoot::new();
        let game = game_fixture(&fixture.0, "GZ2E01", 0);
        write(
            &game.join("files/res/Stage/TEST/STG_00.arc"),
            &rarc("stage.dzs", &0_u32.to_be_bytes()),
        );
        write(
            &game.join("files/res/Msgus/bmgres3.arc"),
            &rarc("zel_03.bmg", &minimal_bmg()),
        );
        write(
            &game.join("files/res/Msgus/bmgres.arc"),
            &rarc("zel_00.bmg", &minimal_bmg()),
        );
        write(&game.join("files/res/Msgus/bmgres99.arc"), &empty_rarc());
        let scan = scan_orig_tree(&game, Some("GZ2E01")).unwrap();
        let content = ContentIdentity::new("gcn-us-1.0", scan.fingerprint.clone()).unwrap();
        let bundle = extract_orig_bundle(&fixture.0, &content).unwrap();
        assert_eq!(bundle.stages.len(), 1);
        assert_eq!(bundle.stages[0].resource_name, "stage.dzs");
        assert_eq!(bundle.message_flows.len(), 2);
        assert_eq!(bundle.message_flows[0].locale_bundle, "us");
        assert_eq!(bundle.message_flows[0].message_group, 0);
        assert_eq!(bundle.message_flows[1].message_group, 3);
        assert_eq!(bundle.message_flows[1].flow.node_count, 0);
        assert_eq!(bundle.ignored_archives.len(), 1);
        assert_eq!(
            bundle.ignored_archives[0].relative_path,
            "files/res/Msgus/bmgres99.arc"
        );
        assert_eq!(
            bundle.ignored_archives[0].reason,
            IgnoredOrigArchiveReason::NoMessageFlowResource
        );
        assert!(bundle.ignored_archives[0].resource_names.is_empty());
        let canonical = bundle.canonical_bytes().unwrap();
        assert!(
            !String::from_utf8(canonical)
                .unwrap()
                .contains(&fixture.0.to_string_lossy()[..])
        );

        write(
            &game.join("files/res/Msgus/bmgres3.arc"),
            b"mutated after identity creation",
        );
        assert!(extract_orig_bundle(&fixture.0, &content).is_err());
    }
}
