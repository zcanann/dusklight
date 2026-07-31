use super::*;

pub const NATIVE_TACTIC_FAULT_RECOVERY_AUDIT_SCHEMA_V1: &str =
    "dusklight-native-tactic-fault-recovery-audit/v1";
pub const NATIVE_TACTIC_FAULT_RECOVERY_AUDIT_SCHEMA_V2: &str =
    "dusklight-native-tactic-fault-recovery-audit/v2";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticFaultRecoverySeedAudit {
    pub seed: u64,
    pub control_semantic_trace_sha256: Digest,
    pub recovered_semantic_trace_sha256: Digest,
    pub semantic_trace_equal: bool,
    pub useful_expansion_set_equal: bool,
    #[serde(default)]
    pub state_graph_equal: bool,
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
    #[serde(default)]
    pub replay_snapshot_equal: bool,
    #[serde(default)]
    pub learner_authority_equal: bool,
    pub seed: NativeTacticFaultRecoverySeedAudit,
    pub passed: bool,
}

impl NativeTacticFaultRecoveryAudit {
    pub fn validate(&self) -> Result<(), NativeTacticRouteRunError> {
        self.marker.validate()?;
        let legacy = self.schema == NATIVE_TACTIC_FAULT_RECOVERY_AUDIT_SCHEMA_V1;
        let current = self.schema == NATIVE_TACTIC_FAULT_RECOVERY_AUDIT_SCHEMA_V2;
        let current_exact_authorities = self.seed.state_graph_equal
            && self.replay_snapshot_equal
            && self.learner_authority_equal;
        let conditions = (legacy || current)
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
            && (legacy || current_exact_authorities)
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
    let (_, marker) = read_fault_marker_source(recovered_report_path)?;
    let audit = build_native_tactic_fault_recovery_audit(&control_bytes, &recovered_bytes, marker)?;
    write_new(
        output_path,
        &serde_json::to_vec_pretty(&audit).map_err(route_error)?,
    )?;
    Ok(audit)
}

pub(super) fn build_native_tactic_fault_recovery_audit(
    control_bytes: &[u8],
    recovered_bytes: &[u8],
    marker: NativeTacticFaultInjectionMarker,
) -> Result<NativeTacticFaultRecoveryAudit, NativeTacticRouteRunError> {
    let control: NativeTacticRouteReport =
        serde_json::from_slice(control_bytes).map_err(route_error)?;
    let recovered: NativeTacticRouteReport =
        serde_json::from_slice(recovered_bytes).map_err(route_error)?;
    if control.schema != NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V38
        || recovered.schema != NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V38
        || control.seeds.len() != 1
        || recovered.seeds.len() != 1
    {
        return Err(route_message(
            "fault recovery requires matched single-seed v37 native tactic reports",
        ));
    }
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
    let control_trace_sha256 = semantic_trace_sha256_v2(&control_seed.trace)?;
    let recovered_trace_sha256 = semantic_trace_sha256_v2(&recovered_seed.trace)?;
    let semantic_trace_equal = control_trace_sha256 == recovered_trace_sha256;
    let useful_expansion_set_equal = control_seed.useful_graph_expansion_set_sha256
        == recovered_seed.useful_graph_expansion_set_sha256;
    let state_graph_equal = control_seed.state_graph_sha256 == recovered_seed.state_graph_sha256;
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
        && state_graph_equal
        && graph_shape_equal
        && replay_shape_equal
        && terminal_result_equal
        && lease_accounting_exact;
    let replay_snapshot_equal = control.replay_revision == recovered.replay_revision
        && control.replay_snapshot_sha256 == recovered.replay_snapshot_sha256;
    let learner_authority_equal = control.learner_authority == recovered.learner_authority;
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
        schema: NATIVE_TACTIC_FAULT_RECOVERY_AUDIT_SCHEMA_V2.into(),
        content_sha256: Digest::ZERO,
        control_report_sha256: Digest(Sha256::digest(control_bytes).into()),
        recovered_report_sha256: Digest(Sha256::digest(recovered_bytes).into()),
        execution_plan_sha256: control.execution_plan_sha256,
        platform_os: std::env::consts::OS.into(),
        platform_arch: std::env::consts::ARCH.into(),
        marker,
        campaign_identity_equal,
        replay_snapshot_equal,
        learner_authority_equal,
        seed: NativeTacticFaultRecoverySeedAudit {
            seed: control_seed.seed,
            control_semantic_trace_sha256: control_trace_sha256,
            recovered_semantic_trace_sha256: recovered_trace_sha256,
            semantic_trace_equal,
            useful_expansion_set_equal,
            state_graph_equal,
            graph_shape_equal,
            replay_shape_equal,
            terminal_result_equal,
            expected_retryable_dispatches,
            observed_retryable_dispatches,
            lease_accounting_exact,
            unresolved_leases: recovered_leases.unresolved_leases,
            passed: seed_passed,
        },
        passed: campaign_identity_equal
            && replay_snapshot_equal
            && learner_authority_equal
            && seed_passed,
    };
    audit.content_sha256 = fault_recovery_audit_digest(&audit)?;
    audit.validate()?;
    Ok(audit)
}

