use super::*;

pub(super) fn tactic_graph_metrics(
    graph: &crate::state_graph::StateGraph,
    graph_sha256: Digest,
    trace: &[NativeTacticDecisionTrace],
    lease_accounting: NativeTacticLeaseAccounting,
) -> Result<NativeTacticGraphMetrics, NativeTacticRouteRunError> {
    let graph_report =
        GraphSearchReport::from_validated_graph(graph, graph_sha256).map_err(route_error)?;
    let completed_trace_dispatches = trace.iter().try_fold(0_u64, |total, decision| {
        total
            .checked_add(u64::try_from(decision.proposal_batch.len()).map_err(route_error)?)
            .ok_or_else(|| route_message("completed tactic lease count overflowed"))
    })?;
    lease_accounting.validate()?;
    if lease_accounting.completed_leases != completed_trace_dispatches
        || lease_accounting.unresolved_leases != 0
    {
        return Err(route_message(
            "native tactic lease accounting is detached from durable completed decisions",
        ));
    }
    let duplicate_transpositions = graph_report
        .observed_segments
        .saturating_add(1)
        .saturating_sub(graph_report.nodes);
    let terminal_paths =
        u64::try_from(graph.nodes().filter(|node| node.terminal).count()).map_err(route_error)?;
    if terminal_paths == 0 && graph_report.best_terminal.is_some()
        || terminal_paths > 0 && graph_report.best_terminal.is_none()
    {
        return Err(route_message(
            "native tactic graph metrics are detached from terminal paths",
        ));
    }
    Ok(NativeTacticGraphMetrics {
        graph: graph_report,
        lease_accounting,
        duplicate_transpositions,
        terminal_paths,
    })
}
