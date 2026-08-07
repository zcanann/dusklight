use super::{
    ExactStateId, ExactTerminalContinuation, StateGraph, StateGraphError, ValidatedStateGraph,
};
use dusklight_automation_contracts::tape::InputTape;
use std::collections::BTreeMap;

impl StateGraph {
    /// Exact Monte Carlo ticks-to-go for every executable node on any
    /// authenticated terminal tape. Route identity is part of the key:
    /// semantically similar states on different tapes never acquire exact
    /// support merely because their fact digests match.
    pub fn exact_terminal_returns(&self) -> Result<BTreeMap<ExactStateId, u64>, StateGraphError> {
        self.validated()?.exact_terminal_returns()
    }

    /// Return executable evidence, not just scalar value, for the shortest
    /// authenticated terminal continuation on this exact route lineage.
    pub fn exact_terminal_continuation(
        &self,
        source: ExactStateId,
    ) -> Result<Option<ExactTerminalContinuation>, StateGraphError> {
        self.validated()?.exact_terminal_continuation(source)
    }
}

impl ValidatedStateGraph<'_> {
    pub(crate) fn exact_terminal_returns(
        self,
    ) -> Result<BTreeMap<ExactStateId, u64>, StateGraphError> {
        let graph = self.graph();
        let terminal_routes = graph
            .nodes()
            .filter(|node| node.terminal && node.restoration.executable)
            .map(|node| {
                graph
                    .route(node.id.route_checkpoint_sha256)
                    .map(|route| (node, route))
                    .ok_or(StateGraphError::Invariant("terminal node route is absent"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut returns = BTreeMap::<ExactStateId, u64>::new();
        for node in graph.nodes().filter(|node| node.restoration.executable) {
            let route =
                graph
                    .route(node.id.route_checkpoint_sha256)
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

    pub(crate) fn exact_terminal_continuation(
        self,
        source: ExactStateId,
    ) -> Result<Option<ExactTerminalContinuation>, StateGraphError> {
        let graph = self.graph();
        let source_node = graph
            .node(source)
            .filter(|node| node.restoration.executable && !node.terminal)
            .ok_or(StateGraphError::Invalid(
                "terminal continuation source is absent, terminal, or not executable",
            ))?;
        let source_route =
            graph
                .route(source.route_checkpoint_sha256)
                .ok_or(StateGraphError::Invariant(
                    "terminal continuation source route is absent",
                ))?;
        let source_frames = source_route.frames.len();
        let mut candidates = graph
            .nodes()
            .filter(|node| node.terminal && node.restoration.executable)
            .filter_map(|terminal| {
                let terminal_route = graph.route(terminal.id.route_checkpoint_sha256)?;
                (same_origin(source_route, terminal_route)
                    && terminal_route.frames.starts_with(&source_route.frames)
                    && terminal.root_ticks > source_node.root_ticks)
                    .then_some((terminal, terminal_route))
            })
            .collect::<Vec<_>>();
        candidates.sort_by_key(|(terminal, _)| {
            (
                terminal.root_ticks - source_node.root_ticks,
                terminal.root_ticks,
                terminal.id,
            )
        });
        let Some((terminal, terminal_route)) = candidates.into_iter().next() else {
            return Ok(None);
        };
        let ticks_to_terminal = terminal.root_ticks - source_node.root_ticks;
        let continuation_frames =
            terminal_route
                .frames
                .get(source_frames..)
                .ok_or(StateGraphError::Invariant(
                    "terminal continuation route precedes its source",
                ))?;
        if continuation_frames.len() as u64 != ticks_to_terminal {
            return Err(StateGraphError::Invariant(
                "terminal continuation frame and tick lengths differ",
            ));
        }
        let mut tape = InputTape {
            tick_rate_numerator: terminal_route.tick_rate_numerator,
            tick_rate_denominator: terminal_route.tick_rate_denominator,
            ..InputTape::default()
        };
        tape.frames.extend_from_slice(continuation_frames);
        tape.validate()?;
        Ok(Some(ExactTerminalContinuation {
            source,
            terminal: terminal.id,
            terminal_route_checkpoint_sha256: terminal.id.route_checkpoint_sha256,
            source_prefix_ticks: source_node.root_ticks,
            ticks_to_terminal,
            tape,
        }))
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
