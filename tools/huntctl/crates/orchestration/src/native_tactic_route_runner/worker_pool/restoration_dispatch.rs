use super::*;

pub(super) fn requires_frontier_materialization(
    has_restoration_contract: bool,
    replayed_prefix: usize,
    has_direct_source: bool,
) -> bool {
    has_restoration_contract && replayed_prefix != 0 && !has_direct_source
}

pub(super) fn validate_restoration_contract(
    restoration: &TacticRestorationContract,
    source_snapshot: &FactSnapshot,
    source_route_tape: &InputTape,
) -> Result<(), NativeTacticRouteRunError> {
    let snapshot_sha256 = source_snapshot.content_sha256().map_err(route_error)?;
    let tape_sha256 =
        Digest(Sha256::digest(source_route_tape.encode().map_err(route_error)?).into());
    let tape_frames = u64::try_from(source_route_tape.frames.len())
        .map_err(|_| route_message("restoration route is too long"))?;
    if restoration.plan.expected_state_sha256 != snapshot_sha256
        || restoration.plan.node.state_sha256 != snapshot_sha256
        || restoration.receipt.restoration_plan_sha256 != restoration.plan.plan_sha256
        || restoration.receipt.node != restoration.plan.node
        || restoration.receipt.observed_state_sha256 != snapshot_sha256
        || restoration.receipt.route_checkpoint_sha256
            != restoration.plan.route.route_checkpoint_sha256
        || restoration.plan.node.route_checkpoint_sha256
            != restoration.plan.route.route_checkpoint_sha256
        || restoration.plan.route.tape_sha256 != tape_sha256
        || restoration.plan.route.tape_frames != tape_frames
    {
        return Err(route_message(
            "native tactic restoration contract is detached from its typed source",
        ));
    }
    Ok(())
}
