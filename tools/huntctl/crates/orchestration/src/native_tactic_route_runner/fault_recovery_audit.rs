use super::*;

pub const NATIVE_TACTIC_FAULT_RECOVERY_AUDIT_SCHEMA_V1: &str =
    "dusklight-native-tactic-fault-recovery-audit/v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticFaultRecoverySeedAudit {
    pub seed: u64,
    pub control_semantic_trace_sha256: Digest,
    pub recovered_semantic_trace_sha256: Digest,
    pub semantic_trace_equal: bool,
    pub useful_expansion_set_equal: bool,
    pub graph_shape_equal: bool,
    pub replay_shape_equal: bool,
    pub terminal_result_equal: bool,
    pub expected_retryable_dispatches: u64,
    pub observed_retryable_dispatches: u64,
    pub lease_accounting_exact: bool,
    pub unresolved_leases: u64,
    pub passed: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticFaultRecoveryAudit {
    pub schema: String,
    pub content_sha256: Digest,
    pub control_report_sha256: Digest,
    pub recovered_report_sha256: Digest,
    pub execution_plan_sha256: Digest,
    pub platform_os: String,
    pub platform_arch: String,
    pub marker: NativeTacticFaultInjectionMarker,
    pub campaign_identity_equal: bool,
    pub seed: NativeTacticFaultRecoverySeedAudit,
    pub passed: bool,
}

impl NativeTacticFaultRecoveryAudit {
    pub fn validate(&self) -> Result<(), NativeTacticRouteRunError> {
        self.marker.validate()?;
        let conditions = self.schema == NATIVE_TACTIC_FAULT_RECOVERY_AUDIT_SCHEMA_V1
            && self.content_sha256 != Digest::ZERO
            && self.control_report_sha256 != Digest::ZERO
            && self.recovered_report_sha256 != Digest::ZERO
            && self.execution_plan_sha256 == self.marker.execution_plan_sha256
            && self.marker.seed == self.seed.seed
            && self.seed.semantic_trace_equal
            && self.seed.useful_expansion_set_equal
            && self.seed.graph_shape_equal
            && self.seed.replay_shape_equal
            && self.seed.terminal_result_equal
            && self.seed.expected_retryable_dispatches == self.seed.observed_retryable_dispatches
            && self.seed.lease_accounting_exact
            && self.seed.unresolved_leases == 0
            && self.seed.passed
            && self.campaign_identity_equal
            && self.passed;
        if !conditions || self.content_sha256 != fault_recovery_audit_digest(self)? {
            return Err(route_message(
                "native tactic fault-recovery audit is invalid",
            ));
        }
        Ok(())
    }
}

