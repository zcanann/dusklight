use super::*;
use crate::native_suffix_worker::NativeSuffixWorkerIdentity;

struct MissingCheckpointWorker {
    identity: NativeSuffixWorkerIdentity,
}

impl PersistentTacticBatchWorker for MissingCheckpointWorker {
    fn identity(&self) -> &NativeSuffixWorkerIdentity {
        &self.identity
    }

    fn run_tactic_batch(
        &mut self,
        _request: &Path,
        _result: &Path,
        _batch: &dusklight_search::suffix_batch::NativeSuffixBatch,
    ) -> Result<ValidatedNativeSuffixBatch, NativeTacticWorkerError> {
        Err(NativeTacticWorkerError::Worker(
            NativeSuffixWorkerError::Rejected {
                code: "batch_rejected".into(),
                message: "requested process-local checkpoint is absent or invalid".into(),
            },
        ))
    }
}

fn proposal_pool(worker_count: usize) -> NativeTacticProposalPool {
    let (senders, _receivers): (Vec<_>, Vec<_>) = (0..worker_count)
        .map(|_| mpsc::channel::<NativeTacticProposalJob>())
        .unzip();
    NativeTacticProposalPool {
        senders: Arc::new(senders),
        next_worker: Arc::new(AtomicUsize::new(0)),
        direct_restore_enabled: true,
        root_source_frame: 506,
        execution_strategy: NativeGenericExecutionStrategy::NativeController,
        execution_plan_sha256: Digest([1; 32]),
        checkpoint_cache_capacity_bytes: TACTIC_CHECKPOINT_CACHE_BYTES,
        dedicated_owner_slots: 0,
        preferred_owner_slot: None,
    }
}

#[test]
fn missing_owner_checkpoint_is_counted_before_exact_replay_fallback() {
    let mut worker = MissingCheckpointWorker {
        identity: NativeSuffixWorkerIdentity {
            executable_sha256: Digest([1; 32]),
            game_data_sha256: Digest([2; 32]),
            input_tape_sha256: Digest([3; 32]),
            milestone_program_sha256: Digest([4; 32]),
            card_fixture_sha256: Digest([5; 32]),
            world_context_sha256: Digest([6; 32]),
            source_frame: 506,
            source_boundary_fingerprint: "7".repeat(32),
            checkpoint_validation_kind: "recorded_replay_window".into(),
            checkpoint_validation_ticks: 8,
            maximum_ticks: 160,
            terminal: NativeTerminalBinding {
                goal: "goal".into(),
                program_sha256: Digest([8; 32]),
                definition_sha256: Digest([9; 32]),
            },
        },
    };
    let mut timed = TimedTacticWorker::new(&mut worker);

    let batch = dusklight_search::suffix_batch::NativeSuffixBatch {
        schema: String::new(),
        source_frame: 0,
        source_boundary_fingerprint: String::new(),
        checkpoint_validation: dusklight_search::suffix_batch::NativeCheckpointValidation {
            kind: String::new(),
            ticks: 0,
        },
        maximum_ticks: 0,
        verify_state_hashes: false,
        checkpoint_cache: None,
        candidates: Vec::new(),
    };
    let error = timed
        .run_tactic_batch(Path::new("request"), Path::new("result"), &batch)
        .unwrap_err();
    assert!(error.is_missing_process_local_checkpoint());
    timed.record_route_replay(530).unwrap();
    let direct_replay_accounting = timed.take_accounting();

    assert_eq!(direct_replay_accounting.cache_misses, 1);
    assert_eq!(direct_replay_accounting.replayed_prefix_ticks, 24);
    assert_eq!(direct_replay_accounting.replay_restore_micros, 0);

    timed
        .record_prefix_materialization(530, true, Duration::from_micros(25))
        .unwrap();
    let accounting = timed.take_accounting();

    assert_eq!(accounting.prefix_materializations, 1);
    assert_eq!(accounting.replayed_prefix_ticks, 24);
    assert_eq!(accounting.replay_restore_micros, 25);
    assert_eq!(accounting.direct_restore_fallback_replays, 1);
}

