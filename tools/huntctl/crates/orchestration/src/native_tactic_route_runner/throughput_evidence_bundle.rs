use super::scratch_discovery::route_report_sha256;
use super::throughput_curve::sample_from_route_report;
use super::*;
use crate::native_residual_campaign::NativeResidualExecutionBinding;
use dusklight_evidence::content_store::{ContentKind, ContentStore};
use dusklight_harness_contracts::objective_suite::ArtifactReference;

pub const NATIVE_TACTIC_THROUGHPUT_EVIDENCE_BUNDLE_SCHEMA_V1: &str =
    "dusklight-native-tactic-throughput-evidence-bundle/v1";
pub const NATIVE_TACTIC_THROUGHPUT_EVIDENCE_MANIFEST: &str = "manifest.json";
const MAXIMUM_COMPRESSED_DOCUMENT_BYTES: usize = 512 * 1024 * 1024;
const DOCUMENT_COMPRESSION_LEVEL: i32 = 3;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticThroughputCompressedArtifact {
    pub logical_identity_sha256: Digest,
    pub uncompressed_bytes: u64,
    pub blob: dusklight_evidence::content_store::ContentBlob,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticThroughputSampleEvidence {
    pub ordinal: u32,
    pub repetition: u32,
    pub workers: usize,
    pub route_report: NativeTacticThroughputCompressedArtifact,
    pub campaign_audit: NativeTacticThroughputCompressedArtifact,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticThroughputEvidenceBundle {
    pub schema: String,
    pub content_sha256: Digest,
    pub curve_report: NativeTacticScratchBundleArtifact,
    pub optimization_request: NativeTacticScratchBundleArtifact,
    pub execution_binding: NativeTacticScratchBundleArtifact,
    pub execution_plan: NativeTacticScratchBundleArtifact,
    pub execution_identity: NativeTacticScratchExecutionIdentity,
    pub authorities: Vec<NativeTacticScratchAuthorityArtifact>,
    pub samples: Vec<NativeTacticThroughputSampleEvidence>,
    pub passed: bool,
}

impl NativeTacticThroughputEvidenceBundle {
    pub fn build(
        bundle_root: &Path,
        repository_root: &Path,
        request_path: &Path,
        execution_path: &Path,
        curve_report_path: &Path,
    ) -> Result<Self, NativeTacticRouteRunError> {
        if bundle_root.exists() {
            return Err(route_message(
                "native tactic throughput evidence bundle output already exists",
            ));
        }
        let repository_root = repository_root.canonicalize().map_err(route_error)?;
        let request_path = confined_source(&repository_root, request_path)?;
        let execution_path = confined_source(&repository_root, execution_path)?;
        let curve_report_path = confined_source(&repository_root, curve_report_path)?;
        let request: OptimizationRequest = read_json_file(&request_path)?;
        let execution: NativeResidualExecutionBinding = read_json_file(&execution_path)?;
        let curve = NativeTacticThroughputCurveReport::read_and_validate(&curve_report_path)?;
        request.validate().map_err(route_error)?;
        execution.validate_seal(&request).map_err(route_error)?;
        if curve.optimization_request_sha256 != request.content_sha256
            || curve.execution_binding_sha256 != execution.content_sha256
        {
            return Err(route_message(
                "native tactic throughput evidence inputs are detached",
            ));
        }

        let route_sources = curve
            .execution_order
            .iter()
            .map(|sample| {
                let path = confined_source(&repository_root, Path::new(&sample.route_report_path))?;
                let bytes = fs::read(&path).map_err(route_error)?;
                let route: NativeTacticRouteReport =
                    serde_json::from_slice(&bytes).map_err(route_error)?;
                validate_route_binding(&curve, sample, &route, &bytes)?;
                Ok((path, bytes, route))
            })
            .collect::<Result<Vec<_>, NativeTacticRouteRunError>>()?;
        let first_route = route_sources
            .first()
            .map(|(_, _, route)| route)
            .ok_or_else(|| route_message("native tactic throughput curve has no route samples"))?;
        let plan_path = confined_source(
            &repository_root,
            Path::new(&first_route.execution_plan_path),
        )?;
        let plan = NativeTacticExecutionPlan::read(&plan_path)?;
        if plan.identity()? != curve.execution_plan_sha256 {
            return Err(route_message(
                "native tactic throughput execution plan identity differs",
            ));
        }

        let store = ContentStore::initialize(bundle_root).map_err(route_error)?;
        let mut guard = BundleBuildGuard::new(bundle_root);
        let curve_report = bundle_file(
            &store,
            &curve_report_path,
            ContentKind::DatasetManifest,
            curve.content_sha256,
        )?;
        let optimization_request = bundle_file(
            &store,
            &request_path,
            ContentKind::DatasetManifest,
            request.content_sha256,
        )?;
        let execution_binding = bundle_file(
            &store,
            &execution_path,
            ContentKind::DatasetManifest,
            execution.content_sha256,
        )?;
        let execution_plan = bundle_file(
            &store,
            &plan_path,
            ContentKind::DatasetManifest,
            plan.identity()?,
        )?;
        let mut authorities =
            throughput_authorities(&store, &repository_root, &request, &execution)?;
        authorities.sort_by(|left, right| left.role.cmp(&right.role));

        let mut samples = Vec::with_capacity(route_sources.len());
        for ((_, route_bytes, route), sample) in route_sources.iter().zip(&curve.execution_order) {
            let audit = NativeTacticScratchCampaignAudit::build(&repository_root, route)?;
            audit.validate_resource_binding(route, &plan)?;
            samples.push(NativeTacticThroughputSampleEvidence {
                ordinal: sample.ordinal,
                repetition: sample.repetition,
                workers: sample.workers,
                route_report: bundle_compressed(&store, route_bytes, sample.route_report_sha256)?,
                campaign_audit: bundle_compressed(
                    &store,
                    &audit.to_pretty_json()?,
                    audit.content_sha256,
                )?,
            });
        }

        let mut bundle = Self {
            schema: NATIVE_TACTIC_THROUGHPUT_EVIDENCE_BUNDLE_SCHEMA_V1.into(),
            content_sha256: Digest::ZERO,
            curve_report,
            optimization_request,
            execution_binding,
            execution_plan,
            execution_identity: execution_identity(&execution),
            authorities,
            samples,
            passed: curve.passed,
        };
        bundle.content_sha256 = bundle.compute_content_sha256()?;
        bundle.validate(bundle_root)?;
        let mut bytes = serde_json::to_vec_pretty(&bundle).map_err(route_error)?;
        bytes.push(b'\n');
        fs::write(
            bundle_root.join(NATIVE_TACTIC_THROUGHPUT_EVIDENCE_MANIFEST),
            bytes,
        )
        .map_err(route_error)?;
        guard.commit();
        Ok(bundle)
    }

    pub fn read_and_validate(bundle_root: &Path) -> Result<Self, NativeTacticRouteRunError> {
        let bundle: Self = serde_json::from_slice(
            &fs::read(bundle_root.join(NATIVE_TACTIC_THROUGHPUT_EVIDENCE_MANIFEST))
                .map_err(route_error)?,
        )
        .map_err(route_error)?;
        bundle.validate(bundle_root)?;
        Ok(bundle)
    }

    pub fn validate(&self, bundle_root: &Path) -> Result<(), NativeTacticRouteRunError> {
        if self.schema != NATIVE_TACTIC_THROUGHPUT_EVIDENCE_BUNDLE_SCHEMA_V1
            || self.content_sha256 == Digest::ZERO
            || self.content_sha256 != self.compute_content_sha256()?
            || self.samples.is_empty()
            || self.authorities.len() != 6
            || !self
                .samples
                .windows(2)
                .all(|pair| pair[0].ordinal < pair[1].ordinal)
            || !self
                .authorities
                .windows(2)
                .all(|pair| pair[0].role < pair[1].role)
        {
            return Err(route_message(
                "native tactic throughput evidence manifest is invalid",
            ));
        }
        let store = ContentStore::open(bundle_root).map_err(route_error)?;
        for artifact in [
            &self.curve_report,
            &self.optimization_request,
            &self.execution_binding,
            &self.execution_plan,
        ] {
            store.verify(&artifact.blob).map_err(route_error)?;
        }
        for authority in &self.authorities {
            store.verify(&authority.blob).map_err(route_error)?;
            if authority.blob.sha256 != authority.declared_sha256 {
                return Err(route_message(
                    "native tactic throughput authority digest differs",
                ));
            }
        }
        for sample in &self.samples {
            store
                .verify(&sample.route_report.blob)
                .map_err(route_error)?;
            store
                .verify(&sample.campaign_audit.blob)
                .map_err(route_error)?;
        }

        let request: OptimizationRequest = read_json(&store, &self.optimization_request)?;
        let execution: NativeResidualExecutionBinding = read_json(&store, &self.execution_binding)?;
        let plan =
            NativeTacticExecutionPlan::read(&store.blob_path(self.execution_plan.blob.sha256))?;
        let curve: NativeTacticThroughputCurveReport = read_json(&store, &self.curve_report)?;
        request.validate().map_err(route_error)?;
        execution.validate_seal(&request).map_err(route_error)?;
        curve.validate()?;
        if self.curve_report.logical_identity_sha256 != curve.content_sha256
            || self.optimization_request.logical_identity_sha256 != request.content_sha256
            || self.execution_binding.logical_identity_sha256 != execution.content_sha256
            || self.execution_plan.logical_identity_sha256 != plan.identity()?
            || curve.optimization_request_sha256 != request.content_sha256
            || curve.execution_binding_sha256 != execution.content_sha256
            || curve.execution_plan_sha256 != plan.identity()?
            || self.execution_identity != execution_identity(&execution)
            || self.passed != curve.passed
            || self.samples.len() != curve.execution_order.len()
            || !authorities_match(self, &request, &execution)
        {
            return Err(route_message(
                "native tactic throughput evidence authorities are detached",
            ));
        }

        for (evidence, expected) in self.samples.iter().zip(&curve.execution_order) {
            let route_bytes = read_compressed(&store, &evidence.route_report)?;
            let audit_bytes = read_compressed(&store, &evidence.campaign_audit)?;
            let route: NativeTacticRouteReport =
                serde_json::from_slice(&route_bytes).map_err(route_error)?;
            let audit: NativeTacticScratchCampaignAudit =
                serde_json::from_slice(&audit_bytes).map_err(route_error)?;
            validate_route_binding(&curve, expected, &route, &route_bytes)?;
            audit.validate()?;
            audit.validate_resource_binding(&route, &plan)?;
            if evidence.ordinal != expected.ordinal
                || evidence.repetition != expected.repetition
                || evidence.workers != expected.workers
                || evidence.route_report.logical_identity_sha256 != expected.route_report_sha256
                || evidence.campaign_audit.logical_identity_sha256 != audit.content_sha256
                || audit.route_report_sha256 != route_report_sha256(&route)?
            {
                return Err(route_message(
                    "native tactic throughput sample evidence is detached",
                ));
            }
        }
        Ok(())
    }

    fn compute_content_sha256(&self) -> Result<Digest, NativeTacticRouteRunError> {
        let mut unsigned = self.clone();
        unsigned.content_sha256 = Digest::ZERO;
        Ok(Digest(
            Sha256::digest(serde_cbor::to_vec(&unsigned).map_err(route_error)?).into(),
        ))
    }
}

fn validate_route_binding(
    curve: &NativeTacticThroughputCurveReport,
    expected: &NativeTacticThroughputCurveSample,
    route: &NativeTacticRouteReport,
    route_bytes: &[u8],
) -> Result<(), NativeTacticRouteRunError> {
    let route_sha256 = Digest(Sha256::digest(route_bytes).into());
    let derived = sample_from_route_report(
        expected.ordinal,
        expected.repetition,
        expected.workers,
        expected.route_report_path.clone(),
        route_sha256,
        route,
    );
    if route.schema != NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V39
        || route.optimization_request_sha256 != curve.optimization_request_sha256
        || route.execution_binding_sha256 != curve.execution_binding_sha256
        || route.execution_plan_sha256 != curve.execution_plan_sha256
        || route.workers != expected.workers
        || route_sha256 != expected.route_report_sha256
        || derived != *expected
    {
        return Err(route_message(
            "native tactic throughput route report differs from its aggregate sample",
        ));
    }
    Ok(())
}

pub(super) fn bundle_compressed(
    store: &ContentStore,
    bytes: &[u8],
    logical_identity_sha256: Digest,
) -> Result<NativeTacticThroughputCompressedArtifact, NativeTacticRouteRunError> {
    if bytes.is_empty() || bytes.len() > MAXIMUM_COMPRESSED_DOCUMENT_BYTES {
        return Err(route_message(
            "native tactic throughput evidence document size is invalid",
        ));
    }
    let compressed =
        zstd::bulk::compress(bytes, DOCUMENT_COMPRESSION_LEVEL).map_err(route_error)?;
    Ok(NativeTacticThroughputCompressedArtifact {
        logical_identity_sha256,
        uncompressed_bytes: u64::try_from(bytes.len()).map_err(route_error)?,
        blob: store
            .put_bytes(&compressed, ContentKind::CrashArtifact)
            .map_err(route_error)?,
    })
}

pub(super) fn read_compressed(
    store: &ContentStore,
    artifact: &NativeTacticThroughputCompressedArtifact,
) -> Result<Vec<u8>, NativeTacticRouteRunError> {
    let expected = usize::try_from(artifact.uncompressed_bytes).map_err(route_error)?;
    if expected == 0 || expected > MAXIMUM_COMPRESSED_DOCUMENT_BYTES {
        return Err(route_message(
            "native tactic throughput compressed document size is invalid",
        ));
    }
    let compressed = store.read_bytes(&artifact.blob).map_err(route_error)?;
    let bytes = zstd::bulk::decompress(&compressed, expected).map_err(route_error)?;
    if bytes.len() != expected {
        return Err(route_message(
            "native tactic throughput decompressed document size differs",
        ));
    }
    Ok(bytes)
}

fn throughput_authorities(
    store: &ContentStore,
    repository_root: &Path,
    request: &OptimizationRequest,
    execution: &NativeResidualExecutionBinding,
) -> Result<Vec<NativeTacticScratchAuthorityArtifact>, NativeTacticRouteRunError> {
    [
        (
            "card_fixture_manifest",
            &execution.card_fixture_manifest,
            ContentKind::DatasetManifest,
        ),
        (
            "milestone_program",
            &execution.milestone_program,
            ContentKind::DatasetManifest,
        ),
        (
            "process_boot_tape",
            &execution.process_boot_tape,
            ContentKind::InputTape,
        ),
        (
            "route_timeline",
            &request.route.timeline,
            ContentKind::DatasetManifest,
        ),
        (
            "terminal_predicate_source",
            &request.terminal_predicate.source,
            ContentKind::DatasetManifest,
        ),
        (
            "world_context",
            &execution.world_context,
            ContentKind::WorldContext,
        ),
    ]
    .into_iter()
    .map(|(role, reference, kind)| bundle_authority(store, repository_root, role, reference, kind))
    .collect()
}

fn authorities_match(
    bundle: &NativeTacticThroughputEvidenceBundle,
    request: &OptimizationRequest,
    execution: &NativeResidualExecutionBinding,
) -> bool {
    let expected = [
        ("card_fixture_manifest", &execution.card_fixture_manifest),
        ("milestone_program", &execution.milestone_program),
        ("process_boot_tape", &execution.process_boot_tape),
        ("route_timeline", &request.route.timeline),
        (
            "terminal_predicate_source",
            &request.terminal_predicate.source,
        ),
        ("world_context", &execution.world_context),
    ];
    bundle
        .authorities
        .iter()
        .zip(expected)
        .all(|(actual, (role, reference))| {
            actual.role == role
                && actual.declared_path == reference.path
                && actual.declared_sha256 == reference.sha256
        })
}

fn execution_identity(
    execution: &NativeResidualExecutionBinding,
) -> NativeTacticScratchExecutionIdentity {
    NativeTacticScratchExecutionIdentity {
        execution_binding_sha256: execution.content_sha256,
        executable_sha256: execution.executable.sha256,
        runtime_dependency_sha256s: execution
            .runtime_dependencies
            .iter()
            .map(|dependency| dependency.sha256)
            .collect(),
        game_data_sha256: execution.game_data.sha256,
        process_boot_tape_sha256: execution.process_boot_tape.sha256,
        milestone_program_sha256: execution.milestone_program.sha256,
        world_context_sha256: execution.world_context.sha256,
        card_fixture_manifest_sha256: execution.card_fixture_manifest.sha256,
    }
}

fn bundle_authority(
    store: &ContentStore,
    repository_root: &Path,
    role: &str,
    reference: &ArtifactReference,
    kind: ContentKind,
) -> Result<NativeTacticScratchAuthorityArtifact, NativeTacticRouteRunError> {
    let path = confined_source(repository_root, Path::new(&reference.path))?;
    let blob = store.put_file(&path, kind).map_err(route_error)?;
    if blob.sha256 != reference.sha256 {
        return Err(route_message(
            "native tactic throughput authority bytes differ",
        ));
    }
    Ok(NativeTacticScratchAuthorityArtifact {
        role: role.into(),
        declared_path: reference.path.clone(),
        declared_sha256: reference.sha256,
        blob,
    })
}

fn bundle_file(
    store: &ContentStore,
    path: &Path,
    kind: ContentKind,
    logical_identity_sha256: Digest,
) -> Result<NativeTacticScratchBundleArtifact, NativeTacticRouteRunError> {
    Ok(NativeTacticScratchBundleArtifact {
        logical_identity_sha256,
        blob: store.put_file(path, kind).map_err(route_error)?,
    })
}

fn read_json<T: for<'de> Deserialize<'de>>(
    store: &ContentStore,
    artifact: &NativeTacticScratchBundleArtifact,
) -> Result<T, NativeTacticRouteRunError> {
    serde_json::from_slice(&store.read_bytes(&artifact.blob).map_err(route_error)?)
        .map_err(route_error)
}

fn read_json_file<T: for<'de> Deserialize<'de>>(
    path: &Path,
) -> Result<T, NativeTacticRouteRunError> {
    serde_json::from_slice(&fs::read(path).map_err(route_error)?).map_err(route_error)
}

fn confined_source(
    repository_root: &Path,
    declared: &Path,
) -> Result<PathBuf, NativeTacticRouteRunError> {
    let candidate = if declared.is_absolute() {
        declared.to_path_buf()
    } else {
        repository_root.join(declared)
    };
    let resolved = candidate.canonicalize().map_err(route_error)?;
    if !resolved.starts_with(repository_root) || !resolved.is_file() {
        return Err(route_message(
            "native tactic throughput evidence source is outside the repository or not a file",
        ));
    }
    Ok(resolved)
}

struct BundleBuildGuard {
    root: PathBuf,
    committed: bool,
}

impl BundleBuildGuard {
    fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            committed: false,
        }
    }

    fn commit(&mut self) {
        self.committed = true;
    }
}

