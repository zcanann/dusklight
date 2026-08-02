//! Portable authority for one authenticated terminal discovered by a tactic campaign.

use super::scratch_discovery::route_report_sha256;
use super::scratch_evidence_bundle::{
    blob_path, bundle_authority, bundle_file, bundle_seed, confined_source, execution_identity,
    read_json_blob, validate_seed,
};
use super::*;
use crate::native_residual_campaign::NativeResidualExecutionBinding;
use dusklight_evidence::content_store::{ContentKind, ContentStore};
use sha2::Sha256;

pub const NATIVE_TACTIC_TERMINAL_EVIDENCE_BUNDLE_SCHEMA_V1: &str =
    "dusklight-native-tactic-terminal-evidence-bundle/v1";
pub const NATIVE_TACTIC_TERMINAL_EVIDENCE_MANIFEST: &str = "manifest.json";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticTerminalEvidenceBundle {
    pub schema: String,
    pub content_sha256: Digest,
    pub optimization_request: NativeTacticScratchBundleArtifact,
    pub execution_binding: NativeTacticScratchBundleArtifact,
    pub execution_plan: NativeTacticScratchBundleArtifact,
    pub route_report: NativeTacticScratchBundleArtifact,
    pub campaign_audit: NativeTacticScratchBundleArtifact,
    pub execution_identity: NativeTacticScratchExecutionIdentity,
    pub authorities: Vec<NativeTacticScratchAuthorityArtifact>,
    pub terminal: NativeTacticScratchSeedEvidence,
    pub authenticated_terminal: bool,
}

