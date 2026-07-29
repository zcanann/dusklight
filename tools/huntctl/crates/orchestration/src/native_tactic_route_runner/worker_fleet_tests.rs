use super::*;

#[test]
fn persistent_fleet_views_select_a_bounded_prefix_and_reset_dispatch_order() {
    let (senders, _receivers): (Vec<_>, Vec<_>) = (0..4)
        .map(|_| mpsc::channel::<NativeTacticProposalJob>())
        .unzip();
    let one = proposal_pool_view(
        &senders,
        1,
        true,
        440,
        NativeGenericExecutionStrategy::NativeController,
        Digest([1; 32]),
    )
    .unwrap();
    let four = proposal_pool_view(
        &senders,
        4,
        false,
        440,
        NativeGenericExecutionStrategy::ProgressiveAudit,
        Digest([2; 32]),
    )
    .unwrap();

    assert_eq!(one.senders.len(), 1);
    assert_eq!(four.senders.len(), 4);
    assert_eq!(one.next_worker.load(Ordering::Relaxed), 0);
    assert_eq!(four.next_worker.load(Ordering::Relaxed), 0);
    assert!(one.direct_restore_enabled);
    assert!(!four.direct_restore_enabled);
    assert_eq!(one.execution_plan_sha256, Digest([1; 32]));
    assert_eq!(four.execution_plan_sha256, Digest([2; 32]));
    assert!(
        proposal_pool_view(
            &senders,
            0,
            false,
            440,
            NativeGenericExecutionStrategy::NativeController,
            Digest([3; 32]),
        )
        .is_err()
    );
    assert!(
        proposal_pool_view(
            &senders,
            5,
            false,
            440,
            NativeGenericExecutionStrategy::NativeController,
            Digest([4; 32]),
        )
        .is_err()
    );
}
