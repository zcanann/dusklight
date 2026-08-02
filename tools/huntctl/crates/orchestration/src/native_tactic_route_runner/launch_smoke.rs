use super::scratch_discovery::route_report_sha256;
use super::*;
use crate::compact_suffix_batch::COMPACT_SUFFIX_BATCH_MAGIC;
use crate::native_residual_campaign::NativeResidualExecutionBinding;
use crate::native_suffix_result::{NATIVE_SUFFIX_BATCH_RESULT_SCHEMA_V9, NativeSuffixBatchResult};
use crate::tactic_q_campaign::{
    TacticQCampaign, TacticQCampaignCheckpoint, validate_checkpoint, validate_checkpoint_snapshot,
};
use dusklight_evidence::content_store::{ContentKind, ContentStore};
use dusklight_harness_contracts::objective_suite::ArtifactReference;
use dusklight_worker_protocol::client::HelloResponse;

pub const NATIVE_TACTIC_LAUNCH_SMOKE_BUNDLE_SCHEMA_V1: &str =
    "dusklight-native-tactic-launch-smoke-bundle/v1";
pub const NATIVE_TACTIC_LAUNCH_SMOKE_MANIFEST: &str = "manifest.json";
const MAXIMUM_SMOKE_CHECKPOINT_SNAPSHOT_BYTES: usize = 64 * 1024 * 1024;
const SMOKE_CHECKPOINT_COMPRESSION_LEVEL: i32 = 3;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticLaunchSmokeSummary {
    pub platform: String,
    pub architecture: String,
    pub worker_revision: String,
    pub worker_aurora_revision: String,
    pub clean_build: bool,
    pub executable_sha256: Digest,
    pub game_data_sha256: Digest,
    pub root_checkpoint_identity: String,
    pub source_boundary_index: u64,
    pub proposal_ticks: u64,
    pub checkpoint_cache_capacity_bytes: u64,
    pub completed_graph_leases: u64,
    pub unresolved_graph_leases: u64,
    pub compact_persistent_control: bool,
    pub resource_audit_passed: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticLaunchSmokeBundle {
    pub schema: String,
    pub content_sha256: Digest,
    pub optimization_request: NativeTacticScratchBundleArtifact,
    pub execution_binding: NativeTacticScratchBundleArtifact,
    pub execution_plan: NativeTacticScratchBundleArtifact,
    pub route_report: NativeTacticScratchBundleArtifact,
    pub campaign_audit: NativeTacticScratchBundleArtifact,
    pub worker_hello: NativeTacticScratchBundleArtifact,
    pub initial_request: NativeTacticScratchBundleArtifact,
    pub initial_result: NativeTacticScratchBundleArtifact,
    pub compact_proposal_request: NativeTacticScratchBundleArtifact,
    pub proposal_result: NativeTacticScratchBundleArtifact,
    pub lease_journal: NativeTacticScratchBundleArtifact,
    pub checkpoint_snapshot: NativeTacticScratchBundleArtifact,
    pub checkpoint_snapshot_uncompressed_bytes: u64,
    pub execution_identity: NativeTacticScratchExecutionIdentity,
    pub authorities: Vec<NativeTacticScratchAuthorityArtifact>,
    pub summary: NativeTacticLaunchSmokeSummary,
    pub passed: bool,
}

impl NativeTacticLaunchSmokeBundle {
    pub fn build(
        bundle_root: &Path,
        repository_root: &Path,
        request_path: &Path,
        execution_path: &Path,
        route_report_path: &Path,
    ) -> Result<Self, NativeTacticRouteRunError> {
        if bundle_root.exists() {
            return Err(route_message(
                "native tactic launch smoke bundle output already exists",
            ));
        }
        let repository_root = repository_root.canonicalize().map_err(route_error)?;
        let request_path = confined_source(&repository_root, request_path)?;
        let execution_path = confined_source(&repository_root, execution_path)?;
        let route_report_path = confined_source(&repository_root, route_report_path)?;
        let request: OptimizationRequest =
            serde_json::from_slice(&fs::read(&request_path).map_err(route_error)?)
                .map_err(route_error)?;
        let execution: NativeResidualExecutionBinding =
            serde_json::from_slice(&fs::read(&execution_path).map_err(route_error)?)
                .map_err(route_error)?;
        request.validate().map_err(route_error)?;
        execution.validate_seal(&request).map_err(route_error)?;
        let route: NativeTacticRouteReport =
            serde_json::from_slice(&fs::read(&route_report_path).map_err(route_error)?)
                .map_err(route_error)?;
        let plan_path = confined_source(&repository_root, Path::new(&route.execution_plan_path))?;
        let plan = NativeTacticExecutionPlan::read(&plan_path)?;
        let campaign_audit = NativeTacticScratchCampaignAudit::build(&repository_root, &route)?;
        let paths = SmokeSourcePaths::resolve(&repository_root, &route_report_path, &route)?;
        let hello: HelloResponse =
            serde_json::from_slice(&fs::read(&paths.worker_hello).map_err(route_error)?)
                .map_err(route_error)?;
        let initial_request: NativeSuffixBatch =
            serde_json::from_slice(&fs::read(&paths.initial_request).map_err(route_error)?)
                .map_err(route_error)?;
        let initial_result: NativeSuffixBatchResult =
            serde_json::from_slice(&fs::read(&paths.initial_result).map_err(route_error)?)
                .map_err(route_error)?;
        let proposal_result: NativeSuffixBatchResult =
            serde_json::from_slice(&fs::read(&paths.proposal_result).map_err(route_error)?)
                .map_err(route_error)?;
        let checkpoint =
            TacticQCampaign::read_checkpoint_payload(&paths.checkpoint).map_err(route_error)?;
        validate_checkpoint(&checkpoint).map_err(route_error)?;
        let summary = validate_smoke_sources(
            &request,
            &execution,
            &plan,
            &route,
            &campaign_audit,
            &hello,
            &initial_request,
            &initial_result,
            &proposal_result,
            &checkpoint,
            &fs::read(&paths.lease_journal).map_err(route_error)?,
        )?;

        let store = ContentStore::initialize(bundle_root).map_err(route_error)?;
        let mut guard = BundleBuildGuard::new(bundle_root);
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
        let route_report = bundle_file(
            &store,
            &route_report_path,
            ContentKind::DatasetManifest,
            route_report_sha256(&route)?,
        )?;
        let campaign_audit = bundle_bytes(
            &store,
            &campaign_audit.to_pretty_json()?,
            ContentKind::DatasetManifest,
            campaign_audit.content_sha256,
        )?;
        let worker_hello = bundle_file(
            &store,
            &paths.worker_hello,
            ContentKind::DatasetManifest,
            file_sha256(&paths.worker_hello)?,
        )?;
        let initial_request = bundle_file(
            &store,
            &paths.initial_request,
            ContentKind::DatasetManifest,
            file_sha256(&paths.initial_request)?,
        )?;
        let initial_result = bundle_file(
            &store,
            &paths.initial_result,
            ContentKind::DatasetManifest,
            file_sha256(&paths.initial_result)?,
        )?;
        let compact_proposal_request = bundle_file(
            &store,
            &paths.compact_proposal_request,
            ContentKind::CrashArtifact,
            file_sha256(&paths.compact_proposal_request)?,
        )?;
        let proposal_result = bundle_file(
            &store,
            &paths.proposal_result,
            ContentKind::DatasetManifest,
            file_sha256(&paths.proposal_result)?,
        )?;
        let lease_journal = bundle_file(
            &store,
            &paths.lease_journal,
            ContentKind::CrashArtifact,
            route.seeds[0]
                .graph_metrics
                .as_ref()
                .expect("validated smoke graph metrics")
                .lease_accounting
                .journal_sha256,
        )?;
        let checkpoint_snapshot_bytes = serde_cbor::to_vec(&checkpoint).map_err(route_error)?;
        if checkpoint_snapshot_bytes.len() > MAXIMUM_SMOKE_CHECKPOINT_SNAPSHOT_BYTES {
            return Err(route_message(
                "native tactic launch smoke checkpoint snapshot is too large",
            ));
        }
        let compressed_checkpoint = zstd::bulk::compress(
            &checkpoint_snapshot_bytes,
            SMOKE_CHECKPOINT_COMPRESSION_LEVEL,
        )
        .map_err(route_error)?;
        let checkpoint_snapshot_uncompressed_bytes =
            u64::try_from(checkpoint_snapshot_bytes.len()).map_err(route_error)?;
        let checkpoint_snapshot = bundle_bytes(
            &store,
            &compressed_checkpoint,
            ContentKind::CrashArtifact,
            checkpoint.content_sha256,
        )?;
        let mut authorities = smoke_authorities(&store, &repository_root, &request, &execution)?;
        authorities.sort_by(|left, right| left.role.cmp(&right.role));
        let mut bundle = Self {
            schema: NATIVE_TACTIC_LAUNCH_SMOKE_BUNDLE_SCHEMA_V1.into(),
            content_sha256: Digest::ZERO,
            optimization_request,
            execution_binding,
            execution_plan,
            route_report,
            campaign_audit,
            worker_hello,
            initial_request,
            initial_result,
            compact_proposal_request,
            proposal_result,
            lease_journal,
            checkpoint_snapshot,
            checkpoint_snapshot_uncompressed_bytes,
            execution_identity: execution_identity(&execution),
            authorities,
            summary,
            passed: true,
        };
        bundle.content_sha256 = bundle.compute_content_sha256()?;
        bundle.validate(bundle_root)?;
        let mut bytes = serde_json::to_vec_pretty(&bundle).map_err(route_error)?;
        bytes.push(b'\n');
        fs::write(bundle_root.join(NATIVE_TACTIC_LAUNCH_SMOKE_MANIFEST), bytes)
            .map_err(route_error)?;
        guard.commit();
        Ok(bundle)
    }

    pub fn read_and_validate(bundle_root: &Path) -> Result<Self, NativeTacticRouteRunError> {
        let bundle: Self = serde_json::from_slice(
            &fs::read(bundle_root.join(NATIVE_TACTIC_LAUNCH_SMOKE_MANIFEST))
                .map_err(route_error)?,
        )
        .map_err(route_error)?;
        bundle.validate(bundle_root)?;
        Ok(bundle)
    }

    pub fn validate(&self, bundle_root: &Path) -> Result<(), NativeTacticRouteRunError> {
        if self.schema != NATIVE_TACTIC_LAUNCH_SMOKE_BUNDLE_SCHEMA_V1
            || self.content_sha256 == Digest::ZERO
            || self.compute_content_sha256()? != self.content_sha256
            || !self.passed
            || self.authorities.len() != 6
            || !self
                .authorities
                .windows(2)
                .all(|pair| pair[0].role < pair[1].role)
        {
            return Err(route_message(
                "native tactic launch smoke bundle manifest is invalid",
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
                    "native tactic launch smoke authority digest differs",
                ));
            }
        }
        let request: OptimizationRequest = read_json(&store, &self.optimization_request)?;
        let execution: NativeResidualExecutionBinding = read_json(&store, &self.execution_binding)?;
        let plan =
            NativeTacticExecutionPlan::read(&store.blob_path(self.execution_plan.blob.sha256))?;
        let route: NativeTacticRouteReport = read_json(&store, &self.route_report)?;
        let audit: NativeTacticScratchCampaignAudit = read_json(&store, &self.campaign_audit)?;
        let hello: HelloResponse = read_json(&store, &self.worker_hello)?;
        let initial_request: NativeSuffixBatch = read_json(&store, &self.initial_request)?;
        let initial_result: NativeSuffixBatchResult = read_json(&store, &self.initial_result)?;
        let proposal_result: NativeSuffixBatchResult = read_json(&store, &self.proposal_result)?;
        let snapshot_len =
            usize::try_from(self.checkpoint_snapshot_uncompressed_bytes).map_err(route_error)?;
        if snapshot_len == 0 || snapshot_len > MAXIMUM_SMOKE_CHECKPOINT_SNAPSHOT_BYTES {
            return Err(route_message(
                "native tactic launch smoke checkpoint size is invalid",
            ));
        }
        let compressed_checkpoint = store
            .read_bytes(&self.checkpoint_snapshot.blob)
            .map_err(route_error)?;
        let checkpoint_bytes =
            zstd::bulk::decompress(&compressed_checkpoint, snapshot_len).map_err(route_error)?;
        if checkpoint_bytes.len() != snapshot_len {
            return Err(route_message(
                "native tactic launch smoke checkpoint size differs",
            ));
        }
        let checkpoint: TacticQCampaignCheckpoint =
            serde_cbor::from_slice(&checkpoint_bytes).map_err(route_error)?;
        validate_checkpoint_snapshot(&checkpoint).map_err(route_error)?;
        let lease_bytes = store
            .read_bytes(&self.lease_journal.blob)
            .map_err(route_error)?;
        let compact_request_bytes = store
            .read_bytes(&self.compact_proposal_request.blob)
            .map_err(route_error)?;
        request.validate().map_err(route_error)?;
        execution.validate_seal(&request).map_err(route_error)?;
        audit.validate()?;
        audit.validate_resource_binding(&route, &plan)?;
        let summary = validate_smoke_sources(
            &request,
            &execution,
            &plan,
            &route,
            &audit,
            &hello,
            &initial_request,
            &initial_result,
            &proposal_result,
            &checkpoint,
            &lease_bytes,
        )?;
        if self.optimization_request.logical_identity_sha256 != request.content_sha256
            || self.execution_binding.logical_identity_sha256 != execution.content_sha256
            || self.execution_plan.logical_identity_sha256 != plan.identity()?
            || self.route_report.logical_identity_sha256 != route_report_sha256(&route)?
            || self.campaign_audit.logical_identity_sha256 != audit.content_sha256
            || self.worker_hello.logical_identity_sha256 != self.worker_hello.blob.sha256
            || self.initial_request.logical_identity_sha256 != self.initial_request.blob.sha256
            || self.initial_result.logical_identity_sha256 != self.initial_result.blob.sha256
            || self.compact_proposal_request.logical_identity_sha256
                != self.compact_proposal_request.blob.sha256
            || !compact_request_bytes.starts_with(&COMPACT_SUFFIX_BATCH_MAGIC)
            || self.proposal_result.logical_identity_sha256 != self.proposal_result.blob.sha256
            || self.lease_journal.logical_identity_sha256
                != route.seeds[0]
                    .graph_metrics
                    .as_ref()
                    .expect("validated smoke graph metrics")
                    .lease_accounting
                    .journal_sha256
            || self.checkpoint_snapshot.logical_identity_sha256 != checkpoint.content_sha256
            || self.execution_identity != execution_identity(&execution)
            || self.summary != summary
            || !authorities_match(self, &request, &execution)
        {
            return Err(route_message(
                "native tactic launch smoke bundle authorities are detached",
            ));
        }
        Ok(())
    }

    fn artifacts(&self) -> [&NativeTacticScratchBundleArtifact; 12] {
        [
            &self.optimization_request,
            &self.execution_binding,
            &self.execution_plan,
            &self.route_report,
            &self.campaign_audit,
            &self.worker_hello,
            &self.initial_request,
            &self.initial_result,
            &self.compact_proposal_request,
            &self.proposal_result,
            &self.lease_journal,
            &self.checkpoint_snapshot,
        ]
    }

    fn compute_content_sha256(&self) -> Result<Digest, NativeTacticRouteRunError> {
        let mut unsigned = self.clone();
        unsigned.content_sha256 = Digest::ZERO;
        Ok(Digest(
            Sha256::digest(serde_cbor::to_vec(&unsigned).map_err(route_error)?).into(),
        ))
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_smoke_sources(
    request: &OptimizationRequest,
    execution: &NativeResidualExecutionBinding,
    plan: &NativeTacticExecutionPlan,
    route: &NativeTacticRouteReport,
    audit: &NativeTacticScratchCampaignAudit,
    hello: &HelloResponse,
    initial_request: &NativeSuffixBatch,
    initial_result: &NativeSuffixBatchResult,
    proposal_result: &NativeSuffixBatchResult,
    checkpoint: &TacticQCampaignCheckpoint,
    lease_bytes: &[u8],
) -> Result<NativeTacticLaunchSmokeSummary, NativeTacticRouteRunError> {
    hello.validate().map_err(route_error)?;
    let terminal = NativeTerminalBinding {
        goal: request.terminal_predicate.goal.clone(),
        program_sha256: request.terminal_predicate.program_sha256,
        definition_sha256: request.terminal_predicate.definition_sha256,
    };
    let initial = initial_result
        .validate_against(initial_request, &terminal)
        .map_err(route_error)?;
    let seed = route
        .seeds
        .first()
        .ok_or_else(|| route_message("native tactic launch smoke has no seed"))?;
    let trace = seed
        .trace
        .first()
        .ok_or_else(|| route_message("native tactic launch smoke has no decision"))?;
    let proposal = trace
        .proposal_batch
        .first()
        .ok_or_else(|| route_message("native tactic launch smoke has no proposal"))?;
    let candidate = proposal_result
        .candidates
        .first()
        .ok_or_else(|| route_message("native tactic launch smoke result has no candidate"))?;
    let cache = proposal_result
        .checkpoint_cache
        .as_ref()
        .ok_or_else(|| route_message("native tactic launch smoke lacks cache telemetry"))?;
    let graph_metrics = seed
        .graph_metrics
        .as_ref()
        .ok_or_else(|| route_message("native tactic launch smoke lacks graph metrics"))?;
    let lease_accounting = NativeTacticLeaseLedger::accounting_from_bytes(lease_bytes)?;
    let memory_bound = match plan.budgets.memory_bytes {
        NativeTacticResourceLimit::Bounded(value) => value,
        NativeTacticResourceLimit::Unbounded => {
            return Err(route_message(
                "native tactic launch smoke requires bounded memory",
            ));
        }
    };
    if !launch_smoke_route_schema_is_supported(&route.schema)
        || route.optimization_request_sha256 != request.content_sha256
        || route.execution_binding_sha256 != execution.content_sha256
        || route.execution_plan_sha256 != plan.identity()?
        || route.objective_sha256 != request.terminal_predicate.definition_sha256
        || route.execution_strategy != NativeGenericExecutionStrategy::NativeController
        || route.workers != 1
        || route.exploration_seeds.len() != 1
        || route.seeds.len() != 1
        || route.total_decisions != 1
        || route.useful_decisions != 1
        || route.unique_useful_graph_expansions != 1
        || route.total_native_ticks == 0
        || route.demonstration_transitions != 0
        || plan.seeds.len() != 1
        || plan.lanes.len() != 1
        || plan.proposal_width_per_decision != 1
        || plan.budgets.decisions_per_lane != 1
        || !matches!(
            plan.budgets.wall_micros,
            NativeTacticResourceLimit::Bounded(_)
        )
        || memory_bound != route.checkpoint_cache_capacity_per_worker_bytes
        || trace.result_admission_schema != NATIVE_TACTIC_RESULT_ADMISSION_SCHEMA_V1
        || trace.proposal_batch.len() != 1
        || proposal.realized_ticks == 0
        || proposal.realized_ticks as u64 != candidate.ticks_executed
        || proposal.terminal != candidate.success
        || proposal_result.schema != NATIVE_SUFFIX_BATCH_RESULT_SCHEMA_V9
        || proposal_result.status != "passed"
        || proposal_result.error.is_some()
        || proposal_result.source_frame != request.route.source_boundary_index
        || proposal_result.candidate_count != 1
        || proposal_result.completed_candidates != 1
        || proposal_result.candidates.len() != 1
        || proposal_result.timing.schema != "dusklight-suffix-batch-timing/v1"
        || !proposal_result.timing.verified
        || proposal_result.timing.candidate_ticks != candidate.ticks_executed
        || !proposal_result.audio_callback_quiesced
        || proposal_result.restore_identity.as_deref() != Some(initial.restore_identity.as_str())
        || cache.capacity_bytes != memory_bound
        || cache.capacity_entries == 0
        || cache.live_endpoint_capacity_entries == 0
        || checkpoint
            .state_graph
            .content_sha256()
            .map_err(route_error)?
            != seed.state_graph_sha256
        || graph_metrics.lease_accounting != lease_accounting
        || lease_accounting.proposal_dispatches != 1
        || lease_accounting.completed_leases != 1
        || lease_accounting.retryable_leases != 0
        || lease_accounting.cancelled_leases != 0
        || lease_accounting.failed_leases != 0
        || lease_accounting.unresolved_leases != 0
        || !hello.capabilities.persistent_control
        || !hello.capabilities.batch_run
        || !hello.capabilities.compact_batch_run
        || hello.build.dirty
        || !audit.resources.passed
        || audit.resources.completed_decisions != 1
    {
        return Err(route_message(
            "native tactic launch smoke contract did not pass",
        ));
    }
    Ok(NativeTacticLaunchSmokeSummary {
        platform: hello.build.platform.clone(),
        architecture: hello.build.architecture.clone(),
        worker_revision: hello.build.revision.clone(),
        worker_aurora_revision: hello.build.aurora_revision.clone(),
        clean_build: true,
        executable_sha256: execution.executable.sha256,
        game_data_sha256: execution.game_data.sha256,
        root_checkpoint_identity: initial.restore_identity,
        source_boundary_index: request.route.source_boundary_index,
        proposal_ticks: candidate.ticks_executed,
        checkpoint_cache_capacity_bytes: cache.capacity_bytes,
        completed_graph_leases: lease_accounting.completed_leases,
        unresolved_graph_leases: lease_accounting.unresolved_leases,
        compact_persistent_control: true,
        resource_audit_passed: true,
    })
}

fn launch_smoke_route_schema_is_supported(schema: &str) -> bool {
    matches!(
        schema,
        NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V38
            | NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V39
            | NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V40
            | NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V41
            | NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V42
            | NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V43
            | NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V44
            | NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V45
    )
}

struct SmokeSourcePaths {
    worker_hello: PathBuf,
    initial_request: PathBuf,
    initial_result: PathBuf,
    compact_proposal_request: PathBuf,
    proposal_result: PathBuf,
    lease_journal: PathBuf,
    checkpoint: PathBuf,
}

impl SmokeSourcePaths {
    fn resolve(
        repository_root: &Path,
        report_path: &Path,
        route: &NativeTacticRouteReport,
    ) -> Result<Self, NativeTacticRouteRunError> {
        let campaign_root = report_path
            .parent()
            .ok_or_else(|| route_message("native tactic smoke report has no campaign root"))?;
        let attempts_root = campaign_root.join("attempts");
        let mut attempts = fs::read_dir(&attempts_root)
            .map_err(route_error)?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(route_error)?;
        attempts.retain(|path| path.is_dir());
        attempts.sort();
        if attempts.len() != 1 {
            return Err(route_message(
                "native tactic launch smoke must use exactly one worker attempt",
            ));
        }
        let checkpoint = confined_source(
            repository_root,
            Path::new(
                &route
                    .seeds
                    .first()
                    .ok_or_else(|| route_message("native tactic smoke has no seed"))?
                    .final_checkpoint,
            ),
        )?;
        let seed_root = checkpoint
            .parent()
            .and_then(Path::parent)
            .ok_or_else(|| route_message("native tactic smoke checkpoint has no seed root"))?;
        Ok(Self {
            worker_hello: confined_source(
                repository_root,
                &attempts[0].join(NATIVE_TACTIC_WORKER_HELLO_FILE),
            )?,
            initial_request: confined_source(
                repository_root,
                &attempts[0].join("initial/request.json"),
            )?,
            initial_result: confined_source(
                repository_root,
                &attempts[0].join("initial/result.json"),
            )?,
            compact_proposal_request: confined_source(
                repository_root,
                &seed_root.join("native/decision-000000/proposal-000/request.dsbx"),
            )?,
            proposal_result: confined_source(
                repository_root,
                &seed_root.join("native/decision-000000/proposal-000/result.json"),
            )?,
            lease_journal: confined_source(
                repository_root,
                &seed_root.join(NATIVE_TACTIC_LEASE_JOURNAL_FILE),
            )?,
            checkpoint,
        })
    }
}

fn smoke_authorities(
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
    bundle: &NativeTacticLaunchSmokeBundle,
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
            "native tactic launch smoke authority bytes differ",
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

fn file_sha256(path: &Path) -> Result<Digest, NativeTacticRouteRunError> {
    Ok(Digest(
        Sha256::digest(fs::read(path).map_err(route_error)?).into(),
    ))
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
            "native tactic launch smoke source is outside the repository or not a file",
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
    fn smoke_schema_is_distinct_from_terminal_acceptance() {
        assert_ne!(
            NATIVE_TACTIC_LAUNCH_SMOKE_BUNDLE_SCHEMA_V1,
            NATIVE_TACTIC_SCRATCH_EVIDENCE_BUNDLE_SCHEMA_V2
        );
    }

    #[test]
    fn retained_and_current_route_schemas_are_supported() {
        assert!(launch_smoke_route_schema_is_supported(
            NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V38
        ));
        assert!(launch_smoke_route_schema_is_supported(
            NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V39
        ));
        assert!(launch_smoke_route_schema_is_supported(
            NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V40
        ));
        assert!(launch_smoke_route_schema_is_supported(
            NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V41
        ));
        assert!(launch_smoke_route_schema_is_supported(
            NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V42
        ));
        assert!(launch_smoke_route_schema_is_supported(
            NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V43
        ));
        assert!(launch_smoke_route_schema_is_supported(
            NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V44
        ));
        assert!(launch_smoke_route_schema_is_supported(
            NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V45
        ));
        assert!(!launch_smoke_route_schema_is_supported(
            NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V37
        ));
    }
}
