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

- [x] Intermediate gate: run five fixed ten-minute seeds and reach the real
  load zone from scratch in minutes, not hours. If fewer than three seeds find
  a terminal, stop and diagnose section 2 instead of extending the run.

Gate result (2026-08-02): failed at 2/5 seeds. Each seed ran 36 cold-root,
900-tick episodes. Seeds 130363 and 181081 produced two authenticated terminal
episodes each, improving 624 to 481 ticks and 876 to 561 ticks respectively;
seeds 104729, 155921, and 208609 produced none. Across the gate the learner
retained 11,815 unique transitions in 2,601 seconds wall time. This proves
occasional scratch discovery, but not the required reliability or route
quality.

Exit: the same minimal learner selects and reproduces a zero-shot route of 124
ticks or less.

### 2. Diagnose the first failed gate

Do not implement every branch. Collect enough evidence to select exactly one.

- [x] Determine whether failure is caused by action expressivity, insufficient
  exploration, incorrect value/return propagation, or insufficient native
  samples. Prove the classification with the retained episode stream and a
  focused deterministic test.
- [x] Record the diagnosis and the smallest proposed intervention in this file
  before implementing it.

Diagnosis (2026-08-02): **insufficient exploration before the first terminal**.
The unchanged action catalog reached and cold-replayed the real load zone four
times, so basic expressivity is present. In both successful seeds the later
terminal was substantially faster than the first and thousands of deployed
updates changed greedy choices, so backward credit is operating. Native work
consumed 2,407 of 2,601 wall seconds and still yielded thousands of unique
transitions per seed, so orchestration overhead is not the first failure.
However, an equal-horizon censored episode gives every root action the same
-900 return regardless of where its trajectory went; a deterministic learner
test now fixes that fact. Until a sparse terminal is found, ordinary Q value
therefore contains no directional discovery signal and three seeds never
escaped that condition.

Smallest proposed intervention: until the first authenticated terminal only,
train the existing Q table on a count-based novelty return over coarse position
cells. Infrequently visited cells provide the only exploration value and native
ticks remain the only cost. Persist the cell counts. On the first terminal,
clear the novelty-trained values, seed the same table from that successful
episode using the ordinary terminal/tick return, and permanently disable
novelty. This adds no route coordinate, waypoint, collision heuristic, second
model, or retained trajectory input. Merely choosing the least-visited action
in the current full state is not an intervention: the plain learner already
prefers its unvisited zero-valued actions over negatively valued failures.

Intervention result (2026-08-02): **passed, 5/5 successful seeds** under the
same ten-minute wall budget. The v3 binary checkpoint persists coarse cell counts;
the first authenticated terminal clears the temporary values and permanently
returns selection to the terminal/tick learner.

| Seed | Control terminals | Treatment terminals | Treatment best | First terminal |
| ---: | ---: | ---: | ---: | ---: |
| 104729 | 0 | 3 | 540 | 76.7 s |
| 130363 | 2 | 2 | 481 | 266.9 s |
| 155921 | 0 | 3 | 368 | 516.0 s |
| 181081 | 2 | 2 | 561 | 35.5 s |
| 208609 | 0 | 8 | 488 | 50.2 s |

The treatment produced 18 terminal episodes versus 4 in control across 208
cold-root episodes and 13,671 unique transitions. Its median per-seed best was
488 ticks and its best was 368. Eleven strict winners each cold-replayed twice.
Seed 181081 reached the terminal on episode zero with the
same action-sequence digest as control, retained zero novelty cells, and then
reproduced the same 561-tick best; this proves the exploration rule does not
leak past immediate discovery. Retain the intervention.

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

- [x] Add one deterministic post-success mutation loop over the fastest
  successful action sequence: delete one option, cold-root evaluate the
  candidate, and accept it only when the real terminal still fires at a strict
  lower tick. Enumerate each deletion once before repeating any; failed
  mutations cannot alter the incumbent or Q table. Do not add another edit
  family until deletion is measured.
- [x] Prove with a focused deterministic test that deletion candidates are
  complete, non-duplicated, resumable, and terminal failures are rejected.
- [x] Re-run five fixed zero-shot seeds with deletion enabled and retain the
  full result distribution, not only the luckiest seed.
- [x] Correct the measured scheduler failure without adding an edit family:
  after the first terminal, alternate one ordinary Q episode with one pending
  deletion candidate. A new strict Q winner resets deletion enumeration to the
  better incumbent. Failed deletion candidates still make zero Q updates.
- [ ] Re-run the same fixed five seeds with alternation. Retain it only if it
  improves on the 360-tick best without the broad per-seed regression caused by
  exclusive deletion.
- [ ] Reproduce 124 ticks or less, then continue the unchanged generic process
  to 123 ticks and 120 ticks or less.
- [ ] Verify that ordinary seed ordering and update cadence do not erase the
  useful behavior.

Exit: a zero-shot route reaches 120 ticks or less and cold-replays twice with
identical evidence.

Deletion checkpoint (2026-08-02): the v4 learner persists a deterministic set
of unique single-option deletions for the current fastest authenticated action
sequence. Each candidate runs from the cold root; failed or non-improving
candidates make zero Q updates and cannot replace the incumbent. A four-episode
seed-181081 native smoke handed off from learning to three deletion attempts:
the first removed option 9, reached the real terminal, cold-replayed twice, and
improved 876 to 868 ticks; the next two failed candidates made zero learner
updates. Focused tests cover complete deterministic enumeration, duplicate-free
resume, failure rejection, and incumbent reset after acceptance.

Exclusive-deletion result (2026-08-02): measured and rejected as a scheduler,
not as an operator. Across the five fixed ten-minute seeds it attempted 159
deletions, 33 still reached the terminal, and 30 were strict cold-replayed
winners. Per-seed best ticks were 567, 589, 360, 852, and 391, compared with
540, 481, 368, 561, and 488 for continued Q learning. Deletion established a
new overall best by 8 ticks and helped two seeds, but regressed three seeds and
worsened the median from 488 to 567 because it monopolized every post-terminal
episode. The smallest correction is deterministic Q/deletion alternation; do
not add shortening, heading, roll-timing, another model, or more wall time yet.

Alternation checkpoint (2026-08-02): cadence is derived from the last persisted
episode mode, so checkpoint resume needs no mutable scheduler field. A fresh
four-episode seed-181081 native smoke ran learning, deletion, learning,
deletion. The accepted deletion improved 876 to 868 ticks and cold-replayed;
the intervening learning episode made 112 Q updates, while the failed final
deletion made zero. The full learning-framework audit passed 431 orchestration
tests before the native smoke.

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
