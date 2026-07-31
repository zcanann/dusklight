use super::fault_recovery_audit::{
    build_native_tactic_fault_recovery_audit, read_fault_marker_source,
};
use super::throughput_evidence_bundle::{bundle_compressed, read_compressed};
use super::*;
use crate::native_residual_campaign::NativeResidualExecutionBinding;
use dusklight_evidence::content_store::{ContentKind, ContentStore};
use dusklight_harness_contracts::objective_suite::ArtifactReference;

pub const NATIVE_TACTIC_FAULT_RECOVERY_EVIDENCE_BUNDLE_SCHEMA_V1: &str =
    "dusklight-native-tactic-fault-recovery-evidence-bundle/v1";
pub const NATIVE_TACTIC_FAULT_RECOVERY_EVIDENCE_MANIFEST: &str = "manifest.json";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticFaultRecoveryEvidenceBundle {
    pub schema: String,
    pub content_sha256: Digest,
    pub optimization_request: NativeTacticScratchBundleArtifact,
    pub execution_binding: NativeTacticScratchBundleArtifact,
    pub execution_plan: NativeTacticScratchBundleArtifact,
    pub execution_identity: NativeTacticScratchExecutionIdentity,
    pub authorities: Vec<NativeTacticScratchAuthorityArtifact>,
    pub control_report: NativeTacticThroughputCompressedArtifact,
    pub recovered_report: NativeTacticThroughputCompressedArtifact,
    pub control_resource_audit: NativeTacticThroughputCompressedArtifact,
    pub recovered_resource_audit: NativeTacticThroughputCompressedArtifact,
    pub fault_marker: NativeTacticScratchBundleArtifact,
    pub fault_recovery_audit: NativeTacticScratchBundleArtifact,
    pub passed: bool,
}

