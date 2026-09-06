//! Exact costs on recorded edges, independent of action chunking or fit count.

use crate::artifact::Digest;
use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};

pub(super) struct ReverseCostGraph {
    incoming: BTreeMap<Digest, Vec<(Digest, u32)>>,
}

impl ReverseCostGraph {
    pub(super) fn new(edges: impl IntoIterator<Item = (Digest, Digest, u32)>) -> Self {
        let mut incoming = BTreeMap::<_, Vec<_>>::new();
        for (before, after, ticks) in edges {
            incoming.entry(after).or_default().push((before, ticks));
        }
        Self { incoming }
    }

    // Nonnegative native durations allow a reverse Dijkstra traversal. Memory
    // and work are bounded by the recorded graph, not a guessed episode depth.
    pub(super) fn costs(
        &self,
        seeds: impl IntoIterator<Item = (Digest, u64)>,
    ) -> BTreeMap<Digest, u64> {
        let mut costs = BTreeMap::new();
        let mut queue = BinaryHeap::new();
        for (state, ticks) in seeds {
            if costs.get(&state).is_none_or(|prior| ticks < *prior) {
                costs.insert(state, ticks);
                queue.push(Reverse((ticks, state)));
            }
        }
        while let Some(Reverse((ticks, state))) = queue.pop() {
            if costs.get(&state) != Some(&ticks) {
                continue;
            }
            for &(before, duration) in self.incoming.get(&state).into_iter().flatten() {
                let next = ticks.saturating_add(u64::from(duration));
                if costs.get(&before).is_none_or(|prior| next < *prior) {
                    costs.insert(before, next);
                    queue.push(Reverse((next, before)));
                }
            }
        }
        costs
    }
}

/// Costs conditioned on executing each recorded action, stopping at its first
/// terminal boundary. Open components are censored (`None`), not failures.
pub(super) fn terminal_edge_costs(edges: &[(Digest, Digest, u32, bool)]) -> Vec<Option<u64>> {
    let graph = ReverseCostGraph::new(
        edges
            .iter()
            .filter(|edge| !edge.3)
            .map(|&(before, after, ticks, _)| (before, after, ticks)),
    );
    let costs = graph.costs(
        edges
            .iter()
            .filter(|edge| edge.3)
            .map(|&(before, _, ticks, _)| (before, u64::from(ticks))),
    );
    edges
        .iter()
        .map(|&(_, after, ticks, terminal)| {
            if terminal {
                Some(u64::from(ticks))
            } else {
                costs
                    .get(&after)
                    .map(|next| next.saturating_add(u64::from(ticks)))
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(index: u64) -> Digest {
        let mut bytes = [0; 32];
        bytes[..8].copy_from_slice(&index.to_le_bytes());
        Digest(bytes)
    }

    #[test]
    fn long_paths_are_independent_of_order_and_action_chunking() {
        let mut edges = (0..2_048)
            .map(|i| (state(i), state(i + 1), 1, i == 2_047))
            .collect::<Vec<_>>();
        let expected = (1..=2_048).rev().map(Some).collect::<Vec<_>>();
        assert_eq!(terminal_edge_costs(&edges), expected);
        let achieved = ReverseCostGraph::new(edges.iter().map(|&(a, b, ticks, _)| (a, b, ticks)))
            .costs([(state(2_048), 0)]);
        assert_eq!(achieved[&state(0)], 2_048);
        edges.reverse();
        assert_eq!(
            terminal_edge_costs(&edges),
            expected.into_iter().rev().collect::<Vec<_>>()
        );
        assert_eq!(
            terminal_edge_costs(&[(state(0), state(2_048), 2_048, true)]),
            vec![Some(2_048)]
        );
    }

    #[test]
    fn branches_cycles_censoring_and_terminal_boundaries_keep_exact_costs() {
        let edges = [
            (state(1), state(2), 4, false),
            (state(2), state(3), 8, true),
            (state(1), state(4), 40, false),
            (state(4), state(3), 40, true),
            (state(2), state(1), 1, false),
            (state(5), state(6), 1, false),
            (state(6), state(5), 1, false),
            // Reusing a terminal after-state must not extend an earlier hit.
            (state(3), state(7), 100, true),
            (state(8), state(9), 1, false),
        ];
        assert_eq!(
            terminal_edge_costs(&edges),
            vec![
                Some(12),
                Some(8),
                Some(80),
                Some(40),
                Some(13),
                None,
                None,
                Some(100),
                None
            ]
        );
    }

    #[test]
    fn weighted_graph_matches_exhaustive_relaxation() {
        // Independent reference on varied graphs, including competing terminal
        // edges, duplicate states, cycles, and disconnected components.
        for seed in 0..32_u64 {
            let mut random = seed + 1;
            let mut draw = || {
                random = random
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1);
                random >> 32
            };
            let edges = (0..64)
                .map(|_| {
                    (
                        state(draw() % 24),
                        state(draw() % 24),
                        (draw() % 40 + 1) as u32,
                        draw() % 11 == 0,
                    )
                })
                .collect::<Vec<_>>();
            let mut expected = edges
                .iter()
                .map(|edge| edge.3.then_some(u64::from(edge.2)))
                .collect::<Vec<_>>();
            for _ in 0..edges.len() {
                let prior = expected.clone();
                for (index, edge) in edges.iter().enumerate().filter(|(_, edge)| !edge.3) {
                    expected[index] = edges
                        .iter()
                        .enumerate()
                        .filter(|(_, next)| edge.1 == next.0)
                        .filter_map(|(next, _)| prior[next])
                        .min()
                        .map(|next| next + u64::from(edge.2));
                }
                if prior == expected {
                    break;
                }
            }
            assert_eq!(terminal_edge_costs(&edges), expected);
        }
    }
}