impl Drop for BundleBuildGuard {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn throughput_evidence_schema_is_distinct_from_launch_and_acceptance() {
        assert_ne!(
            NATIVE_TACTIC_THROUGHPUT_EVIDENCE_BUNDLE_SCHEMA_V1,
            NATIVE_TACTIC_LAUNCH_SMOKE_BUNDLE_SCHEMA_V1
        );
        assert_ne!(
            NATIVE_TACTIC_THROUGHPUT_EVIDENCE_BUNDLE_SCHEMA_V1,
            NATIVE_TACTIC_SCRATCH_EVIDENCE_BUNDLE_SCHEMA_V2
        );
    }

    #[test]
    fn compressed_documents_round_trip_without_losing_logical_identity() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "dusklight-throughput-bundle-{}-{unique}",
            std::process::id()
        ));
        let store = ContentStore::initialize(&root).unwrap();
        let bytes = br#"{"schema":"test","rows":[1,2,3]}"#;
        let identity = Digest(Sha256::digest(bytes).into());
        let artifact = bundle_compressed(&store, bytes, identity).unwrap();
        store.verify(&artifact.blob).unwrap();
        assert_eq!(artifact.logical_identity_sha256, identity);
        assert_eq!(read_compressed(&store, &artifact).unwrap(), bytes);
        fs::remove_dir_all(root).unwrap();
    }
}
