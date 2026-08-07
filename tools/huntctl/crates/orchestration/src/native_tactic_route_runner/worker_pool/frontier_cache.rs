use super::*;
use crate::state_graph::ExactStateId;
use std::collections::{BTreeSet, VecDeque};

/// Rust-side identities for the bounded checkpoint caches owned by persistent
/// native workers.
///
/// The emulator owns the checkpoint bytes. This registry deliberately mirrors
/// only identities and their exact logical graph bindings so scheduling can
/// return to more than the last endpoint without pretending a root replay was
/// a save-state branch.
#[derive(Debug)]
pub(in crate::native_tactic_route_runner) struct RetainedNativeTacticFrontiers {
    entries: VecDeque<CachedTacticFrontier>,
    portable_entries_per_worker: usize,
    pending_locality_reuse: BTreeSet<(usize, String)>,
}

impl RetainedNativeTacticFrontiers {
    pub(in crate::native_tactic_route_runner) fn new(portable_entries_per_worker: usize) -> Self {
        assert!(portable_entries_per_worker > 0);
        Self {
            entries: VecDeque::new(),
            portable_entries_per_worker,
            pending_locality_reuse: BTreeSet::new(),
        }
    }

    pub(in crate::native_tactic_route_runner) fn matching(
        &self,
        state_sha256: Digest,
        route_frames: usize,
        route_checkpoint_sha256: Digest,
        route_tape_sha256: Digest,
    ) -> Option<&CachedTacticFrontier> {
        self.entries.iter().find(|frontier| {
            frontier.state_sha256 == state_sha256
                && frontier.route_frames == route_frames
                && frontier.route_checkpoint_sha256 == route_checkpoint_sha256
                && frontier.route_tape_sha256 == route_tape_sha256
        })
    }

    /// Mirror a successful native cache access in the same least-recently-used
    /// order used when a subsequent portable image is inserted.
    pub(in crate::native_tactic_route_runner) fn touch(
        &mut self,
        worker_slot: usize,
        restore_identity: &str,
    ) {
        let Some(index) = self.entries.iter().position(|frontier| {
            frontier.worker_slot == worker_slot
                && frontier.source.restore_identity == restore_identity
        }) else {
            return;
        };
        let frontier = self
            .entries
            .remove(index)
            .expect("located retained frontier");
        self.entries.push_back(frontier);
    }

    pub(in crate::native_tactic_route_runner) fn remove(
        &mut self,
        worker_slot: usize,
        restore_identity: &str,
    ) {
        self.pending_locality_reuse
            .remove(&(worker_slot, restore_identity.to_owned()));
        self.entries.retain(|frontier| {
            frontier.worker_slot != worker_slot
                || frontier.source.restore_identity != restore_identity
        });
    }

    /// Admit a handle returned by the native worker and mirror the eviction
    /// semantics of its bounded cache. Portable images are LRU-bounded per
    /// worker; a live endpoint, when used by another caller, replaces only the
    /// prior live endpoint on that worker.
    pub(in crate::native_tactic_route_runner) fn retain(&mut self, frontier: CachedTacticFrontier) {
        self.remove(frontier.worker_slot, &frontier.source.restore_identity);
        match frontier.source.storage {
            NativeTacticCheckpointStorage::PortableImage => {
                while self
                    .entries
                    .iter()
                    .filter(|entry| {
                        entry.worker_slot == frontier.worker_slot
                            && entry.source.storage == NativeTacticCheckpointStorage::PortableImage
                    })
                    .count()
                    >= self.portable_entries_per_worker
                {
                    let index = self
                        .entries
                        .iter()
                        .position(|entry| {
                            entry.worker_slot == frontier.worker_slot
                                && entry.source.storage
                                    == NativeTacticCheckpointStorage::PortableImage
                        })
                        .expect("bounded portable frontier exists");
                    let evicted = self
                        .entries
                        .remove(index)
                        .expect("located portable frontier");
                    self.pending_locality_reuse
                        .remove(&(evicted.worker_slot, evicted.source.restore_identity));
                }
            }
            NativeTacticCheckpointStorage::LiveEndpoint => {
                self.entries.retain(|entry| {
                    entry.worker_slot != frontier.worker_slot
                        || entry.source.storage != NativeTacticCheckpointStorage::LiveEndpoint
                });
            }
        }
        if frontier.source.storage == NativeTacticCheckpointStorage::PortableImage {
            self.pending_locality_reuse.insert((
                frontier.worker_slot,
                frontier.source.restore_identity.clone(),
            ));
        }
        self.entries.push_back(frontier);
    }

    /// A newly materialized portable branch base receives one bounded chance
    /// to amortize its replay with another rollout from the same exact state.
    /// The learning controller still rejects exhausted or ineligible states.
    pub(in crate::native_tactic_route_runner) fn pending_locality_targets(
        &self,
    ) -> Vec<ExactStateId> {
        self.entries
            .iter()
            // Give the source materialized by the immediately preceding
            // branch its bounded reuse before older pending entries. Oldest-
            // first ordering routinely evicts that source before it is ever
            // selected again.
            .rev()
            .filter(|frontier| {
                frontier.source.storage == NativeTacticCheckpointStorage::PortableImage
                    && self.pending_locality_reuse.contains(&(
                        frontier.worker_slot,
                        frontier.source.restore_identity.clone(),
                    ))
            })
            .map(|frontier| ExactStateId {
                route_checkpoint_sha256: frontier.route_checkpoint_sha256,
                state_sha256: frontier.state_sha256,
            })
            .collect()
    }