impl NativeTacticFaultRecoveryEvidenceBundle {
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        bundle_root: &Path,
        repository_root: &Path,
        request_path: &Path,
        execution_path: &Path,
        control_report_path: &Path,
        recovered_report_path: &Path,
    ) -> Result<Self, NativeTacticRouteRunError> {
        if bundle_root.exists() {
            return Err(route_message(
                "native tactic fault-recovery evidence bundle output already exists",
            ));
        }
        let repository_root = repository_root.canonicalize().map_err(route_error)?;
        let request_path = confined_source(&repository_root, request_path)?;
        let execution_path = confined_source(&repository_root, execution_path)?;
        let control_report_path = confined_source(&repository_root, control_report_path)?;
        let recovered_report_path = confined_source(&repository_root, recovered_report_path)?;
        let (marker_path, marker) = read_fault_marker_source(&recovered_report_path)?;
        let marker_path = confined_source(&repository_root, &marker_path)?;

        let request_bytes = fs::read(&request_path).map_err(route_error)?;
        let execution_bytes = fs::read(&execution_path).map_err(route_error)?;
        let control_bytes = fs::read(&control_report_path).map_err(route_error)?;
        let recovered_bytes = fs::read(&recovered_report_path).map_err(route_error)?;
        let marker_bytes = fs::read(&marker_path).map_err(route_error)?;
        let request: OptimizationRequest =
            serde_json::from_slice(&request_bytes).map_err(route_error)?;
        let execution: NativeResidualExecutionBinding =
            serde_json::from_slice(&execution_bytes).map_err(route_error)?;
        let control: NativeTacticRouteReport =
            serde_json::from_slice(&control_bytes).map_err(route_error)?;
        let recovered: NativeTacticRouteReport =
            serde_json::from_slice(&recovered_bytes).map_err(route_error)?;
        request.validate().map_err(route_error)?;
        execution.validate_seal(&request).map_err(route_error)?;
        validate_route_authorities(&request, &execution, &control, &recovered)?;

        let control_plan_path =
            confined_source(&repository_root, Path::new(&control.execution_plan_path))?;
        let recovered_plan_path =
            confined_source(&repository_root, Path::new(&recovered.execution_plan_path))?;
        let plan = NativeTacticExecutionPlan::read(&control_plan_path)?;
        let recovered_plan = NativeTacticExecutionPlan::read(&recovered_plan_path)?;
        if plan.identity()? != control.execution_plan_sha256
            || recovered_plan.identity()? != control.execution_plan_sha256
        {
            return Err(route_message(
                "native tactic fault-recovery execution plans differ",
            ));
        }

        let audit =
            build_native_tactic_fault_recovery_audit(&control_bytes, &recovered_bytes, marker)?;
        let control_resource = NativeTacticScratchCampaignAudit::build(&repository_root, &control)?;
        let recovered_resource =
            NativeTacticScratchCampaignAudit::build(&repository_root, &recovered)?;
        control_resource.validate_resource_binding(&control, &plan)?;
        recovered_resource.validate_resource_binding(&recovered, &plan)?;

        let store = ContentStore::initialize(bundle_root).map_err(route_error)?;
        let mut guard = BundleBuildGuard::new(bundle_root);
        let optimization_request = bundle_bytes(
            &store,
            &request_bytes,
            ContentKind::DatasetManifest,
            request.content_sha256,
        )?;
        let execution_binding = bundle_bytes(
            &store,
            &execution_bytes,
            ContentKind::DatasetManifest,
            execution.content_sha256,
        )?;
        let execution_plan = bundle_file(
            &store,
            &control_plan_path,
            ContentKind::DatasetManifest,
            plan.identity()?,
        )?;
        let mut authorities = bundle_authorities(&store, &repository_root, &request, &execution)?;
        authorities.sort_by(|left, right| left.role.cmp(&right.role));
        let control_report = bundle_compressed(
            &store,
            &control_bytes,
            Digest(Sha256::digest(&control_bytes).into()),
        )?;
        let recovered_report = bundle_compressed(
            &store,
            &recovered_bytes,
            Digest(Sha256::digest(&recovered_bytes).into()),
        )?;
        let control_resource_audit = bundle_compressed(
            &store,
            &control_resource.to_pretty_json()?,
            control_resource.content_sha256,
        )?;
        let recovered_resource_audit = bundle_compressed(
            &store,
            &recovered_resource.to_pretty_json()?,
            recovered_resource.content_sha256,
        )?;
        let fault_marker = bundle_bytes(
            &store,
            &marker_bytes,
            ContentKind::DatasetManifest,
            Digest(Sha256::digest(&marker_bytes).into()),
        )?;
        let fault_recovery_audit = bundle_bytes(
            &store,
            &serde_json::to_vec_pretty(&audit).map_err(route_error)?,
            ContentKind::DatasetManifest,
            audit.content_sha256,
        )?;

        let mut bundle = Self {
            schema: NATIVE_TACTIC_FAULT_RECOVERY_EVIDENCE_BUNDLE_SCHEMA_V1.into(),
            content_sha256: Digest::ZERO,
            optimization_request,
            execution_binding,
            execution_plan,
            execution_identity: execution_identity(&execution),
            authorities,
            control_report,
            recovered_report,
            control_resource_audit,
            recovered_resource_audit,
            fault_marker,
            fault_recovery_audit,
            passed: audit.passed,
        };
        bundle.content_sha256 = bundle.compute_content_sha256()?;
        bundle.validate(bundle_root)?;
        let mut bytes = serde_json::to_vec_pretty(&bundle).map_err(route_error)?;
        bytes.push(b'\n');
        fs::write(
            bundle_root.join(NATIVE_TACTIC_FAULT_RECOVERY_EVIDENCE_MANIFEST),
            bytes,
        )
        .map_err(route_error)?;
        guard.commit();
        Ok(bundle)
    }

    pub fn read_and_validate(bundle_root: &Path) -> Result<Self, NativeTacticRouteRunError> {
        let bundle: Self = serde_json::from_slice(
            &fs::read(bundle_root.join(NATIVE_TACTIC_FAULT_RECOVERY_EVIDENCE_MANIFEST))
                .map_err(route_error)?,
        )
        .map_err(route_error)?;
        bundle.validate(bundle_root)?;
        Ok(bundle)
    }

    pub fn validate(&self, bundle_root: &Path) -> Result<(), NativeTacticRouteRunError> {
        if self.schema != NATIVE_TACTIC_FAULT_RECOVERY_EVIDENCE_BUNDLE_SCHEMA_V1
            || self.content_sha256 == Digest::ZERO
            || self.content_sha256 != self.compute_content_sha256()?
            || self.authorities.len() != 6
            || !self
                .authorities
                .windows(2)
                .all(|pair| pair[0].role < pair[1].role)
            || !self.passed
        {
            return Err(route_message(
                "native tactic fault-recovery evidence manifest is invalid",
            ));
        }
        let store = ContentStore::open(bundle_root).map_err(route_error)?;
        for artifact in [
            &self.optimization_request,
            &self.execution_binding,
            &self.execution_plan,
            &self.fault_marker,
            &self.fault_recovery_audit,
        ] {
            store.verify(&artifact.blob).map_err(route_error)?;
        }
        for artifact in [
            &self.control_report,
            &self.recovered_report,
            &self.control_resource_audit,
            &self.recovered_resource_audit,
        ] {
            store.verify(&artifact.blob).map_err(route_error)?;
        }
        for authority in &self.authorities {
            store.verify(&authority.blob).map_err(route_error)?;
            if authority.blob.sha256 != authority.declared_sha256 {
                return Err(route_message(
                    "native tactic fault-recovery authority digest differs",
                ));
            }
        }

        let request: OptimizationRequest = read_json(&store, &self.optimization_request)?;
        let execution: NativeResidualExecutionBinding = read_json(&store, &self.execution_binding)?;
        let plan =
            NativeTacticExecutionPlan::read(&store.blob_path(self.execution_plan.blob.sha256))?;
        let marker_bytes = store
            .read_bytes(&self.fault_marker.blob)
            .map_err(route_error)?;
        let marker: NativeTacticFaultInjectionMarker =
            serde_json::from_slice(&marker_bytes).map_err(route_error)?;
        let audit: NativeTacticFaultRecoveryAudit = read_json(&store, &self.fault_recovery_audit)?;
        let control_bytes = read_compressed(&store, &self.control_report)?;
        let recovered_bytes = read_compressed(&store, &self.recovered_report)?;
        let control: NativeTacticRouteReport =
            serde_json::from_slice(&control_bytes).map_err(route_error)?;
        let recovered: NativeTacticRouteReport =
            serde_json::from_slice(&recovered_bytes).map_err(route_error)?;
        let control_resource_bytes = read_compressed(&store, &self.control_resource_audit)?;
        let recovered_resource_bytes = read_compressed(&store, &self.recovered_resource_audit)?;
        let control_resource: NativeTacticScratchCampaignAudit =
            serde_json::from_slice(&control_resource_bytes).map_err(route_error)?;
        let recovered_resource: NativeTacticScratchCampaignAudit =
            serde_json::from_slice(&recovered_resource_bytes).map_err(route_error)?;

        request.validate().map_err(route_error)?;
        execution.validate_seal(&request).map_err(route_error)?;
        marker.validate()?;
        audit.validate()?;
        control_resource.validate()?;
        recovered_resource.validate()?;
        control_resource.validate_resource_binding(&control, &plan)?;
        recovered_resource.validate_resource_binding(&recovered, &plan)?;
        validate_route_authorities(&request, &execution, &control, &recovered)?;
        let recomputed =
            build_native_tactic_fault_recovery_audit(&control_bytes, &recovered_bytes, marker)?;
        if audit != recomputed
            || self.optimization_request.logical_identity_sha256 != request.content_sha256
            || self.execution_binding.logical_identity_sha256 != execution.content_sha256
            || self.execution_plan.logical_identity_sha256 != plan.identity()?
            || self.fault_marker.logical_identity_sha256
                != Digest(Sha256::digest(&marker_bytes).into())
            || self.fault_recovery_audit.logical_identity_sha256 != audit.content_sha256
            || self.control_report.logical_identity_sha256 != audit.control_report_sha256
            || self.recovered_report.logical_identity_sha256 != audit.recovered_report_sha256
            || self.control_resource_audit.logical_identity_sha256
                != control_resource.content_sha256
            || self.recovered_resource_audit.logical_identity_sha256
                != recovered_resource.content_sha256
            || self.execution_identity != execution_identity(&execution)
            || !authorities_match(&self.authorities, &request, &execution)
            || self.passed != audit.passed
        {
            return Err(route_message(
                "native tactic fault-recovery evidence authorities are detached",
            ));
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

fn validate_route_authorities(
    request: &OptimizationRequest,
    execution: &NativeResidualExecutionBinding,
    control: &NativeTacticRouteReport,
    recovered: &NativeTacticRouteReport,
) -> Result<(), NativeTacticRouteRunError> {
    if control.schema != NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V38
        || recovered.schema != NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V38
        || control.optimization_request_sha256 != request.content_sha256
        || recovered.optimization_request_sha256 != request.content_sha256
        || control.execution_binding_sha256 != execution.content_sha256
        || recovered.execution_binding_sha256 != execution.content_sha256
        || control.execution_plan_sha256 != recovered.execution_plan_sha256
        || control.seeds.len() != 1
        || recovered.seeds.len() != 1
    {
        return Err(route_message(
            "native tactic fault-recovery reports are detached from their authorities",
        ));
    }
    Ok(())
}

fn bundle_authorities(
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
            "native tactic fault-recovery authority bytes differ",
        ));
    }
    Ok(NativeTacticScratchAuthorityArtifact {
        role: role.into(),
        declared_path: reference.path.clone(),
        declared_sha256: reference.sha256,
        blob,
    })
}

