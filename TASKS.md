# Route learning

## Goal

Build the smallest generic learner that starts when Link gains control, chooses
actions until the native Ordon Springs load-zone predicate fires, and minimizes
native elapsed ticks.

The scored campaign may use generic observations and a fixed route-agnostic
action library. It may not use a human demonstration, inherited route, authored
waypoints, route-specific reward, proxy terminal, or an Ordon-derived tactic.

The known human route is 125 ticks. The targets are 124, then 123, then 120 or
less. A result counts only when its selected controller tape reaches the real
terminal and cold-replays twice from the named root with identical terminal
identity and tick count.

The retained 190-tick route proves execution and replay, not learning quality.
It is not an input to the scored campaign.

## Rules

- Add no mechanism until a measured failure of the simpler system requires it.
- The objective is terminal success with minimum ticks. Motion, velocity,
  camera, rolling, collision, and history are observations, not authored reward
  terms.
- Run bounded experiments and reject failed approaches. Do not accumulate
  treatment versions.
- Use at most two native or build workers on this machine and own every child
  process directly.
- Keep the workspace clean and commit/push completed milestones.

## Work queue

### 1. Run one plain scratch learner

- [x] Add a thin scratch mode that reuses the existing native worker, terminal
  predicate, observation capture, action execution, and exact replay, while
  bypassing demonstrations, inherited routes/policies, graph scheduling,
  frontier critics, calibration, save-state branching, and tactic mining.
- [x] Give it one small generic option set alongside raw controller input:
  move along a chosen heading, roll along a chosen heading, camera-align plus
  movement, and camera-align plus roll. Options are bounded and interruptible;
  heading and duration are parameters.
- [x] Represent state with coarse generic motion cells: position, velocity,
  facing, camera orientation, prompted-action availability, and a short recent
  input/motion history.
- [x] Implement one semi-Markov Q-style loop:
  epsilon-greedy option selection, native tick cost on every transition, the
  authenticated terminal as success, a single small value table/approximator,
  and one backward return pass through completed episodes. Do not add another
  model, policy partition, or shaped reward.
- [x] Start every episode from the authenticated root and allow at least 900
  ticks. Retain all unique transitions and the fastest successful tape. Avoid
  exact duplicate episodes; add no novelty objective yet.
- [x] Make each learner update affect the next eligible action choice. Report
  completed episodes, unique transitions, terminal episodes, fastest selected
  ticks, updates, changed choices, native time, wall time, and time to first
  terminal.
- [x] Automatically cold-replay every strict winner twice.

Implementation checkpoint (2026-08-02): `huntctl learn scratch-route` owns one
native child per cold-root episode, uses a fixed 256-action route-agnostic
catalog and one coarse tabular learner, and persists a checksummed binary
checkpoint. A two-episode Windows smoke resumed that checkpoint, executed
1,800 logical native ticks, retained 200 unique transitions, applied 210
backward-return updates, and changed 141 greedy choices in 81.6 seconds total
wall time. It found no terminal; this is execution evidence, not the
intermediate gate.

- [ ] Intermediate gate: run five fixed ten-minute seeds and reach the real
  load zone from scratch in minutes, not hours. If fewer than three seeds find
  a terminal, stop and diagnose section 2 instead of extending the run.

Exit: the same minimal learner selects and reproduces a zero-shot route of 124
ticks or less.

### 2. Diagnose the first failed gate

Do not implement every branch. Collect enough evidence to select exactly one.

- [ ] Determine whether failure is caused by action expressivity, insufficient
  exploration, incorrect value/return propagation, or insufficient native
  samples. Prove the classification with the retained episode stream and a
  focused deterministic test.
- [ ] Record the diagnosis and the smallest proposed intervention in this file
  before implementing it.

Conditional interventions:

- **Action expressivity:** add only the missing generic option or parameter and
  prove it can improve held-out trajectories.
- **Exploration:** add one coarse state/trajectory novelty rule until the first
  terminal, then disable it for tick optimization.
- **Credit assignment:** correct the Q/return implementation or replace the
  single estimator; do not add parallel critics.
- **Throughput:** measure one versus two workers and root replay versus retained
  save states, then keep only the change that improves unique authenticated
  transitions per second end to end.
- **Local optimum after success:** add one simple successful-trajectory mutation
  loop (delete, shorten, change heading, or change roll timing) while keeping
  terminal success as a hard constraint.

Exit: the selected intervention makes the failed gate pass under the same fixed
budget. Remove it if it does not.

### 3. Establish the benchmark result

- [ ] Re-run five fixed zero-shot seeds with the final minimal mechanism set and
  retain the full result distribution, not only the luckiest seed.
- [ ] Reproduce 124 ticks or less, then continue the unchanged generic process
  to 123 ticks and 120 ticks or less.
- [ ] Verify that ordinary seed ordering and update cadence do not erase the
  useful behavior.

Exit: a zero-shot route reaches 120 ticks or less and cold-replays twice with
identical evidence.

### 4. Prove learning value only after the route works

- [ ] Compare adaptive, frozen, and random-valid selection over the same seeds,
  action opportunities, and native budget.
- [ ] Require adaptive updates to improve terminals per sample, time to first
  terminal, or selected terminal ticks on held-out seeds.
- [ ] Run a separate human-demonstration ablation only after the zero-shot result
  exists. Human input may improve sample efficiency but cannot be required or
  cap the policy.

Exit: accumulated experience causally improves future terminal outcomes over
both controls.

### 5. Generalize only after Ordon passes

- [ ] Apply the unchanged contracts to a second native route.
- [ ] Add tactic mining and composition only if retained experience shows a
  repeated useful control structure; promote a tactic only on held-out gain.
- [ ] Split remaining mixed-responsibility code along execution, evidence,
  learning, persistence, and reporting boundaries; enforce source-size gates.
- [ ] Keep persistent artifacts binary, bounded, checksummed, atomic, and
  migration-tested.

Exit: the second route passes scratch discovery, learned improvement, and exact
cold replay without route-specific framework changes.
