//! Small deterministic search fixtures for proving planner invariants.
//!
//! These fixtures deliberately expose only exact state, applicable actions,
//! realized transitions, terminal truth, and native-tick cost. Search code
//! cannot recover authored waypoints or a shaped reward from this interface.

use dusklight_automation_contracts::artifact::Digest;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

const AROUND_CORNER_FIXTURE_SCHEMA_V1: &[u8] = b"dusklight-around-corner-fixture/v1";

/// A coordinate is the complete future-affecting state of this fixture.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureState {
    pub x: u8,
    pub y: u8,
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
    origin: FixtureState,
    start: FixtureState,
    goal: FixtureState,
    blocked: Vec<FixtureState>,
}

impl Default for AroundCornerFixture {
    fn default() -> Self {
        Self {
            width: 4,
            height: 4,
            origin: FixtureState { x: 0, y: 0 },
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

    /// Produce an isomorphic arena at a different absolute location. Exact
    /// state identities therefore remain disjoint while relative typed
    /// observations support held-out generalization.
    pub fn seeded(seed: u64) -> Self {
        let origin = FixtureState {
            x: (seed as u8) & 0x0f,
            y: ((seed >> 4) as u8) & 0x0f,
        };
        let offset = |x: u8, y: u8| FixtureState {
            x: origin.x + x,
            y: origin.y + y,
        };
        Self {
            width: origin.x + 4,
            height: origin.y + 4,
            origin,
            start: offset(0, 0),
            goal: offset(3, 0),
            blocked: vec![offset(1, 0), offset(1, 1), offset(1, 2)],
        }
    }

    pub fn start(&self) -> FixtureState {
        self.start
    }

    pub fn state_sha256(&self, state: FixtureState) -> Digest {
        let mut hasher = Sha256::new();
        hasher.update(AROUND_CORNER_FIXTURE_SCHEMA_V1);
        hasher.update([
            self.width,
            self.height,
            self.origin.x,
            self.origin.y,
            self.start.x,
            self.start.y,
            self.goal.x,
            self.goal.y,
        ]);
        for blocked in &self.blocked {
            hasher.update([blocked.x, blocked.y]);
        }
        hasher.update([state.x, state.y]);
        Digest(hasher.finalize().into())
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
        (x >= self.origin.x
            && y >= self.origin.y
            && x < self.width
            && y < self.height
            && !self.blocked.contains(&target))
        .then_some(target)
    }

    fn learning_state(&self, state: FixtureState) -> FixtureLearningState {
        FixtureLearningState {
            relative_x: state.x - self.origin.x,
            relative_y: state.y - self.origin.y,
            goal_relative_x: i16::from(self.goal.x) - i16::from(state.x),
            goal_relative_y: i16::from(self.goal.y) - i16::from(state.y),
        }
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
    let mut fastest_ticks = BTreeMap::from([(fixture.state_sha256(fixture.start()), 0_u64)]);
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

            let identity = fixture.state_sha256(transition.target);
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

/// Route-independent learner input. It contains typed state and goal-relative
/// facts, but no wall map, waypoint, or privileged shortest-path label.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureLearningState {
    pub relative_x: u8,
    pub relative_y: u8,
    pub goal_relative_x: i16,
    pub goal_relative_y: i16,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureLearnerSnapshot {
    conditional_ticks_to_terminal: BTreeMap<(FixtureLearningState, FixtureAction), u64>,
}

impl FixtureLearnerSnapshot {
    pub fn conditional_ticks_to_terminal(
        &self,
        fixture: &AroundCornerFixture,
        state: FixtureState,
        action: FixtureAction,
    ) -> Option<u64> {
        self.conditional_ticks_to_terminal
            .get(&(fixture.learning_state(state), action))
            .copied()
    }

    pub fn supported_expansions(&self) -> usize {
        self.conditional_ticks_to_terminal.len()
    }
}

/// Learn exact conditional terminal returns from exhaustive transition
/// evidence. The resulting table is keyed by typed, relative observations
/// rather than exact state identity, so it can rank a held-out translated
/// fixture without sharing its graph.
pub fn learn_fixture_returns(fixture: &AroundCornerFixture) -> FixtureLearnerSnapshot {
    let mut frontier = VecDeque::from([fixture.start()]);
    let mut visited = BTreeSet::from([fixture.state_sha256(fixture.start())]);
    let mut transitions = Vec::new();
    let mut states = vec![fixture.start()];
    while let Some(state) = frontier.pop_front() {
        for action in fixture.applicable_actions(state) {
            let transition = fixture
                .execute(state, action)
                .expect("applicable fixture action must execute");
            transitions.push(transition);
            let identity = fixture.state_sha256(transition.target);
            if visited.insert(identity) {
                states.push(transition.target);
                frontier.push_back(transition.target);
            }
        }
    }

    let mut ticks_to_terminal = BTreeMap::from([(fixture.goal, 0_u64)]);
    for _ in 0..states.len() {
        let mut changed = false;
        for transition in &transitions {
            let Some(target_ticks) = ticks_to_terminal.get(&transition.target).copied() else {
                continue;
            };
            let candidate = transition.native_ticks + target_ticks;
            match ticks_to_terminal.get_mut(&transition.source) {
                Some(current) if candidate < *current => {
                    *current = candidate;
                    changed = true;
                }
                None => {
                    ticks_to_terminal.insert(transition.source, candidate);
                    changed = true;
                }
                _ => {}
            }
        }
        if !changed {
            break;
        }
    }

    let conditional_ticks_to_terminal = transitions
        .into_iter()
        .filter_map(|transition| {
            let target_ticks = ticks_to_terminal.get(&transition.target)?;
            Some((
                (fixture.learning_state(transition.source), transition.action),
                transition.native_ticks + *target_ticks,
            ))
        })
        .collect();
    FixtureLearnerSnapshot {
        conditional_ticks_to_terminal,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FixtureSchedulerResult {
    pub actions: Vec<FixtureAction>,
    pub native_ticks: u64,
    pub unique_expansions: u64,
    pub duplicate_transpositions: u64,
    pub learner_supported_expansions: u64,
    pub trace: Vec<FixtureExpansionTrace>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FixtureExpansionTrace {
    pub source_sha256: Digest,
    pub action: FixtureAction,
    /// Filled only when this exact expansion lies on the authenticated
    /// terminal route returned by the search.
    pub exact_terminal_ticks_to_go: Option<u64>,
    /// Generalized learner output used to rank this expansion.
    pub generalized_conditional_ticks_to_go: Option<u64>,
    /// This exact-return learner does not claim an uncertainty estimate.
    pub uncertainty_millionths: Option<u64>,
    /// Complete deterministic queue key apart from the content tie rank.
    pub exploration_priority: u64,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ExpansionPriority {
    predicted_total_ticks: u64,
    unsupported: bool,
    tie_rank: [u8; 32],
    source: FixtureState,
    action: FixtureAction,
}

#[derive(Clone, Debug)]
struct PendingExpansion {
    source_ticks: u64,
    route: Vec<FixtureAction>,
}

/// Deterministic node/action scheduling with exact-state transpositions.
///
/// Every applicable expansion remains pending until executed or made stale by
/// a faster route to its source. Learned estimates only affect ordering.
pub fn schedule_around_corner(
    fixture: &AroundCornerFixture,
    learner: Option<&FixtureLearnerSnapshot>,
    seed: u64,
    maximum_expansions: u64,
) -> Option<FixtureSchedulerResult> {
    let mut pending = BTreeMap::new();
    let mut best_ticks = BTreeMap::from([(fixture.state_sha256(fixture.start()), 0_u64)]);
    let mut expanded = BTreeSet::new();
    register_fixture_expansions(
        fixture,
        learner,
        seed,
        fixture.start(),
        0,
        Vec::new(),
        &mut pending,
    );
    let mut unique_expansions = 0_u64;
    let mut duplicate_transpositions = 0_u64;
    let mut learner_supported_expansions = 0_u64;
    let mut trace = Vec::new();

    while let Some((priority, candidate)) = pending.pop_first() {
        let source_identity = fixture.state_sha256(priority.source);
        if best_ticks.get(&source_identity) != Some(&candidate.source_ticks)
            || !expanded.insert((source_identity, priority.action))
        {
            continue;
        }
        if unique_expansions >= maximum_expansions {
            return None;
        }
        unique_expansions += 1;
        learner_supported_expansions += u64::from(!priority.unsupported);
        trace.push(FixtureExpansionTrace {
            source_sha256: source_identity,
            action: priority.action,
            exact_terminal_ticks_to_go: None,
            generalized_conditional_ticks_to_go: (!priority.unsupported)
                .then_some(priority.predicted_total_ticks - candidate.source_ticks),
            uncertainty_millionths: None,
            exploration_priority: priority.predicted_total_ticks,
        });
        let transition = fixture.execute(priority.source, priority.action)?;
        let target_ticks = candidate.source_ticks + transition.native_ticks;
        let mut route = candidate.route;
        route.push(priority.action);
        if transition.terminal {
            publish_exact_fixture_returns(fixture, &route, &mut trace);
            return Some(FixtureSchedulerResult {
                actions: route,
                native_ticks: target_ticks,
                unique_expansions,
                duplicate_transpositions,
                learner_supported_expansions,
                trace,
            });
        }

        let target_identity = fixture.state_sha256(transition.target);
        if best_ticks
            .get(&target_identity)
            .is_some_and(|known| *known <= target_ticks)
        {
            duplicate_transpositions += 1;
            continue;
        }
        best_ticks.insert(target_identity, target_ticks);
        register_fixture_expansions(
            fixture,
            learner,
            seed,
            transition.target,
            target_ticks,
            route,
            &mut pending,
        );
    }
    None
}

fn publish_exact_fixture_returns(
    fixture: &AroundCornerFixture,
    route: &[FixtureAction],
    trace: &mut [FixtureExpansionTrace],
) {
    let mut state = fixture.start();
    for (index, action) in route.iter().enumerate() {
        let identity = fixture.state_sha256(state);
        let ticks_to_go = (route.len() - index) as u64;
        if let Some(row) = trace
            .iter_mut()
            .find(|row| row.source_sha256 == identity && row.action == *action)
        {
            row.exact_terminal_ticks_to_go = Some(ticks_to_go);
        }
        state = fixture
            .execute(state, *action)
            .expect("returned fixture route must remain executable")
            .target;
    }
}

#[allow(clippy::too_many_arguments)]
fn register_fixture_expansions(
    fixture: &AroundCornerFixture,
    learner: Option<&FixtureLearnerSnapshot>,
    seed: u64,
    source: FixtureState,
    source_ticks: u64,
    route: Vec<FixtureAction>,
    pending: &mut BTreeMap<ExpansionPriority, PendingExpansion>,
) {
    for action in fixture.applicable_actions(source) {
        let learned_ticks = learner
            .and_then(|snapshot| snapshot.conditional_ticks_to_terminal(fixture, source, action));
        let priority = ExpansionPriority {
            predicted_total_ticks: source_ticks + learned_ticks.unwrap_or(1),
            unsupported: learned_ticks.is_none(),
            tie_rank: fixture_tie_rank(seed, fixture.state_sha256(source), action),
            source,
            action,
        };
        pending.insert(
            priority.clone(),
            PendingExpansion {
                source_ticks,
                route: route.clone(),
            },
        );
    }
}

fn fixture_tie_rank(seed: u64, state: Digest, action: FixtureAction) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight-fixture-scheduler-tie/v1");
    hasher.update(seed.to_le_bytes());
    hasher.update(state.0);
    hasher.update([action as u8]);
    hasher.finalize().into()
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
        let fixture = AroundCornerFixture::default();
        let state = FixtureState { x: 0, y: 2 };
        assert_eq!(fixture.state_sha256(state), fixture.state_sha256(state));
        assert_ne!(
            fixture.state_sha256(state),
            fixture.state_sha256(FixtureState { x: 0, y: 3 })
        );
    }

    #[test]
    fn learned_scheduler_beats_uniform_cost_on_held_out_exact_states() {
        let training = AroundCornerFixture::seeded(0);
        let learner = learn_fixture_returns(&training);
        assert!(learner.supported_expansions() > 0);

        for seed in [0x11, 0x22, 0x33, 0x44] {
            let held_out = AroundCornerFixture::seeded(seed);
            assert_ne!(
                training.state_sha256(training.start()),
                held_out.state_sha256(held_out.start())
            );
            let learned = schedule_around_corner(&held_out, Some(&learner), seed, 1_000).unwrap();
            let exhaustive = schedule_around_corner(&held_out, None, 0, 1_000).unwrap();
            let non_learning = schedule_around_corner(&held_out, None, seed, 1_000).unwrap();

            assert_eq!(
                learned.native_ticks,
                AroundCornerFixture::KNOWN_SHORTEST_TICKS
            );
            assert_eq!(learned.actions.len() as u64, learned.native_ticks);
            assert_eq!(
                learned.learner_supported_expansions,
                learned.unique_expansions
            );
            assert_eq!(
                learned
                    .trace
                    .iter()
                    .filter(|row| row.exact_terminal_ticks_to_go.is_some())
                    .count(),
                learned.actions.len()
            );
            assert!(learned.trace.iter().all(|row| {
                row.generalized_conditional_ticks_to_go.is_some()
                    && row.uncertainty_millionths.is_none()
            }));
            assert!(learned.unique_expansions < exhaustive.unique_expansions);
            assert!(learned.unique_expansions < non_learning.unique_expansions);
        }
    }
}