fn authorities_match(
    authorities: &[NativeTacticScratchAuthorityArtifact],
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
    authorities
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

fn bundle_bytes(
    store: &ContentStore,
    bytes: &[u8],
    kind: ContentKind,
    logical_identity_sha256: Digest,
) -> Result<NativeTacticScratchBundleArtifact, NativeTacticRouteRunError> {
    Ok(NativeTacticScratchBundleArtifact {
        logical_identity_sha256,
        blob: store.put_bytes(bytes, kind).map_err(route_error)?,
    })
}

fn read_json<T: for<'de> Deserialize<'de>>(
    store: &ContentStore,
    artifact: &NativeTacticScratchBundleArtifact,
) -> Result<T, NativeTacticRouteRunError> {
    serde_json::from_slice(&store.read_bytes(&artifact.blob).map_err(route_error)?)
        .map_err(route_error)
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
            "native tactic fault-recovery source is outside the repository or not a file",
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

    #[test]
    fn fault_recovery_bundle_schema_is_distinct_from_audit_and_throughput() {
        assert_ne!(
            NATIVE_TACTIC_FAULT_RECOVERY_EVIDENCE_BUNDLE_SCHEMA_V1,
            NATIVE_TACTIC_FAULT_RECOVERY_AUDIT_SCHEMA_V2
        );
        assert_ne!(
            NATIVE_TACTIC_FAULT_RECOVERY_EVIDENCE_BUNDLE_SCHEMA_V1,
            NATIVE_TACTIC_THROUGHPUT_EVIDENCE_BUNDLE_SCHEMA_V1
        );
    }
}