pub fn audit_native_tactic_fault_recovery(
    control_report_path: &Path,
    recovered_report_path: &Path,
    output_path: &Path,
) -> Result<NativeTacticFaultRecoveryAudit, NativeTacticRouteRunError> {
    let control_bytes = fs::read(control_report_path).map_err(route_error)?;
    let recovered_bytes = fs::read(recovered_report_path).map_err(route_error)?;
    let control: NativeTacticRouteReport =
        serde_json::from_slice(&control_bytes).map_err(route_error)?;
    let recovered: NativeTacticRouteReport =
        serde_json::from_slice(&recovered_bytes).map_err(route_error)?;
    if control.schema != NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V36
        || recovered.schema != NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V36
        || control.seeds.len() != 1
        || recovered.seeds.len() != 1
    {
        return Err(route_message(
            "fault recovery requires matched single-seed v36 native tactic reports",
        ));
    }
    let marker = read_fault_marker(recovered_report_path)?;
    let control_seed = &control.seeds[0];
    let recovered_seed = &recovered.seeds[0];
    if control_seed.seed != marker.seed
        || recovered_seed.seed != marker.seed
        || marker.execution_plan_sha256 != recovered.execution_plan_sha256
    {
        return Err(route_message(
            "native tactic fault marker is detached from its matched reports",
        ));
    }
    let control_trace_sha256 = semantic_trace_sha256(&control_seed.trace)?;
    let recovered_trace_sha256 = semantic_trace_sha256(&recovered_seed.trace)?;
    let semantic_trace_equal = control_trace_sha256 == recovered_trace_sha256;
    let useful_expansion_set_equal = control_seed.useful_graph_expansion_set_sha256
        == recovered_seed.useful_graph_expansion_set_sha256;
    let control_metrics = control_seed
        .graph_metrics
        .as_ref()
        .ok_or_else(|| route_message("control report lacks graph metrics"))?;
    let recovered_metrics = recovered_seed
        .graph_metrics
        .as_ref()
        .ok_or_else(|| route_message("recovered report lacks graph metrics"))?;
    let control_graph = &control_metrics.graph;
    let recovered_graph = &recovered_metrics.graph;
    let graph_shape_equal = control_graph.nodes == recovered_graph.nodes
        && control_graph.observed_segments == recovered_graph.observed_segments
        && control_graph.untried_expansions == recovered_graph.untried_expansions
        && control_graph.leased_expansions == recovered_graph.leased_expansions
        && control_graph.retryable_expansions == recovered_graph.retryable_expansions
        && control_graph.completed_expansions == recovered_graph.completed_expansions
        && control_graph.failed_validation_expansions
            == recovered_graph.failed_validation_expansions
        && control_graph.best_terminal == recovered_graph.best_terminal;
    let replay_shape_equal = control_seed.decisions == recovered_seed.decisions
        && control_seed.episodes == recovered_seed.episodes
        && control_seed.native_ticks == recovered_seed.native_ticks
        && control_seed.training_replay_rows == recovered_seed.training_replay_rows
        && control_seed.imported_training_replay_rows
            == recovered_seed.imported_training_replay_rows
        && control_seed.duplicate_training_transitions
            == recovered_seed.duplicate_training_transitions
        && control_seed.censored_training_transitions
            == recovered_seed.censored_training_transitions;
    let terminal_result_equal = control_seed.terminal_discovered
        == recovered_seed.terminal_discovered
        && control_seed.best_authenticated_tick == recovered_seed.best_authenticated_tick
        && control_seed.best_terminal_state_sha256 == recovered_seed.best_terminal_state_sha256
        && control_seed.best_terminal_route_checkpoint_sha256
            == recovered_seed.best_terminal_route_checkpoint_sha256;
    let proposal_count = control_seed
        .trace
        .iter()
        .find(|decision| decision.decision_index == marker.decision_index)
        .map(|decision| decision.proposal_batch.len() as u64)
        .ok_or_else(|| route_message("fault decision is absent from the control trace"))?;
    let expected_retryable_dispatches = expected_retryable_dispatches(marker.point, proposal_count);
    let control_leases = &control_metrics.lease_accounting;
    let recovered_leases = &recovered_metrics.lease_accounting;
    let observed_retryable_dispatches = recovered_leases
        .retryable_leases
        .saturating_sub(control_leases.retryable_leases);
    let lease_accounting_exact = recovered_leases.proposal_dispatches
        == control_leases
            .proposal_dispatches
            .saturating_add(expected_retryable_dispatches)
        && recovered_leases.completed_leases == control_leases.completed_leases
        && recovered_leases.retryable_leases
            == control_leases
                .retryable_leases
                .saturating_add(expected_retryable_dispatches)
        && recovered_leases.cancelled_leases == control_leases.cancelled_leases
        && recovered_leases.failed_leases == control_leases.failed_leases
        && recovered_leases.unresolved_leases == 0;
    let seed_passed = semantic_trace_equal
        && useful_expansion_set_equal
        && graph_shape_equal
        && replay_shape_equal
        && terminal_result_equal
        && lease_accounting_exact;
    let campaign_identity_equal = control.optimization_request_sha256
        == recovered.optimization_request_sha256
        && control.execution_binding_sha256 == recovered.execution_binding_sha256
        && control.execution_plan_sha256 == recovered.execution_plan_sha256
        && control.objective_sha256 == recovered.objective_sha256
        && control.feature_schema_sha256 == recovered.feature_schema_sha256
        && control.action_schema_sha256 == recovered.action_schema_sha256
        && control.exploration_seeds == recovered.exploration_seeds
        && control.proposal_policy == recovered.proposal_policy
        && control.value_treatment == recovered.value_treatment
        && control.execution_strategy == recovered.execution_strategy
        && control.resource_budgets == recovered.resource_budgets;
    let mut audit = NativeTacticFaultRecoveryAudit {
        schema: NATIVE_TACTIC_FAULT_RECOVERY_AUDIT_SCHEMA_V1.into(),
        content_sha256: Digest::ZERO,
        control_report_sha256: Digest(Sha256::digest(control_bytes).into()),
        recovered_report_sha256: Digest(Sha256::digest(recovered_bytes).into()),
        execution_plan_sha256: control.execution_plan_sha256,
        platform_os: std::env::consts::OS.into(),
        platform_arch: std::env::consts::ARCH.into(),
        marker,
        campaign_identity_equal,
        seed: NativeTacticFaultRecoverySeedAudit {
            seed: control_seed.seed,
            control_semantic_trace_sha256: control_trace_sha256,
            recovered_semantic_trace_sha256: recovered_trace_sha256,
            semantic_trace_equal,
            useful_expansion_set_equal,
            graph_shape_equal,
            replay_shape_equal,
            terminal_result_equal,
            expected_retryable_dispatches,
            observed_retryable_dispatches,
            lease_accounting_exact,
            unresolved_leases: recovered_leases.unresolved_leases,
            passed: seed_passed,
        },
        passed: campaign_identity_equal && seed_passed,
    };
    audit.content_sha256 = fault_recovery_audit_digest(&audit)?;
    audit.validate()?;
    write_new(
        output_path,
        &serde_json::to_vec_pretty(&audit).map_err(route_error)?,
    )?;
    Ok(audit)
}

