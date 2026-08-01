use super::scratch_discovery::route_report_sha256;
use super::throughput_evidence_bundle::{bundle_compressed, read_compressed};
use super::*;
use crate::native_residual_campaign::NativeResidualExecutionBinding;
use dusklight_evidence::content_store::{ContentBlob, ContentStore};

pub const NATIVE_TACTIC_THROUGHPUT_TREATMENT_BUNDLE_SCHEMA_V1: &str =
    "dusklight-native-tactic-throughput-treatment-bundle/v1";
pub const NATIVE_TACTIC_THROUGHPUT_TREATMENT_MANIFEST: &str = "manifest.json";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticThroughputTreatmentSampleEvidence {
    pub route_report: NativeTacticThroughputCompressedArtifact,
    pub campaign_audit: NativeTacticThroughputCompressedArtifact,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticThroughputTreatmentBundle {
    pub schema: String,
    pub content_sha256: Digest,
    pub control_curve_bundle_sha256: Digest,
    pub control_sample_ordinal: u32,
    pub optimization_request: NativeTacticScratchBundleArtifact,
    pub execution_binding: NativeTacticScratchBundleArtifact,
    pub execution_plan: NativeTacticScratchBundleArtifact,
    pub execution_identity: NativeTacticScratchExecutionIdentity,
    pub authorities: Vec<NativeTacticScratchAuthorityArtifact>,
    pub control: NativeTacticThroughputTreatmentSampleEvidence,
    pub treatment: NativeTacticThroughputTreatmentSampleEvidence,
    pub audit: NativeTacticThroughputTreatmentAudit,
    pub passed: bool,
}

impl NativeTacticThroughputTreatmentBundle {
    pub fn build(
        bundle_root: &Path,
        repository_root: &Path,
        control_curve_bundle_root: &Path,
        control_sample_ordinal: u32,
        treatment_report_path: &Path,
    ) -> Result<Self, NativeTacticRouteRunError> {
        if bundle_root.exists() {
            return Err(route_message(
                "native tactic throughput treatment bundle output already exists",
            ));
        }
        let repository_root = repository_root.canonicalize().map_err(route_error)?;
        let treatment_report_path =
            confined_file(&repository_root, treatment_report_path, "treatment report")?;
        let control_curve_bundle =
            NativeTacticThroughputEvidenceBundle::read_and_validate(control_curve_bundle_root)?;
        let control_sample = control_curve_bundle
            .samples
            .iter()
            .find(|sample| sample.ordinal == control_sample_ordinal)
            .ok_or_else(|| {
                route_message("native tactic throughput control sample ordinal is absent")
            })?;
        let control_store = ContentStore::open(control_curve_bundle_root).map_err(route_error)?;
        let control_route_bytes = read_compressed(&control_store, &control_sample.route_report)?;
        let control_audit_bytes = read_compressed(&control_store, &control_sample.campaign_audit)?;
        let treatment_route_bytes = fs::read(&treatment_report_path).map_err(route_error)?;
        let control_route: NativeTacticRouteReport =
            serde_json::from_slice(&control_route_bytes).map_err(route_error)?;
        let treatment_route: NativeTacticRouteReport =
            serde_json::from_slice(&treatment_route_bytes).map_err(route_error)?;
        let control_audit: NativeTacticScratchCampaignAudit =
            serde_json::from_slice(&control_audit_bytes).map_err(route_error)?;
        let request: OptimizationRequest =
            read_bundle_json(&control_store, &control_curve_bundle.optimization_request)?;
        let execution: NativeResidualExecutionBinding =
            read_bundle_json(&control_store, &control_curve_bundle.execution_binding)?;
        let plan = NativeTacticExecutionPlan::read(
            &control_store.blob_path(control_curve_bundle.execution_plan.blob.sha256),
        )?;
        request.validate().map_err(route_error)?;
        execution.validate_seal(&request).map_err(route_error)?;
        validate_route_authorities(&control_route, &request, &execution, &plan)?;
        validate_route_authorities(&treatment_route, &request, &execution, &plan)?;
        control_audit.validate()?;
        control_audit.validate_resource_binding(&control_route, &plan)?;
        let treatment_audit =
            NativeTacticScratchCampaignAudit::build(&repository_root, &treatment_route)?;
        treatment_audit.validate_resource_binding(&treatment_route, &plan)?;
        let audit = NativeTacticThroughputTreatmentAudit::build(
            &control_route_bytes,
            &treatment_route_bytes,
        )?;

        let store = ContentStore::initialize(bundle_root).map_err(route_error)?;
        let mut guard = TreatmentBundleBuildGuard::new(bundle_root);
        let optimization_request = copy_artifact(
            &control_store,
            &store,
            &control_curve_bundle.optimization_request,
        )?;
        let execution_binding = copy_artifact(
            &control_store,
            &store,
            &control_curve_bundle.execution_binding,
        )?;
        let execution_plan =
            copy_artifact(&control_store, &store, &control_curve_bundle.execution_plan)?;
        let authorities = control_curve_bundle
            .authorities
            .iter()
            .map(|authority| copy_authority(&control_store, &store, authority))
            .collect::<Result<Vec<_>, _>>()?;
        let control = NativeTacticThroughputTreatmentSampleEvidence {
            route_report: bundle_compressed(
                &store,
                &control_route_bytes,
                audit.control_report_sha256,
            )?,
            campaign_audit: bundle_compressed(
                &store,
                &control_audit_bytes,
                control_audit.content_sha256,
            )?,
        };
        let treatment = NativeTacticThroughputTreatmentSampleEvidence {
            route_report: bundle_compressed(
                &store,
                &treatment_route_bytes,
                audit.treatment_report_sha256,
            )?,
            campaign_audit: bundle_compressed(
                &store,
                &treatment_audit.to_pretty_json()?,
                treatment_audit.content_sha256,
            )?,
        };
        let mut bundle = Self {
            schema: NATIVE_TACTIC_THROUGHPUT_TREATMENT_BUNDLE_SCHEMA_V1.into(),
            content_sha256: Digest::ZERO,
            control_curve_bundle_sha256: control_curve_bundle.content_sha256,
            control_sample_ordinal,
            optimization_request,
            execution_binding,
            execution_plan,
            execution_identity: control_curve_bundle.execution_identity,
            authorities,
            control,
            treatment,
            passed: audit.passed,
            audit,
        };
        bundle.content_sha256 = bundle.compute_content_sha256()?;
        bundle.validate(bundle_root)?;
        let mut bytes = serde_json::to_vec_pretty(&bundle).map_err(route_error)?;
        bytes.push(b'\n');
        fs::write(
            bundle_root.join(NATIVE_TACTIC_THROUGHPUT_TREATMENT_MANIFEST),
            bytes,
        )
        .map_err(route_error)?;
        guard.commit();
        Ok(bundle)
    }

    pub fn read_and_validate(bundle_root: &Path) -> Result<Self, NativeTacticRouteRunError> {
        let bundle: Self = serde_json::from_slice(
            &fs::read(bundle_root.join(NATIVE_TACTIC_THROUGHPUT_TREATMENT_MANIFEST))
                .map_err(route_error)?,
        )
        .map_err(route_error)?;
        bundle.validate(bundle_root)?;
        Ok(bundle)
    }

    pub fn validate(&self, bundle_root: &Path) -> Result<(), NativeTacticRouteRunError> {
        if self.schema != NATIVE_TACTIC_THROUGHPUT_TREATMENT_BUNDLE_SCHEMA_V1
            || self.content_sha256 == Digest::ZERO
            || self.content_sha256 != self.compute_content_sha256()?
            || self.control_curve_bundle_sha256 == Digest::ZERO
            || self.control_sample_ordinal == 0
            || self.authorities.len() != 6
            || !self
                .authorities
                .windows(2)
                .all(|pair| pair[0].role < pair[1].role)
            || !self.passed
            || !self.audit.passed
        {
            return Err(route_message(
                "native tactic throughput treatment bundle manifest is invalid",
            ));
        }
        self.audit.validate()?;
        let store = ContentStore::open(bundle_root).map_err(route_error)?;
        for artifact in [
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
                    "native tactic throughput treatment authority digest differs",
                ));
            }
        }
        for sample in [&self.control, &self.treatment] {
            store
                .verify(&sample.route_report.blob)
                .map_err(route_error)?;
            store
                .verify(&sample.campaign_audit.blob)
                .map_err(route_error)?;
        }

        let request: OptimizationRequest = read_bundle_json(&store, &self.optimization_request)?;
        let execution: NativeResidualExecutionBinding =
            read_bundle_json(&store, &self.execution_binding)?;
        let plan =
            NativeTacticExecutionPlan::read(&store.blob_path(self.execution_plan.blob.sha256))?;
        request.validate().map_err(route_error)?;
        execution.validate_seal(&request).map_err(route_error)?;
        if self.optimization_request.logical_identity_sha256 != request.content_sha256
            || self.execution_binding.logical_identity_sha256 != execution.content_sha256
            || self.execution_plan.logical_identity_sha256 != plan.identity()?
            || self.execution_identity != treatment_execution_identity(&execution)
            || !treatment_authorities_match(self, &request, &execution)
        {
            return Err(route_message(
                "native tactic throughput treatment authorities are detached",
            ));
        }

        let control_route_bytes = read_compressed(&store, &self.control.route_report)?;
        let treatment_route_bytes = read_compressed(&store, &self.treatment.route_report)?;
        let control_audit_bytes = read_compressed(&store, &self.control.campaign_audit)?;
        let treatment_audit_bytes = read_compressed(&store, &self.treatment.campaign_audit)?;
        let control_route: NativeTacticRouteReport =
            serde_json::from_slice(&control_route_bytes).map_err(route_error)?;
        let treatment_route: NativeTacticRouteReport =
            serde_json::from_slice(&treatment_route_bytes).map_err(route_error)?;
        let control_audit: NativeTacticScratchCampaignAudit =
            serde_json::from_slice(&control_audit_bytes).map_err(route_error)?;
        let treatment_audit: NativeTacticScratchCampaignAudit =
            serde_json::from_slice(&treatment_audit_bytes).map_err(route_error)?;
        validate_route_authorities(&control_route, &request, &execution, &plan)?;
        validate_route_authorities(&treatment_route, &request, &execution, &plan)?;
        control_audit.validate()?;
        treatment_audit.validate()?;
        control_audit.validate_resource_binding(&control_route, &plan)?;
        treatment_audit.validate_resource_binding(&treatment_route, &plan)?;
        let recomputed = NativeTacticThroughputTreatmentAudit::build(
            &control_route_bytes,
            &treatment_route_bytes,
        )?;
        if recomputed != self.audit
            || self.control.route_report.logical_identity_sha256 != self.audit.control_report_sha256
            || self.treatment.route_report.logical_identity_sha256
                != self.audit.treatment_report_sha256
            || self.control.campaign_audit.logical_identity_sha256 != control_audit.content_sha256
            || self.treatment.campaign_audit.logical_identity_sha256
                != treatment_audit.content_sha256
            || control_audit.route_report_sha256 != route_report_sha256(&control_route)?
            || treatment_audit.route_report_sha256 != route_report_sha256(&treatment_route)?
        {
            return Err(route_message(
                "native tactic throughput treatment evidence is detached",
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
    route: &NativeTacticRouteReport,
    request: &OptimizationRequest,
    execution: &NativeResidualExecutionBinding,
    plan: &NativeTacticExecutionPlan,
) -> Result<(), NativeTacticRouteRunError> {
    if !supports_current_route_report_schema(&route.schema)
        || route.optimization_request_sha256 != request.content_sha256
        || route.execution_binding_sha256 != execution.content_sha256
        || route.execution_plan_sha256 != plan.identity()?
        || route.exploration_seeds != plan.seeds
        || route.decisions_per_seed != plan.budgets.decisions_per_lane
        || route.resource_budgets != plan.budgets
    {
        return Err(route_message(
            "native tactic throughput treatment route authority differs",
        ));
    }
    Ok(())
}

fn copy_artifact(
    source: &ContentStore,
    destination: &ContentStore,
    artifact: &NativeTacticScratchBundleArtifact,
) -> Result<NativeTacticScratchBundleArtifact, NativeTacticRouteRunError> {
    Ok(NativeTacticScratchBundleArtifact {
        logical_identity_sha256: artifact.logical_identity_sha256,
        blob: copy_blob(source, destination, &artifact.blob)?,
    })
}

fn copy_authority(
    source: &ContentStore,
    destination: &ContentStore,
    authority: &NativeTacticScratchAuthorityArtifact,
) -> Result<NativeTacticScratchAuthorityArtifact, NativeTacticRouteRunError> {
    Ok(NativeTacticScratchAuthorityArtifact {
        role: authority.role.clone(),
        declared_path: authority.declared_path.clone(),
        declared_sha256: authority.declared_sha256,
        blob: copy_blob(source, destination, &authority.blob)?,
    })
}

fn copy_blob(
    source: &ContentStore,
    destination: &ContentStore,
    blob: &ContentBlob,
) -> Result<ContentBlob, NativeTacticRouteRunError> {
    source.verify(blob).map_err(route_error)?;
    let copied = destination
        .put_bytes(&source.read_bytes(blob).map_err(route_error)?, blob.kind)
        .map_err(route_error)?;
    if copied.sha256 != blob.sha256 || copied.size != blob.size {
        return Err(route_message(
            "native tactic throughput treatment blob copy differs",
        ));
    }
    Ok(copied)
}

fn read_bundle_json<T: for<'de> Deserialize<'de>>(
    store: &ContentStore,
    artifact: &NativeTacticScratchBundleArtifact,
) -> Result<T, NativeTacticRouteRunError> {
    serde_json::from_slice(&store.read_bytes(&artifact.blob).map_err(route_error)?)
        .map_err(route_error)
}

fn confined_file(
    repository_root: &Path,
    declared: &Path,
    label: &str,
) -> Result<PathBuf, NativeTacticRouteRunError> {
    let candidate = if declared.is_absolute() {
        declared.to_path_buf()
    } else {
        repository_root.join(declared)
    };
    let resolved = candidate.canonicalize().map_err(route_error)?;
    if !resolved.starts_with(repository_root) || !resolved.is_file() {
        return Err(route_message(format!(
            "native tactic throughput {label} is outside the repository or not a file"
        )));
    }
    Ok(resolved)
}

fn treatment_execution_identity(
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

fn treatment_authorities_match(
    bundle: &NativeTacticThroughputTreatmentBundle,
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

struct TreatmentBundleBuildGuard {
    root: PathBuf,
    committed: bool,
}

impl TreatmentBundleBuildGuard {
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

impl Drop for TreatmentBundleBuildGuard {
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
    fn treatment_bundle_schema_is_distinct_from_the_curve_bundle() {
        assert_ne!(
            NATIVE_TACTIC_THROUGHPUT_TREATMENT_BUNDLE_SCHEMA_V1,
            NATIVE_TACTIC_THROUGHPUT_EVIDENCE_BUNDLE_SCHEMA_V1
        );
    }
}