#[test]
fn every_selected_decision_retains_a_single_use_live_endpoint() {
    assert_eq!(
        primary_checkpoint_retention(true),
        NativeTacticCheckpointRetention::LiveEndpoint
    );
    assert_eq!(
        primary_checkpoint_retention(false),
        NativeTacticCheckpointRetention::None
    );
}

#[test]
fn selected_live_frontiers_remain_directly_eligible_for_wide_decisions() {
    let frontier = CachedTacticFrontier {
        worker_slot: 1,
        source: NativeTacticCheckpointSource {
            restore_identity: "a".repeat(32),
            boundary_fingerprint: "b".repeat(32),
            route_ticks: 40,
            storage: NativeTacticCheckpointStorage::LiveEndpoint,
        },
        state_sha256: Digest([3; 32]),
        route_frames: 546,
        route_checkpoint_sha256: Digest([4; 32]),
        route_tape_sha256: Digest([5; 32]),
    };
    assert!(proposal_pool(2).direct_frontier_eligible(&frontier));

    let mut portable = frontier;
    portable.source.storage = NativeTacticCheckpointStorage::PortableImage;
    assert!(proposal_pool(2).direct_frontier_eligible(&portable));
}

#[test]
fn dedicated_lane_owners_are_never_used_for_counterfactual_siblings() {
    assert_eq!(dedicated_owner_slot_count(16, 4, 4, true), 4);
    assert_eq!(dedicated_owner_slot_count(15, 4, 4, true), 0);
    assert_eq!(dedicated_owner_slot_count(16, 4, 4, false), 0);
    let mut pool = proposal_pool(16);
    pool.dedicated_owner_slots = 4;
    pool.preferred_owner_slot = Some(2);
    for _ in 0..24 {
        assert!((4..16).contains(&pool.next_counterfactual_worker(Some(2))));
    }
    let owner = CachedTacticFrontier {
        worker_slot: 2,
        source: NativeTacticCheckpointSource {
            restore_identity: "a".repeat(32),
            boundary_fingerprint: "b".repeat(32),
            route_ticks: 40,
            storage: NativeTacticCheckpointStorage::LiveEndpoint,
        },
        state_sha256: Digest([3; 32]),
        route_frames: 546,
        route_checkpoint_sha256: Digest([4; 32]),
        route_tape_sha256: Digest([5; 32]),
    };
    assert!(pool.direct_frontier_eligible(&owner));
    let mut another_lane = owner;
    another_lane.worker_slot = 1;
    assert!(!pool.direct_frontier_eligible(&another_lane));
}

#[test]
fn counterfactual_replay_never_rearms_the_live_endpoint_owner_when_pool_is_wide() {
    let next = AtomicUsize::new(0);
    for _ in 0..8 {
        assert_ne!(next_worker_excluding(&next, 4, Some(2)), 2);
    }
    assert_eq!(next_worker_excluding(&next, 1, Some(0)), 0);
}

#[test]
fn uncached_non_root_graph_expansion_materializes_before_its_action() {
    assert!(requires_frontier_materialization(true, 40, false));
    assert!(!requires_frontier_materialization(true, 0, false));
    assert!(!requires_frontier_materialization(true, 40, true));
    assert!(!requires_frontier_materialization(false, 40, false));
}

#[test]
fn concurrent_proposals_have_disjoint_execution_and_replay_artifact_roots() {
    let decision_root = Path::new("decision-000001");
    let first = proposal_artifact_root(decision_root, 0);
    let sibling = proposal_artifact_root(decision_root, 2);

    assert_eq!(first, decision_root.join("proposal-000"));
    assert_eq!(sibling, decision_root.join("proposal-002"));
    assert_ne!(
        first.join("frontier-source"),
        sibling.join("frontier-source")
    );
    assert_ne!(
        first.join("frontier-replay-fallback"),
        sibling.join("frontier-replay-fallback")
    );
}