fn read_fault_marker(
    recovered_report_path: &Path,
) -> Result<NativeTacticFaultInjectionMarker, NativeTacticRouteRunError> {
    let campaign_root = recovered_report_path
        .parent()
        .ok_or_else(|| route_message("recovered report has no campaign root"))?;
    let mut markers = Vec::new();
    for entry in fs::read_dir(campaign_root).map_err(route_error)? {
        let entry = entry.map_err(route_error)?;
        if !entry.file_type().map_err(route_error)?.is_dir()
            || !entry.file_name().to_string_lossy().starts_with("seed-")
        {
            continue;
        }
        let marker_path = entry.path().join(NATIVE_TACTIC_FAULT_INJECTION_FILE);
        if marker_path.is_file() {
            let marker: NativeTacticFaultInjectionMarker =
                serde_json::from_slice(&fs::read(marker_path).map_err(route_error)?)
                    .map_err(route_error)?;
            marker.validate()?;
            markers.push(marker);
        }
    }
    if markers.len() != 1 {
        return Err(route_message(
            "recovered campaign must contain exactly one fault-injection marker",
        ));
    }
    Ok(markers.pop().expect("length checked"))
}

fn expected_retryable_dispatches(point: NativeTacticFaultPoint, proposal_count: u64) -> u64 {
    match point {
        NativeTacticFaultPoint::AfterDecisionCommit => 0,
        NativeTacticFaultPoint::BeforeDispatch
        | NativeTacticFaultPoint::DuringExecution
        | NativeTacticFaultPoint::AfterNativeCompletion
        | NativeTacticFaultPoint::AfterRecoveryPointCommit => proposal_count,
    }
}

fn semantic_trace_sha256(
    trace: &[NativeTacticDecisionTrace],
) -> Result<Digest, NativeTacticRouteRunError> {
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight/native-tactic/recovered-semantic-trace/v1");
    hasher.update((trace.len() as u64).to_le_bytes());
    for decision in trace {
        hasher.update(decision.execution_plan_sha256.0);
        hasher.update(decision.decision_index.to_le_bytes());
        hasher.update(decision.learner_snapshot_sha256.0);
        hasher.update(decision.replay_rows_at_decision.to_le_bytes());
        hasher.update(decision.replay_generation.to_le_bytes());
        hasher.update((decision.lane_index as u64).to_le_bytes());
        hash_cbor(&mut hasher, &decision.lane_role)?;
        hasher.update(decision.acquisition_rank.to_le_bytes());
        hasher.update(decision.frontier_identity.0);
        hasher.update(decision.episode.to_le_bytes());
        hasher.update(decision.source_route_ticks.to_le_bytes());
        hasher.update(decision.route_suffix_ticks.to_le_bytes());
        hash_cbor(&mut hasher, &decision.selected_option_id)?;
        hash_cbor(&mut hasher, &decision.selection_reason)?;
        hash_cbor(&mut hasher, &decision.scheduler_decision)?;
        hash_cbor(&mut hasher, &decision.branch_acquisition)?;
        hasher.update(decision.before.snapshot_sha256.0);
        hasher.update(decision.after.snapshot_sha256.0);
        hasher.update([u8::from(decision.terminal)]);
        hasher.update((decision.proposal_batch.len() as u64).to_le_bytes());
        for proposal in &decision.proposal_batch {
            hasher.update(proposal.execution_plan_sha256.0);
            hash_cbor(&mut hasher, &proposal.option_id)?;
            hash_cbor(&mut hasher, &proposal.selection_reason)?;
            hasher.update(proposal.realized_ticks.to_le_bytes());
            hasher.update(proposal.root_route_ticks.to_le_bytes());
            hasher.update(proposal.emitted_tape_sha256.0);
            hasher.update([u8::from(proposal.terminal)]);
            hasher.update(proposal.after_snapshot_sha256.0);
            hasher.update([u8::from(proposal.retained)]);
        }
    }
    Ok(Digest(hasher.finalize().into()))
}