pub(super) fn read_fault_marker_source(
    recovered_report_path: &Path,
) -> Result<(PathBuf, NativeTacticFaultInjectionMarker), NativeTacticRouteRunError> {
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
                serde_json::from_slice(&fs::read(&marker_path).map_err(route_error)?)
                    .map_err(route_error)?;
            marker.validate()?;
            markers.push((marker_path, marker));
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

pub(super) fn semantic_trace_sha256_v2(
    trace: &[NativeTacticDecisionTrace],
) -> Result<Digest, NativeTacticRouteRunError> {
    semantic_trace_value_sha256(serde_json::to_value(trace).map_err(route_error)?)
}

fn semantic_trace_value_sha256(
    mut trace: serde_json::Value,
) -> Result<Digest, NativeTacticRouteRunError> {
    let decisions = trace
        .as_array_mut()
        .ok_or_else(|| route_message("native tactic semantic trace is not an array"))?;
    for decision in decisions {
        let decision = decision
            .as_object_mut()
            .ok_or_else(|| route_message("native tactic semantic decision is not an object"))?;
        for physical_field in [
            "cumulative_wall_micros",
            "checkpoint_owner_worker_slot",
            "proposal_worker_slots",
            "restore_source",
            "directly_restorable_native_frontiers",
            "replay_only_frontiers",
        ] {
            decision.remove(physical_field);
        }
        if let Some(scheduler) = decision
            .get_mut("scheduler_decision")
            .and_then(serde_json::Value::as_object_mut)
        {
            for run_specific_seal in ["graph_sha256", "queue_sha256", "decision_sha256"] {
                scheduler.remove(run_specific_seal);
            }
        }
    }
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight/native-tactic/recovered-semantic-trace/v2");
    hash_cbor(&mut hasher, &trace)?;
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
    if audit.schema == NATIVE_TACTIC_FAULT_RECOVERY_AUDIT_SCHEMA_V1 {
        return legacy_fault_recovery_audit_digest(audit);
    }
    let mut unsigned = audit.clone();
    unsigned.content_sha256 = Digest::ZERO;
    Ok(Digest(
        Sha256::digest(serde_cbor::to_vec(&unsigned).map_err(route_error)?).into(),
    ))
}

#[derive(Serialize)]
struct LegacyNativeTacticFaultRecoverySeedAudit {
    seed: u64,
    control_semantic_trace_sha256: Digest,
    recovered_semantic_trace_sha256: Digest,
    semantic_trace_equal: bool,
    useful_expansion_set_equal: bool,
    graph_shape_equal: bool,
    replay_shape_equal: bool,
    terminal_result_equal: bool,
    expected_retryable_dispatches: u64,
    observed_retryable_dispatches: u64,
    lease_accounting_exact: bool,
    unresolved_leases: u64,
    passed: bool,
}

#[derive(Serialize)]
struct LegacyNativeTacticFaultRecoveryAudit<'a> {
    schema: &'a str,
    content_sha256: Digest,
    control_report_sha256: Digest,
    recovered_report_sha256: Digest,
    execution_plan_sha256: Digest,
    platform_os: &'a str,
    platform_arch: &'a str,
    marker: &'a NativeTacticFaultInjectionMarker,
    campaign_identity_equal: bool,
    seed: LegacyNativeTacticFaultRecoverySeedAudit,
    passed: bool,
}

