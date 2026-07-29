//! Small deterministic search fixtures for proving planner invariants.
//!
//! These fixtures deliberately expose only exact state, applicable actions,
//! realized transitions, terminal truth, and native-tick cost. Search code
//! cannot recover authored waypoints or a shaped reward from this interface.

use dusklight_automation_contracts::artifact::Digest;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, VecDeque};

const AROUND_CORNER_FIXTURE_SCHEMA_V1: &[u8] = b"dusklight-around-corner-fixture/v1";

/// A coordinate is the complete future-affecting state of this fixture.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureState {
    pub x: u8,
    pub y: u8,
}

impl FixtureState {
    pub fn content_sha256(self) -> Digest {
        let mut hasher = Sha256::new();
        hasher.update(AROUND_CORNER_FIXTURE_SCHEMA_V1);
        hasher.update([self.x, self.y]);
        Digest(hasher.finalize().into())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FixtureAction {
    North,
    East,
    South,
    West,
}

impl FixtureAction {
    const ALL: [Self; 4] = [Self::North, Self::East, Self::South, Self::West];
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FixtureTransition {
    pub source: FixtureState,
    pub action: FixtureAction,
    pub target: FixtureState,
    pub native_ticks: u64,
    pub terminal: bool,
}

/// A narrow wall separates start and goal. The only route passes around its
/// north end, making a greedy eastward policy fail while retaining a tiny,
/// exactly enumerable state space.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AroundCornerFixture {
    width: u8,
    height: u8,
    start: FixtureState,
    goal: FixtureState,
    blocked: Vec<FixtureState>,
}

impl Default for AroundCornerFixture {
    fn default() -> Self {
        Self {
            width: 4,
            height: 4,
            start: FixtureState { x: 0, y: 0 },
            goal: FixtureState { x: 3, y: 0 },
            blocked: vec![
                FixtureState { x: 1, y: 0 },
                FixtureState { x: 1, y: 1 },
                FixtureState { x: 1, y: 2 },
            ],
        }
    }
}

impl AroundCornerFixture {
    pub const KNOWN_SHORTEST_TICKS: u64 = 9;

    pub fn start(&self) -> FixtureState {
        self.start
    }

    pub fn is_terminal(&self, state: FixtureState) -> bool {
        state == self.goal
    }

    pub fn applicable_actions(&self, state: FixtureState) -> Vec<FixtureAction> {
        FixtureAction::ALL
            .into_iter()
            .filter(|action| self.target(state, *action).is_some())
            .collect()
    }

    pub fn execute(
        &self,
        source: FixtureState,
        action: FixtureAction,
    ) -> Option<FixtureTransition> {
        let target = self.target(source, action)?;
        Some(FixtureTransition {
            source,
            action,
            target,
            native_ticks: 1,
            terminal: self.is_terminal(target),
        })
    }

    fn target(&self, state: FixtureState, action: FixtureAction) -> Option<FixtureState> {
        let (x, y) = match action {
            FixtureAction::North => (state.x, state.y.checked_add(1)?),
            FixtureAction::East => (state.x.checked_add(1)?, state.y),
            FixtureAction::South => (state.x, state.y.checked_sub(1)?),
            FixtureAction::West => (state.x.checked_sub(1)?, state.y),
        };
        let target = FixtureState { x, y };
        (x < self.width && y < self.height && !self.blocked.contains(&target)).then_some(target)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FixtureSearchResult {
    pub actions: Vec<FixtureAction>,
    pub native_ticks: u64,
    /// Number of state/action pairs actually expanded.
    pub expansions: u64,
    /// Number of candidates rejected because their exact state was already
    /// reachable at an equal or lower cost.
    pub duplicate_transpositions: u64,
    pub unique_states: usize,
}

#[derive(Clone)]
struct FrontierEntry {
    state: FixtureState,
    route: Vec<FixtureAction>,
}

/// Deterministic breadth-first search. When `merge_transpositions` is enabled,
/// content-identical states retain only their fastest route. Disabling it is a
/// bounded tree-search control over precisely the same environment.
pub fn exhaustive_around_corner(
    fixture: &AroundCornerFixture,
    merge_transpositions: bool,
    maximum_expansions: u64,
) -> Option<FixtureSearchResult> {
    let mut frontier = VecDeque::from([FrontierEntry {
        state: fixture.start(),
        route: Vec::new(),
    }]);
    let mut fastest_ticks = BTreeMap::from([(fixture.start().content_sha256(), 0_u64)]);
    let mut expansions = 0_u64;
    let mut duplicate_transpositions = 0_u64;

    while let Some(entry) = frontier.pop_front() {
        for action in fixture.applicable_actions(entry.state) {
            if expansions >= maximum_expansions {
                return None;
            }
            expansions += 1;
            let transition = fixture.execute(entry.state, action)?;
            let mut route = entry.route.clone();
            route.push(action);
            let ticks = route.len() as u64;
            if transition.terminal {
                return Some(FixtureSearchResult {
                    actions: route,
                    native_ticks: ticks,
                    expansions,
                    duplicate_transpositions,
                    unique_states: fastest_ticks.len() + 1,
                });
            }

            let identity = transition.target.content_sha256();
            let duplicate = fastest_ticks
                .get(&identity)
                .is_some_and(|known_ticks| *known_ticks <= ticks);
            if duplicate && merge_transpositions {
                duplicate_transpositions += 1;
                continue;
            }
            fastest_ticks
                .entry(identity)
                .and_modify(|known_ticks| *known_ticks = (*known_ticks).min(ticks))
                .or_insert(ticks);
            frontier.push_back(FrontierEntry {
                state: transition.target,
                route,
            });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exhaustive_mode_finds_the_known_shortest_route() {
        let fixture = AroundCornerFixture::default();
        let result = exhaustive_around_corner(&fixture, true, 1_000).unwrap();

        assert_eq!(
            result.native_ticks,
            AroundCornerFixture::KNOWN_SHORTEST_TICKS
        );
        assert_eq!(
            result.actions,
            vec![
                FixtureAction::North,
                FixtureAction::North,
                FixtureAction::North,
                FixtureAction::East,
                FixtureAction::East,
                FixtureAction::East,
                FixtureAction::South,
                FixtureAction::South,
                FixtureAction::South,
            ]
        );
        assert!(
            fixture.is_terminal(
                result
                    .actions
                    .iter()
                    .try_fold(fixture.start(), |state, action| {
                        fixture.execute(state, *action).map(|step| step.target)
                    })
                    .unwrap()
            )
        );
    }

    #[test]
    fn exact_transpositions_reduce_duplicate_work_without_changing_the_optimum() {
        let fixture = AroundCornerFixture::default();
        let merged = exhaustive_around_corner(&fixture, true, 10_000).unwrap();
        let tree = exhaustive_around_corner(&fixture, false, 10_000).unwrap();

        assert_eq!(merged.native_ticks, tree.native_ticks);
        assert_eq!(merged.actions, tree.actions);
        assert!(merged.duplicate_transpositions > 0);
        assert!(merged.expansions < tree.expansions);
        assert_eq!(merged.unique_states, tree.unique_states);
    }

    #[test]
    fn state_identity_is_route_independent_but_not_semantic() {
        let state = FixtureState { x: 0, y: 2 };
        assert_eq!(state.content_sha256(), state.content_sha256());
        assert_ne!(
            state.content_sha256(),
            FixtureState { x: 0, y: 3 }.content_sha256()
        );
    }
}
