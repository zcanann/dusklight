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
    ) -> Result<ValidatedNativeSuffixBatch, NativeTacticWorkerError> {
        Err(NativeTacticWorkerError::Worker(
            NativeSuffixWorkerError::Rejected {
                code: "batch_rejected".into(),
                message: "requested process-local checkpoint is absent or invalid".into(),
            },
        ))
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

    let error = timed
        .run_tactic_batch(Path::new("request"), Path::new("result"))
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
    assert!(direct_frontier_eligible(true, 2, &frontier));

    let mut portable = frontier;
    portable.source.storage = NativeTacticCheckpointStorage::PortableImage;
    assert!(direct_frontier_eligible(true, 2, &portable));
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
