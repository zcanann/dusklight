use super::candidate_retention::route_frames_first_hit_tick;
use super::scratch_discovery::route_report_sha256;
use super::*;
use crate::native_residual_campaign::NativeResidualExecutionBinding;
use crate::tactic_q_campaign::{
    TacticQCampaign, TacticQCampaignCheckpoint, TacticQFinalResult, route_checkpoint,
    validate_checkpoint,
};
use dusklight_evidence::content_store::{ContentBlob, ContentKind, ContentStore};
use dusklight_harness_contracts::objective_suite::ArtifactReference;

pub const NATIVE_TACTIC_SCRATCH_EVIDENCE_BUNDLE_SCHEMA_V2: &str =
    "dusklight-native-tactic-scratch-evidence-bundle/v2";
pub const NATIVE_TACTIC_SCRATCH_EVIDENCE_MANIFEST: &str = "manifest.json";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticScratchBundleArtifact {
    pub logical_identity_sha256: Digest,
    pub blob: ContentBlob,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticScratchAuthorityArtifact {
    pub role: String,
    pub declared_path: String,
    pub declared_sha256: Digest,
    pub blob: ContentBlob,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticScratchExecutionIdentity {
    pub execution_binding_sha256: Digest,
    pub executable_sha256: Digest,
    pub runtime_dependency_sha256s: Vec<Digest>,
    pub game_data_sha256: Digest,
    pub process_boot_tape_sha256: Digest,
    pub milestone_program_sha256: Digest,
    pub world_context_sha256: Digest,
    pub card_fixture_manifest_sha256: Digest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticScratchSeedEvidence {
    pub seed: u64,
    pub terminal_discovered: bool,
    pub best_authenticated_tick: Option<u64>,
    pub first_terminal_decision_index: Option<u64>,
    pub time_to_first_terminal_micros: Option<u64>,
    pub state_graph_sha256: Digest,
    pub best_terminal_state_sha256: Option<Digest>,
    pub best_terminal_route_checkpoint_sha256: Option<Digest>,
    pub seed_result: NativeTacticScratchBundleArtifact,
    pub lease_journal: NativeTacticScratchBundleArtifact,
    pub checkpoint_envelope: NativeTacticScratchBundleArtifact,
    pub checkpoint_snapshot: NativeTacticScratchBundleArtifact,
    pub best_terminal_tape: Option<NativeTacticScratchBundleArtifact>,
    pub best_terminal_result: Option<NativeTacticScratchBundleArtifact>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticScratchEvidenceBundle {
    pub schema: String,
    pub content_sha256: Digest,
    pub acceptance: NativeTacticScratchBundleArtifact,
    pub optimization_request: NativeTacticScratchBundleArtifact,
    pub execution_binding: NativeTacticScratchBundleArtifact,
    pub execution_plan: NativeTacticScratchBundleArtifact,
    pub route_report: NativeTacticScratchBundleArtifact,
    pub campaign_audit: NativeTacticScratchBundleArtifact,
    pub execution_identity: NativeTacticScratchExecutionIdentity,
    pub authorities: Vec<NativeTacticScratchAuthorityArtifact>,
    pub seeds: Vec<NativeTacticScratchSeedEvidence>,
    pub passed: bool,
}

impl NativeTacticScratchEvidenceBundle {
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
        acceptance: &NativeTacticScratchDiscoveryReport,
    ) -> Result<Self, NativeTacticRouteRunError> {
        if bundle_root.exists() {
            return Err(route_message(
                "scratch evidence bundle output already exists",
            ));
        }
        request.validate().map_err(route_error)?;
        execution.validate_seal(request).map_err(route_error)?;
        acceptance.validate()?;
        let route_sha256 = route_report_sha256(route)?;
        if execution.content_sha256 != route.execution_binding_sha256
            || route.optimization_request_sha256 != request.content_sha256
            || acceptance.optimization_request_sha256 != request.content_sha256
            || acceptance.route_report_sha256 != route_sha256
        {
            return Err(route_message(
                "scratch evidence inputs do not share one execution authority",
            ));
        }

        let repository_root = repository_root.canonicalize().map_err(route_error)?;
        let request_path = confined_source(&repository_root, request_path)?;
        let execution_path = confined_source(&repository_root, execution_path)?;
        let route_report_path = confined_source(&repository_root, route_report_path)?;
        let plan_path = confined_source(&repository_root, Path::new(&route.execution_plan_path))?;
        let plan = NativeTacticExecutionPlan::read(&plan_path)?;
        let plan_sha256 = plan.identity()?;
        if plan_sha256 != route.execution_plan_sha256
            || plan_sha256 != acceptance.execution_plan_sha256
        {
            return Err(route_message(
                "scratch evidence execution plan identity differs",
            ));
        }

        let store = ContentStore::initialize(bundle_root).map_err(route_error)?;
        let mut build_guard = BundleBuildGuard::new(bundle_root);
        let passed = acceptance.passed;
        let acceptance = bundle_bytes(
            &store,
            &acceptance.to_pretty_json()?,
            ContentKind::DatasetManifest,
            acceptance.content_sha256,
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
        let campaign_audit = bundle_bytes(
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

        let mut seeds = Vec::with_capacity(route.seeds.len());
        for seed in &route.seeds {
            seeds.push(bundle_seed(
                &store,
                &repository_root,
                request.route.source_boundary_index,
                route,
                seed,
            )?);
        }
        seeds.sort_by_key(|seed| seed.seed);

        let mut bundle = Self {
            schema: NATIVE_TACTIC_SCRATCH_EVIDENCE_BUNDLE_SCHEMA_V2.into(),
            content_sha256: Digest::ZERO,
            acceptance,
            optimization_request,
            execution_binding,
            execution_plan,
            route_report,
            campaign_audit,
            execution_identity: NativeTacticScratchExecutionIdentity {
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
            },
            authorities,
            seeds,
            passed,
        };
        bundle.content_sha256 = bundle.compute_content_sha256()?;
        bundle.validate(bundle_root)?;
        let manifest_path = bundle_root.join(NATIVE_TACTIC_SCRATCH_EVIDENCE_MANIFEST);
        let mut bytes = serde_json::to_vec_pretty(&bundle).map_err(route_error)?;
        bytes.push(b'\n');
        fs::write(manifest_path, bytes).map_err(route_error)?;
        build_guard.commit();
        Ok(bundle)
    }

    pub fn read_and_validate(bundle_root: &Path) -> Result<Self, NativeTacticRouteRunError> {
        let manifest_path = bundle_root.join(NATIVE_TACTIC_SCRATCH_EVIDENCE_MANIFEST);
        let bundle: Self = serde_json::from_slice(&fs::read(manifest_path).map_err(route_error)?)
            .map_err(route_error)?;
        bundle.validate(bundle_root)?;
        Ok(bundle)
    }

    pub fn validate(&self, bundle_root: &Path) -> Result<(), NativeTacticRouteRunError> {
        if self.schema != NATIVE_TACTIC_SCRATCH_EVIDENCE_BUNDLE_SCHEMA_V2
            || self.content_sha256 == Digest::ZERO
            || self.compute_content_sha256()? != self.content_sha256
            || self.passed
                != read_json_blob::<NativeTacticScratchDiscoveryReport>(
                    bundle_root,
                    &self.acceptance,
                )?
                .passed
            || self.authorities.is_empty()
            || self.seeds.is_empty()
            || !self
                .authorities
                .windows(2)
                .all(|pair| pair[0].role < pair[1].role)
            || !self
                .seeds
                .windows(2)
                .all(|pair| pair[0].seed < pair[1].seed)
        {
            return Err(route_message("scratch evidence bundle manifest is invalid"));
        }

        let store = ContentStore::open(bundle_root).map_err(route_error)?;
        for artifact in self.artifacts() {
            store.verify(&artifact.blob).map_err(route_error)?;
        }
        for authority in &self.authorities {
            store.verify(&authority.blob).map_err(route_error)?;
            if authority.declared_sha256 != authority.blob.sha256 {
                return Err(route_message(
                    "scratch evidence authority digest differs from its blob",
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
        let campaign_audit: NativeTacticScratchCampaignAudit =
            read_json_blob(bundle_root, &self.campaign_audit)?;
        let acceptance: NativeTacticScratchDiscoveryReport =
            read_json_blob(bundle_root, &self.acceptance)?;
        campaign_audit.validate()?;
        campaign_audit.validate_resource_binding(&route, &plan)?;
        acceptance.validate()?;

        if self.optimization_request.logical_identity_sha256 != request.content_sha256
            || self.execution_binding.logical_identity_sha256 != execution.content_sha256
            || self.execution_plan.logical_identity_sha256 != plan.identity()?
            || self.route_report.logical_identity_sha256 != route_report_sha256(&route)?
            || self.campaign_audit.logical_identity_sha256 != campaign_audit.content_sha256
            || self.acceptance.logical_identity_sha256 != acceptance.content_sha256
            || route.optimization_request_sha256 != request.content_sha256
            || route.execution_binding_sha256 != execution.content_sha256
            || route.execution_plan_sha256 != plan.identity()?
            || acceptance.optimization_request_sha256 != request.content_sha256
            || acceptance.execution_plan_sha256 != plan.identity()?
            || acceptance.route_report_sha256 != route_report_sha256(&route)?
            || campaign_audit.route_report_sha256 != route_report_sha256(&route)?
            || campaign_audit.execution_plan_sha256 != route.execution_plan_sha256
            || campaign_audit.objective_sha256 != route.objective_sha256
            || campaign_audit.execution_binding_sha256 != route.execution_binding_sha256
            || campaign_audit.seeds.len() != route.seeds.len()
            || self.execution_identity != execution_identity(&execution)
            || route.seeds.len() != self.seeds.len()
        {
            return Err(route_message(
                "scratch evidence bundle authorities are detached",
            ));
        }
        validate_authorities(self, &request, &execution)?;
        for bundled in &self.seeds {
            let reported = route
                .seeds
                .iter()
                .find(|reported| reported.seed == bundled.seed)
                .ok_or_else(|| route_message("scratch bundled seed is absent from route report"))?;
            let audited = campaign_audit
                .seeds
                .iter()
                .find(|audited| audited.seed == bundled.seed)
                .ok_or_else(|| {
                    route_message("scratch bundled seed is absent from campaign audit")
                })?;
            validate_seed(
                bundle_root,
                request.route.source_boundary_index,
                &route,
                reported,
                bundled,
                audited,
            )?;
        }
        Ok(())
    }

    fn artifacts(&self) -> Vec<&NativeTacticScratchBundleArtifact> {
        let mut artifacts = vec![
            &self.acceptance,
            &self.optimization_request,
            &self.execution_binding,
            &self.execution_plan,
            &self.route_report,
            &self.campaign_audit,
        ];
        for seed in &self.seeds {
            artifacts.extend([
                &seed.seed_result,
                &seed.lease_journal,
                &seed.checkpoint_envelope,
                &seed.checkpoint_snapshot,
            ]);
            if let Some(artifact) = &seed.best_terminal_tape {
                artifacts.push(artifact);
            }
            if let Some(artifact) = &seed.best_terminal_result {
                artifacts.push(artifact);
            }
        }
        artifacts
    }

    fn compute_content_sha256(&self) -> Result<Digest, NativeTacticRouteRunError> {
        let mut unsigned = self.clone();
        unsigned.content_sha256 = Digest::ZERO;
        Ok(Digest(
            Sha256::digest(serde_json::to_vec(&unsigned).map_err(route_error)?).into(),
        ))
    }
}

fn bundle_seed(
    store: &ContentStore,
    repository_root: &Path,
    source_boundary_index: u64,
    route: &NativeTacticRouteReport,
    seed: &NativeTacticSeedResult,
) -> Result<NativeTacticScratchSeedEvidence, NativeTacticRouteRunError> {
    let checkpoint_path = confined_source(repository_root, Path::new(&seed.final_checkpoint))?;
    let seed_root = checkpoint_path
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| route_message("scratch seed checkpoint has no seed root"))?;
    let seed_result_path = confined_source(repository_root, &seed_root.join("seed-result.json"))?;
    let seed_result_identity = canonical_json_sha256(seed)?;
    let seed_result = bundle_file(
        store,
        &seed_result_path,
        ContentKind::DatasetManifest,
        seed_result_identity,
    )?;
    let graph_metrics = seed
        .graph_metrics
        .as_ref()
        .ok_or_else(|| route_message("scratch seed has no graph metrics"))?;
    let lease_journal_path = confined_source(
        repository_root,
        &seed_root.join(NATIVE_TACTIC_LEASE_JOURNAL_FILE),
    )?;
    let lease_journal = bundle_file(
        store,
        &lease_journal_path,
        ContentKind::CrashArtifact,
        graph_metrics.lease_accounting.journal_sha256,
    )?;
    if lease_journal.blob.sha256 != graph_metrics.lease_accounting.journal_sha256 {
        return Err(route_message(
            "scratch seed lease journal differs from reported accounting",
        ));
    }
    let checkpoint =
        TacticQCampaign::read_checkpoint_payload(&checkpoint_path).map_err(route_error)?;
    validate_checkpoint(&checkpoint).map_err(route_error)?;
    let graph_sha256 = checkpoint
        .state_graph
        .content_sha256()
        .map_err(route_error)?;
    if graph_sha256 != seed.state_graph_sha256 {
        return Err(route_message(
            "scratch seed checkpoint graph identity differs",
        ));
    }
    let checkpoint_envelope = bundle_file(
        store,
        &checkpoint_path,
        ContentKind::CrashArtifact,
        checkpoint.content_sha256,
    )?;
    let checkpoint_snapshot_bytes = serde_cbor::to_vec(&checkpoint).map_err(route_error)?;
    let checkpoint_snapshot = bundle_bytes(
        store,
        &checkpoint_snapshot_bytes,
        ContentKind::StateGraph,
        checkpoint.content_sha256,
    )?;

    let (best_terminal_tape, best_terminal_result) =
        match (&seed.best_terminal_tape, &seed.best_terminal_result) {
            (Some(tape), Some(result)) => {
                let tape_path = confined_source(repository_root, Path::new(tape))?;
                let result_path = confined_source(repository_root, Path::new(result))?;
                let tape_bytes = fs::read(&tape_path).map_err(route_error)?;
                let tape = InputTape::decode(&tape_bytes).map_err(route_error)?.tape;
                let result = TacticQFinalResult::read(&result_path).map_err(route_error)?;
                if result.route_tape != tape
                    || result.objective_sha256 != route.objective_sha256
                    || result.execution_authority_sha256 != route.execution_plan_sha256
                    || Some(result.terminal_state_sha256) != seed.best_terminal_state_sha256
                    || Some(
                        route_checkpoint(result.root_checkpoint_sha256, &tape)
                            .map_err(route_error)?,
                    ) != seed.best_terminal_route_checkpoint_sha256
                    || terminal_tape_first_hit_tick(&tape, source_boundary_index)
                        != seed.best_authenticated_tick
                {
                    return Err(route_message(
                        "scratch seed best terminal artifacts are detached",
                    ));
                }
                (
                    Some(bundle_bytes(
                        store,
                        &tape_bytes,
                        ContentKind::InputTape,
                        result.route_tape_sha256,
                    )?),
                    Some(bundle_file(
                        store,
                        &result_path,
                        ContentKind::CrashArtifact,
                        result.content_sha256,
                    )?),
                )
            }
            (None, None) if !seed.terminal_discovered => (None, None),
            _ => {
                return Err(route_message(
                    "scratch terminal seed lacks a complete best-artifact pair",
                ));
            }
        };

    Ok(NativeTacticScratchSeedEvidence {
        seed: seed.seed,
        terminal_discovered: seed.terminal_discovered,
        best_authenticated_tick: seed.best_authenticated_tick,
        first_terminal_decision_index: seed.first_terminal_decision_index,
        time_to_first_terminal_micros: seed.time_to_first_terminal_micros,
        state_graph_sha256: seed.state_graph_sha256,
        best_terminal_state_sha256: seed.best_terminal_state_sha256,
        best_terminal_route_checkpoint_sha256: seed.best_terminal_route_checkpoint_sha256,
        seed_result,
        lease_journal,
        checkpoint_envelope,
        checkpoint_snapshot,
        best_terminal_tape,
        best_terminal_result,
    })
}

fn validate_seed(
    bundle_root: &Path,
    source_boundary_index: u64,
    route: &NativeTacticRouteReport,
    reported: &NativeTacticSeedResult,
    bundled: &NativeTacticScratchSeedEvidence,
    audited: &NativeTacticScratchSeedAudit,
) -> Result<(), NativeTacticRouteRunError> {
    let stored: NativeTacticSeedResult = read_json_blob(bundle_root, &bundled.seed_result)?;
    let snapshot_bytes = read_blob(bundle_root, &bundled.checkpoint_snapshot)?;
    let lease_journal_bytes = read_blob(bundle_root, &bundled.lease_journal)?;
    let bundled_lease_accounting =
        NativeTacticLeaseLedger::accounting_from_bytes(&lease_journal_bytes)?;
    let checkpoint: TacticQCampaignCheckpoint =
        serde_cbor::from_slice(&snapshot_bytes).map_err(route_error)?;
    validate_checkpoint(&checkpoint).map_err(route_error)?;
    let mut terminal_path_ticks = checkpoint
        .state_graph
        .nodes()
        .filter(|node| node.terminal && node.restoration.executable)
        .map(|node| node.root_ticks.saturating_sub(1))
        .collect::<Vec<_>>();
    terminal_path_ticks.sort_unstable();
    if canonical_json_sha256(reported)? != canonical_json_sha256(&stored)?
        || bundled.seed_result.logical_identity_sha256 != canonical_json_sha256(&stored)?
        || bundled.seed != stored.seed
        || bundled.terminal_discovered != stored.terminal_discovered
        || bundled.best_authenticated_tick != stored.best_authenticated_tick
        || bundled.first_terminal_decision_index != stored.first_terminal_decision_index
        || bundled.time_to_first_terminal_micros != stored.time_to_first_terminal_micros
        || bundled.state_graph_sha256 != stored.state_graph_sha256
        || bundled.best_terminal_state_sha256 != stored.best_terminal_state_sha256
        || bundled.best_terminal_route_checkpoint_sha256
            != stored.best_terminal_route_checkpoint_sha256
        || stored.graph_metrics.as_ref().is_none_or(|metrics| {
            bundled.lease_journal.logical_identity_sha256 != metrics.lease_accounting.journal_sha256
                || bundled_lease_accounting != metrics.lease_accounting
                || audited.proposal_dispatches != metrics.lease_accounting.proposal_dispatches
                || audited.completed_graph_leases != metrics.lease_accounting.completed_leases
                || audited.retryable_graph_leases != metrics.lease_accounting.retryable_leases
                || audited.cancelled_graph_leases != metrics.lease_accounting.cancelled_leases
                || audited.failed_graph_leases != metrics.lease_accounting.failed_leases
                || audited.unresolved_graph_leases != metrics.lease_accounting.unresolved_leases
        })
        || checkpoint.content_sha256 != bundled.checkpoint_snapshot.logical_identity_sha256
        || checkpoint.content_sha256 != bundled.checkpoint_envelope.logical_identity_sha256
        || checkpoint
            .state_graph
            .content_sha256()
            .map_err(route_error)?
            != bundled.state_graph_sha256
        || audited.terminal_discovered != stored.terminal_discovered
        || audited.best_authenticated_tick != stored.best_authenticated_tick
        || audited.first_terminal_decision_index != stored.first_terminal_decision_index
        || audited.time_to_first_terminal_micros != stored.time_to_first_terminal_micros
        || audited.total_proposal_expansions
            != stored
                .trace
                .iter()
                .map(|decision| decision.proposal_batch.len() as u64)
                .sum::<u64>()
        || audited.native_ticks != stored.native_ticks
        || audited.unique_useful_graph_expansions != stored.unique_useful_graph_expansions
        || audited.native_restore_accounting != stored.native_restore_accounting
        || audited.timing != stored.timing
        || audited.terminal_path_ticks != terminal_path_ticks
    {
        return Err(route_message(
            "scratch bundled seed differs from route or graph evidence",
        ));
    }
    match (&bundled.best_terminal_tape, &bundled.best_terminal_result) {
        (Some(tape), Some(result)) => {
            let tape_bytes = read_blob(bundle_root, tape)?;
            let tape = InputTape::decode(&tape_bytes).map_err(route_error)?.tape;
            let result_path = blob_path(bundle_root, &result.blob);
            let result = TacticQFinalResult::read(&result_path).map_err(route_error)?;
            if result.content_sha256
                != bundled
                    .best_terminal_result
                    .as_ref()
                    .expect("checked above")
                    .logical_identity_sha256
                || result.route_tape != tape
                || result.objective_sha256 != route.objective_sha256
                || result.execution_authority_sha256 != route.execution_plan_sha256
                || Some(result.terminal_state_sha256) != bundled.best_terminal_state_sha256
                || Some(
                    route_checkpoint(result.root_checkpoint_sha256, &tape).map_err(route_error)?,
                ) != bundled.best_terminal_route_checkpoint_sha256
                || terminal_tape_first_hit_tick(&tape, source_boundary_index)
                    != bundled.best_authenticated_tick
            {
                return Err(route_message(
                    "scratch bundled terminal artifacts are invalid",
                ));
            }
        }
        (None, None) if !bundled.terminal_discovered => {}
        _ => {
            return Err(route_message(
                "scratch bundled terminal artifact pair is incomplete",
            ));
        }
    }
    Ok(())
}

fn terminal_tape_first_hit_tick(tape: &InputTape, source_boundary_index: u64) -> Option<u64> {
    u64::try_from(tape.frames.len())
        .ok()
        .and_then(|frames| route_frames_first_hit_tick(frames, source_boundary_index))
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
            "scratch authority bytes differ from their declared digest",
        ));
    }
    Ok(NativeTacticScratchAuthorityArtifact {
        role: role.into(),
        declared_path: reference.path.clone(),
        declared_sha256: reference.sha256,
        blob,
    })
}

fn validate_authorities(
    bundle: &NativeTacticScratchEvidenceBundle,
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
    if bundle.authorities.len() != expected.len()
        || bundle
            .authorities
            .iter()
            .zip(expected)
            .any(|(actual, (role, reference))| {
                actual.role != role
                    || actual.declared_path != reference.path
                    || actual.declared_sha256 != reference.sha256
            })
    {
        return Err(route_message(
            "scratch evidence source authorities are incomplete",
        ));
    }
    Ok(())
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

pub(super) fn read_json_blob<T: serde::de::DeserializeOwned>(
    bundle_root: &Path,
    artifact: &NativeTacticScratchBundleArtifact,
) -> Result<T, NativeTacticRouteRunError> {
    serde_json::from_slice(&read_blob(bundle_root, artifact)?).map_err(route_error)
}

pub(super) fn read_blob(
    bundle_root: &Path,
    artifact: &NativeTacticScratchBundleArtifact,
) -> Result<Vec<u8>, NativeTacticRouteRunError> {
    ContentStore::open(bundle_root)
        .map_err(route_error)?
        .read_bytes(&artifact.blob)
        .map_err(route_error)
}

pub(super) fn blob_path(bundle_root: &Path, blob: &ContentBlob) -> PathBuf {
    bundle_root.join(&blob.relative_path)
}

fn canonical_json_sha256<T: Serialize>(value: &T) -> Result<Digest, NativeTacticRouteRunError> {
    Ok(Digest(
        Sha256::digest(serde_json::to_vec(value).map_err(route_error)?).into(),
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
            "scratch evidence source is outside the repository or not a file",
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

    struct TestRoot(PathBuf);

    impl TestRoot {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "dusklight-scratch-bundle-{name}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir(&root).unwrap();
            Self(root)
        }
    }

    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn bundled_bytes_are_rejected_after_content_tampering() {
        let root = TestRoot::new("tamper");
        let store = ContentStore::initialize(&root.0).unwrap();
        let artifact = bundle_bytes(
            &store,
            b"terminal evidence",
            ContentKind::CrashArtifact,
            Digest([7; 32]),
        )
        .unwrap();
        assert_eq!(read_blob(&root.0, &artifact).unwrap(), b"terminal evidence");

        fs::write(blob_path(&root.0, &artifact.blob), b"altered").unwrap();
        assert!(read_blob(&root.0, &artifact).is_err());
    }

    #[test]
    fn authority_publication_requires_the_declared_digest() {
        let root = TestRoot::new("authority");
        let source = root.0.join("source.json");
        fs::write(&source, b"source authority").unwrap();
        let bundle_root = root.0.join("bundle");
        let store = ContentStore::initialize(&bundle_root).unwrap();
        let reference = ArtifactReference {
            path: "source.json".into(),
            sha256: Digest([9; 32]),
        };

        assert!(
            bundle_authority(
                &store,
                &root.0.canonicalize().unwrap(),
                "source",
                &reference,
                ContentKind::DatasetManifest,
            )
            .is_err()
        );
    }

    #[test]
    fn confined_sources_cannot_escape_the_declared_repository() {
        let repository = TestRoot::new("repository");
        let outside = TestRoot::new("outside");
        let inside_file = repository.0.join("inside");
        let outside_file = outside.0.join("outside");
        fs::write(&inside_file, b"inside").unwrap();
        fs::write(&outside_file, b"outside").unwrap();
        let repository = repository.0.canonicalize().unwrap();

        assert_eq!(
            confined_source(&repository, Path::new("inside")).unwrap(),
            inside_file.canonicalize().unwrap()
        );
        assert!(confined_source(&repository, &outside_file).is_err());
    }

    #[test]
    fn failed_bundle_build_removes_only_its_new_output_root() {
        let parent = TestRoot::new("build-guard");
        let output = parent.0.join("new-bundle");
        ContentStore::initialize(&output).unwrap();
        {
            let _guard = BundleBuildGuard::new(&output);
        }
        assert!(!output.exists());
        assert!(parent.0.exists());
    }

    #[test]
    fn terminal_tape_ticks_are_relative_to_the_authenticated_source_boundary() {
        let tape = InputTape {
            frames: vec![InputFrame::default(); 506 + 123 + 1],
            ..InputTape::default()
        };
        assert_eq!(terminal_tape_first_hit_tick(&tape, 506), Some(123));
        assert_eq!(terminal_tape_first_hit_tick(&tape, 630), None);
    }
}