    pub(in crate::native_tactic_route_runner) fn consume_locality_reuse(
        &mut self,
        worker_slot: usize,
        restore_identity: &str,
    ) {
        self.pending_locality_reuse
            .remove(&(worker_slot, restore_identity.to_owned()));
    }

    #[cfg(test)]
    pub(super) fn identities(&self) -> Vec<&str> {
        self.entries
            .iter()
            .map(|entry| entry.source.restore_identity.as_str())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(byte: u8) -> Digest {
        Digest([byte; 32])
    }

    fn frontier(worker_slot: usize, identity: &str, byte: u8) -> CachedTacticFrontier {
        CachedTacticFrontier {
            worker_slot,
            source: NativeTacticCheckpointSource {
                restore_identity: identity.into(),
                boundary_fingerprint: format!("boundary-{identity}"),
                route_ticks: usize::from(byte),
                storage: NativeTacticCheckpointStorage::PortableImage,
            },
            state_sha256: digest(byte),
            route_frames: usize::from(byte),
            route_checkpoint_sha256: digest(byte.wrapping_add(1)),
            route_tape_sha256: digest(byte.wrapping_add(2)),
        }
    }

    #[test]
    fn retains_multiple_native_frontiers_and_evicts_per_worker() {
        let mut retained = RetainedNativeTacticFrontiers::new(2);
        retained.retain(frontier(0, "a", 1));
        retained.retain(frontier(0, "b", 2));
        retained.retain(frontier(1, "other-worker", 3));
        assert_eq!(retained.identities(), vec!["a", "b", "other-worker"]);

        retained.retain(frontier(0, "c", 4));
        assert_eq!(retained.identities(), vec!["b", "other-worker", "c"]);
    }

    #[test]
    fn native_restore_touch_protects_a_frontier_from_next_eviction() {
        let mut retained = RetainedNativeTacticFrontiers::new(2);
        retained.retain(frontier(0, "a", 1));
        retained.retain(frontier(0, "b", 2));
        retained.touch(0, "a");
        retained.retain(frontier(0, "c", 3));
        assert_eq!(retained.identities(), vec!["a", "c"]);
    }

    #[test]
    fn live_rollout_progress_does_not_evict_its_portable_branch_base() {
        let mut retained = RetainedNativeTacticFrontiers::new(2);
        retained.retain(frontier(0, "branch-base", 1));
        let mut first_live = frontier(0, "live-one", 2);
        first_live.source.storage = NativeTacticCheckpointStorage::LiveEndpoint;
        retained.retain(first_live);
        let mut second_live = frontier(0, "live-two", 3);
        second_live.source.storage = NativeTacticCheckpointStorage::LiveEndpoint;
        retained.retain(second_live);
        assert_eq!(retained.identities(), vec!["branch-base", "live-two"]);
        assert_eq!(
            retained.pending_locality_targets(),
            vec![ExactStateId {
                route_checkpoint_sha256: digest(2),
                state_sha256: digest(1),
            }]
        );
    }

    #[test]
    fn portable_locality_preference_is_consumed_once_without_eviction() {
        let mut retained = RetainedNativeTacticFrontiers::new(2);
        retained.retain(frontier(0, "branch-base", 1));
        assert_eq!(retained.pending_locality_targets().len(), 1);

        retained.consume_locality_reuse(0, "branch-base");

        assert!(retained.pending_locality_targets().is_empty());
        assert_eq!(retained.identities(), vec!["branch-base"]);
    }

    #[test]
    fn locality_prefers_the_most_recent_materialized_source() {
        let mut retained = RetainedNativeTacticFrontiers::new(2);
        retained.retain(frontier(0, "older", 1));
        retained.retain(frontier(0, "newer", 2));

        assert_eq!(
            retained.pending_locality_targets(),
            vec![
                ExactStateId {
                    route_checkpoint_sha256: digest(3),
                    state_sha256: digest(2),
                },
                ExactStateId {
                    route_checkpoint_sha256: digest(2),
                    state_sha256: digest(1),
                },
            ]
        );
    }

    #[test]
    fn matching_requires_the_complete_logical_restore_binding() {
        let mut retained = RetainedNativeTacticFrontiers::new(2);
        let expected = frontier(0, "a", 7);
        retained.retain(expected.clone());
        assert_eq!(
            retained
                .matching(
                    expected.state_sha256,
                    expected.route_frames,
                    expected.route_checkpoint_sha256,
                    expected.route_tape_sha256,
                )
                .map(|entry| entry.source.restore_identity.as_str()),
            Some("a")
        );
        assert!(
            retained
                .matching(
                    expected.state_sha256,
                    expected.route_frames + 1,
                    expected.route_checkpoint_sha256,
                    expected.route_tape_sha256,
                )
                .is_none()
        );
    }
}