impl NativeTacticTerminalEvidenceBundle {
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        bundle_root: &Path,
        repository_root: &Path,
        request_path: &Path,
        execution_path: &Path,
        route_report_path: &Path,
        request: &OptimizationRequest,
        execution: &NativeResidualExecutionBinding,
        route: &NativeTacticRouteReport,
        seed: u64,
    ) -> Result<Self, NativeTacticRouteRunError> {
        if bundle_root.exists() {
            return Err(route_message(
                "terminal evidence bundle output already exists",
            ));
        }
        request.validate().map_err(route_error)?;
        execution.validate_seal(request).map_err(route_error)?;
        let route_sha256 = route_report_sha256(route)?;
        if execution.content_sha256 != route.execution_binding_sha256
            || route.optimization_request_sha256 != request.content_sha256
        {
            return Err(route_message(
                "terminal evidence inputs do not share one execution authority",
            ));
        }
        let reported = route
            .seeds
            .iter()
            .find(|reported| reported.seed == seed)
            .ok_or_else(|| route_message("selected terminal seed is absent from route report"))?;
        if !reported.terminal_discovered
            || reported.best_authenticated_tick.is_none()
            || reported.best_terminal_tape.is_none()
            || reported.best_terminal_result.is_none()
        {
            return Err(route_message(
                "selected campaign seed has no authenticated terminal",
            ));
        }

        let repository_root = repository_root.canonicalize().map_err(route_error)?;
        let request_path = confined_source(&repository_root, request_path)?;
        let execution_path = confined_source(&repository_root, execution_path)?;
        let route_report_path = confined_source(&repository_root, route_report_path)?;
        let plan_path = confined_source(&repository_root, Path::new(&route.execution_plan_path))?;
        let plan = NativeTacticExecutionPlan::read(&plan_path)?;
        let plan_sha256 = plan.identity()?;
        if plan_sha256 != route.execution_plan_sha256 {
            return Err(route_message(
                "terminal evidence execution plan identity differs",
            ));
        }

        let store = ContentStore::initialize(bundle_root).map_err(route_error)?;
        let mut guard = TerminalBundleBuildGuard::new(bundle_root.to_path_buf());
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
            plan_sha256,
        )?;
        let route_report = bundle_file(
            &store,
            &route_report_path,
            ContentKind::DatasetManifest,
            route_sha256,
        )?;
        let campaign_audit_report =
            NativeTacticScratchCampaignAudit::build(&repository_root, route)?;
        let campaign_audit = super::scratch_evidence_bundle::bundle_bytes(
            &store,
            &campaign_audit_report.to_pretty_json()?,
            ContentKind::DatasetManifest,
            campaign_audit_report.content_sha256,
        )?;
        let mut authorities = vec![
            bundle_authority(
                &store,
                &repository_root,
                "route_timeline",
                &request.route.timeline,
                ContentKind::DatasetManifest,
            )?,
            bundle_authority(
                &store,
                &repository_root,
                "terminal_predicate_source",
                &request.terminal_predicate.source,
                ContentKind::DatasetManifest,
            )?,
            bundle_authority(
                &store,
                &repository_root,
                "process_boot_tape",
                &execution.process_boot_tape,
                ContentKind::InputTape,
            )?,
            bundle_authority(
                &store,
                &repository_root,
                "milestone_program",
                &execution.milestone_program,
                ContentKind::DatasetManifest,
            )?,
            bundle_authority(
                &store,
                &repository_root,
                "world_context",
                &execution.world_context,
                ContentKind::WorldContext,
            )?,
            bundle_authority(
                &store,
                &repository_root,
                "card_fixture_manifest",
                &execution.card_fixture_manifest,
                ContentKind::DatasetManifest,
            )?,
        ];
        authorities.sort_by(|left, right| left.role.cmp(&right.role));
        let terminal = bundle_seed(
            &store,
            &repository_root,
            request.route.source_boundary_index,
            route,
            reported,
        )?;
        let mut bundle = Self {
            schema: NATIVE_TACTIC_TERMINAL_EVIDENCE_BUNDLE_SCHEMA_V1.into(),
            content_sha256: Digest::ZERO,
            optimization_request,
            execution_binding,
            execution_plan,
            route_report,
            campaign_audit,
            execution_identity: execution_identity(execution),
            authorities,
            terminal,
            authenticated_terminal: true,
        };
        bundle.content_sha256 = bundle.identity()?;
        bundle.validate(bundle_root)?;
        let mut bytes = serde_json::to_vec_pretty(&bundle).map_err(route_error)?;
        bytes.push(b'\n');
        fs::write(
            bundle_root.join(NATIVE_TACTIC_TERMINAL_EVIDENCE_MANIFEST),
            bytes,
        )
        .map_err(route_error)?;
        guard.commit();
        Ok(bundle)
    }

    pub fn read_and_validate(bundle_root: &Path) -> Result<Self, NativeTacticRouteRunError> {
        let bundle: Self = serde_json::from_slice(
            &fs::read(bundle_root.join(NATIVE_TACTIC_TERMINAL_EVIDENCE_MANIFEST))
                .map_err(route_error)?,
        )
        .map_err(route_error)?;
        bundle.validate(bundle_root)?;
        Ok(bundle)
    }

    pub fn validate(&self, bundle_root: &Path) -> Result<(), NativeTacticRouteRunError> {
        if self.schema != NATIVE_TACTIC_TERMINAL_EVIDENCE_BUNDLE_SCHEMA_V1
            || self.content_sha256 == Digest::ZERO
            || self.content_sha256 != self.identity()?
            || !self.authenticated_terminal
            || !self.terminal.terminal_discovered
            || self.terminal.best_authenticated_tick.is_none()
            || self.terminal.best_terminal_tape.is_none()
            || self.terminal.best_terminal_result.is_none()
            || self.authorities.is_empty()
            || !self
                .authorities
                .windows(2)
                .all(|pair| pair[0].role < pair[1].role)
        {
            return Err(route_message(
                "terminal evidence bundle manifest is invalid",
            ));
        }
        let store = ContentStore::open(bundle_root).map_err(route_error)?;
        for artifact in self.artifacts() {
            store.verify(&artifact.blob).map_err(route_error)?;
        }
        for authority in &self.authorities {
            store.verify(&authority.blob).map_err(route_error)?;
            if authority.declared_sha256 != authority.blob.sha256 {
                return Err(route_message(
                    "terminal evidence authority digest differs from its blob",
                ));
            }
        }

        let request: OptimizationRequest = read_json_blob(bundle_root, &self.optimization_request)?;
        request.validate().map_err(route_error)?;
        let execution: NativeResidualExecutionBinding =
            read_json_blob(bundle_root, &self.execution_binding)?;
        execution.validate_seal(&request).map_err(route_error)?;
        let plan =
            NativeTacticExecutionPlan::read(&blob_path(bundle_root, &self.execution_plan.blob))?;
        let route: NativeTacticRouteReport = read_json_blob(bundle_root, &self.route_report)?;
        let audit: NativeTacticScratchCampaignAudit =
            read_json_blob(bundle_root, &self.campaign_audit)?;
        audit.validate()?;
        audit.validate_resource_binding(&route, &plan)?;
        if self.optimization_request.logical_identity_sha256 != request.content_sha256
            || self.execution_binding.logical_identity_sha256 != execution.content_sha256
            || self.execution_plan.logical_identity_sha256 != plan.identity()?
            || self.route_report.logical_identity_sha256 != route_report_sha256(&route)?
            || self.campaign_audit.logical_identity_sha256 != audit.content_sha256
            || route.optimization_request_sha256 != request.content_sha256
            || route.execution_binding_sha256 != execution.content_sha256
            || route.execution_plan_sha256 != plan.identity()?
            || audit.route_report_sha256 != route_report_sha256(&route)?
            || audit.execution_plan_sha256 != route.execution_plan_sha256
            || audit.objective_sha256 != route.objective_sha256
            || audit.execution_binding_sha256 != route.execution_binding_sha256
            || self.execution_identity != execution_identity(&execution)
        {
            return Err(route_message(
                "terminal evidence bundle authorities are detached",
            ));
        }
        validate_authorities(&self.authorities, &request, &execution)?;
        let reported = route
            .seeds
            .iter()
            .find(|reported| reported.seed == self.terminal.seed)
            .ok_or_else(|| route_message("bundled terminal seed is absent from route report"))?;
        let audited = audit
            .seeds
            .iter()
            .find(|audited| audited.seed == self.terminal.seed)
            .ok_or_else(|| route_message("bundled terminal seed is absent from campaign audit"))?;
        validate_seed(
            bundle_root,
            request.route.source_boundary_index,
            &route,
            reported,
            &self.terminal,
            audited,
        )
    }

    fn artifacts(&self) -> Vec<&NativeTacticScratchBundleArtifact> {
        let mut artifacts = vec![
            &self.optimization_request,
            &self.execution_binding,
            &self.execution_plan,
            &self.route_report,
            &self.campaign_audit,
            &self.terminal.seed_result,
            &self.terminal.lease_journal,
            &self.terminal.checkpoint_envelope,
            &self.terminal.checkpoint_snapshot,
        ];
        if let Some(artifact) = &self.terminal.best_terminal_tape {
            artifacts.push(artifact);
        }
        if let Some(artifact) = &self.terminal.best_terminal_result {
            artifacts.push(artifact);
        }
        artifacts
    }

    fn identity(&self) -> Result<Digest, NativeTacticRouteRunError> {
        let mut unsigned = self.clone();
        unsigned.content_sha256 = Digest::ZERO;
        Ok(Digest(
            Sha256::digest(serde_json::to_vec(&unsigned).map_err(route_error)?).into(),
        ))
    }
}

fn validate_authorities(
    authorities: &[NativeTacticScratchAuthorityArtifact],
    request: &OptimizationRequest,
    execution: &NativeResidualExecutionBinding,
) -> Result<(), NativeTacticRouteRunError> {
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
    if authorities.len() != expected.len()
        || authorities
            .iter()
            .zip(expected)
            .any(|(actual, (role, reference))| {
                actual.role != role
                    || actual.declared_path != reference.path
                    || actual.declared_sha256 != reference.sha256
            })
    {
        return Err(route_message(
            "terminal evidence source authorities are incomplete",
        ));
    }
    Ok(())
}

struct TerminalBundleBuildGuard {
    root: PathBuf,
    committed: bool,
}

impl TerminalBundleBuildGuard {
    fn new(root: PathBuf) -> Self {
        Self {
            root,
            committed: false,
        }
    }

    fn commit(&mut self) {
        self.committed = true;
    }
}

impl Drop for TerminalBundleBuildGuard {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}
