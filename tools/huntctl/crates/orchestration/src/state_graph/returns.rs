use super::{ExactStateId, StateGraph, StateGraphError};
use std::collections::BTreeMap;

impl StateGraph {
    /// Exact Monte Carlo ticks-to-go for every executable node on any
    /// authenticated terminal tape. Route identity is part of the key:
    /// semantically similar states on different tapes never acquire exact
    /// support merely because their fact digests match.
    pub fn exact_terminal_returns(&self) -> Result<BTreeMap<ExactStateId, u64>, StateGraphError> {
        self.validate()?;
        let terminal_routes = self
            .nodes()
            .filter(|node| node.terminal && node.restoration.executable)
            .map(|node| {
                self.route(node.id.route_checkpoint_sha256)
                    .map(|route| (node, route))
                    .ok_or(StateGraphError::Invariant("terminal node route is absent"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut returns = BTreeMap::<ExactStateId, u64>::new();
        for node in self.nodes().filter(|node| node.restoration.executable) {
            let route =
                self.route(node.id.route_checkpoint_sha256)
                    .ok_or(StateGraphError::Invariant(
                        "executable node route is absent",
                    ))?;
            for (terminal, terminal_route) in &terminal_routes {
                if same_origin(route, terminal_route)
                    && terminal_route.frames.starts_with(&route.frames)
                    && terminal.root_ticks >= node.root_ticks
                {
                    let candidate = terminal.root_ticks - node.root_ticks;
                    returns
                        .entry(node.id)
                        .and_modify(|current| *current = (*current).min(candidate))
                        .or_insert(candidate);
                }
            }
        }
        Ok(returns)
    }
}

fn same_origin(
    left: &dusklight_automation_contracts::tape::InputTape,
    right: &dusklight_automation_contracts::tape::InputTape,
) -> bool {
    left.boot == right.boot
        && left.tick_rate_numerator == right.tick_rate_numerator
        && left.tick_rate_denominator == right.tick_rate_denominator
}