fn hash_cbor<T: Serialize>(
    hasher: &mut Sha256,
    value: &T,
) -> Result<(), NativeTacticRouteRunError> {
    let bytes = serde_cbor::to_vec(value).map_err(route_error)?;
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
    Ok(())
}

fn fault_recovery_audit_digest(
    audit: &NativeTacticFaultRecoveryAudit,
) -> Result<Digest, NativeTacticRouteRunError> {
    let mut unsigned = audit.clone();
    unsigned.content_sha256 = Digest::ZERO;
    Ok(Digest(
        Sha256::digest(serde_cbor::to_vec(&unsigned).map_err(route_error)?).into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_audit() -> NativeTacticFaultRecoveryAudit {
        let mut audit = NativeTacticFaultRecoveryAudit {
            schema: NATIVE_TACTIC_FAULT_RECOVERY_AUDIT_SCHEMA_V1.into(),
            content_sha256: Digest::ZERO,
            control_report_sha256: Digest([1; 32]),
            recovered_report_sha256: Digest([2; 32]),
            execution_plan_sha256: Digest([3; 32]),
            platform_os: "test".into(),
            platform_arch: "test".into(),
            marker: NativeTacticFaultInjectionMarker {
                schema: NATIVE_TACTIC_FAULT_INJECTION_SCHEMA_V1.into(),
                execution_plan_sha256: Digest([3; 32]),
                seed: 17,
                decision_index: 2,
                point: NativeTacticFaultPoint::DuringExecution,
            },
            campaign_identity_equal: true,
            seed: NativeTacticFaultRecoverySeedAudit {
                seed: 17,
                control_semantic_trace_sha256: Digest([4; 32]),
                recovered_semantic_trace_sha256: Digest([4; 32]),
                semantic_trace_equal: true,
                useful_expansion_set_equal: true,
                graph_shape_equal: true,
                replay_shape_equal: true,
                terminal_result_equal: true,
                expected_retryable_dispatches: 4,
                observed_retryable_dispatches: 4,
                lease_accounting_exact: true,
                unresolved_leases: 0,
                passed: true,
            },
            passed: true,
        };
        audit.content_sha256 = fault_recovery_audit_digest(&audit).unwrap();
        audit
    }

    #[test]
    fn every_precommit_loss_retries_the_exact_dispatched_batch() {
        for point in [
            NativeTacticFaultPoint::BeforeDispatch,
            NativeTacticFaultPoint::DuringExecution,
            NativeTacticFaultPoint::AfterNativeCompletion,
            NativeTacticFaultPoint::AfterRecoveryPointCommit,
        ] {
            assert_eq!(expected_retryable_dispatches(point, 4), 4);
        }
        assert_eq!(
            expected_retryable_dispatches(NativeTacticFaultPoint::AfterDecisionCommit, 4),
            0
        );
    }

    #[test]
    fn resealed_retry_mismatch_fails_validation() {
        let mut audit = valid_audit();
        audit.seed.observed_retryable_dispatches = 3;
        audit.content_sha256 = fault_recovery_audit_digest(&audit).unwrap();
        assert!(audit.validate().is_err());
    }
}