fn legacy_fault_recovery_audit_digest(
    audit: &NativeTacticFaultRecoveryAudit,
) -> Result<Digest, NativeTacticRouteRunError> {
    let seed = &audit.seed;
    let legacy = LegacyNativeTacticFaultRecoveryAudit {
        schema: &audit.schema,
        content_sha256: Digest::ZERO,
        control_report_sha256: audit.control_report_sha256,
        recovered_report_sha256: audit.recovered_report_sha256,
        execution_plan_sha256: audit.execution_plan_sha256,
        platform_os: &audit.platform_os,
        platform_arch: &audit.platform_arch,
        marker: &audit.marker,
        campaign_identity_equal: audit.campaign_identity_equal,
        seed: LegacyNativeTacticFaultRecoverySeedAudit {
            seed: seed.seed,
            control_semantic_trace_sha256: seed.control_semantic_trace_sha256,
            recovered_semantic_trace_sha256: seed.recovered_semantic_trace_sha256,
            semantic_trace_equal: seed.semantic_trace_equal,
            useful_expansion_set_equal: seed.useful_expansion_set_equal,
            graph_shape_equal: seed.graph_shape_equal,
            replay_shape_equal: seed.replay_shape_equal,
            terminal_result_equal: seed.terminal_result_equal,
            expected_retryable_dispatches: seed.expected_retryable_dispatches,
            observed_retryable_dispatches: seed.observed_retryable_dispatches,
            lease_accounting_exact: seed.lease_accounting_exact,
            unresolved_leases: seed.unresolved_leases,
            passed: seed.passed,
        },
        passed: audit.passed,
    };
    Ok(Digest(
        Sha256::digest(serde_cbor::to_vec(&legacy).map_err(route_error)?).into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_audit() -> NativeTacticFaultRecoveryAudit {
        let mut audit = NativeTacticFaultRecoveryAudit {
            schema: NATIVE_TACTIC_FAULT_RECOVERY_AUDIT_SCHEMA_V2.into(),
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
            replay_snapshot_equal: true,
            learner_authority_equal: true,
            seed: NativeTacticFaultRecoverySeedAudit {
                seed: 17,
                control_semantic_trace_sha256: Digest([4; 32]),
                recovered_semantic_trace_sha256: Digest([4; 32]),
                semantic_trace_equal: true,
                useful_expansion_set_equal: true,
                state_graph_equal: true,
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

    #[test]
    fn semantic_trace_v2_ignores_physical_placement_and_run_specific_graph_seals() {
        let trace = serde_json::json!([{
            "decision_index": 2,
            "cumulative_wall_micros": 10,
            "checkpoint_owner_worker_slot": 0,
            "proposal_worker_slots": [0, 1],
            "restore_source": "process_local_checkpoint",
            "directly_restorable_native_frontiers": 3,
            "replay_only_frontiers": 1,
            "scheduler_decision": {
                "graph_sha256": "21",
                "queue_sha256": "22",
                "decision_sha256": "23",
                "selected_expansion": "semantic"
            },
            "reward": -0.25,
            "measurements": [{"name": "speed", "before": 4.0, "after": 3.0}],
            "proposal_batch": [{
                "option_id": "roll",
                "emitted_tape_sha256": "11"
            }]
        }]);
        let expected = semantic_trace_value_sha256(trace.clone()).unwrap();

        let mut physical_change = trace.clone();
        let decision = physical_change[0].as_object_mut().unwrap();
        decision.insert("cumulative_wall_micros".into(), 99.into());
        decision.insert("proposal_worker_slots".into(), serde_json::json!([7, 6]));
        physical_change[0]["scheduler_decision"]["graph_sha256"] = serde_json::json!("31");
        physical_change[0]["scheduler_decision"]["queue_sha256"] = serde_json::json!("32");
        physical_change[0]["scheduler_decision"]["decision_sha256"] = serde_json::json!("33");
        assert_eq!(
            expected,
            semantic_trace_value_sha256(physical_change).unwrap()
        );

        let mut semantic_change = trace;
        semantic_change[0]["reward"] = serde_json::json!(-0.5);
        assert_ne!(
            expected,
            semantic_trace_value_sha256(semantic_change).unwrap()
        );
    }

    #[test]
    fn v2_rejects_resealed_exact_authority_drift() {
        for mutate in [
            |audit: &mut NativeTacticFaultRecoveryAudit| audit.seed.state_graph_equal = false,
            |audit: &mut NativeTacticFaultRecoveryAudit| audit.replay_snapshot_equal = false,
            |audit: &mut NativeTacticFaultRecoveryAudit| audit.learner_authority_equal = false,
        ] {
            let mut audit = valid_audit();
            mutate(&mut audit);
            audit.content_sha256 = fault_recovery_audit_digest(&audit).unwrap();
            assert!(audit.validate().is_err());
        }
    }

    #[test]
    fn legacy_v1_digest_remains_valid_without_v2_authorities() {
        let mut audit = valid_audit();
        audit.schema = NATIVE_TACTIC_FAULT_RECOVERY_AUDIT_SCHEMA_V1.into();
        audit.seed.state_graph_equal = false;
        audit.replay_snapshot_equal = false;
        audit.learner_authority_equal = false;
        audit.content_sha256 = fault_recovery_audit_digest(&audit).unwrap();

        assert!(audit.validate().is_ok());
    }
}
