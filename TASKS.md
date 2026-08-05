# Usable route learning

## Goal

Build a generic learner that discovers substantially better-than-human routes
and creative solutions from native game experience. The algorithm is not
prescribed: use Q-style learning, planning, search, imitation, or a hybrid that
fits the available samples and machine throughput.

The learner must:

- discover complete routes without authored waypoints or route-specific
  shaping rewards;
- learn from every attempted branch, not only terminal runs;
- use binary save states to branch from useful intermediate states;
- choose among primitive inputs, simultaneous inputs, variable-duration
  actions, and learned multi-action tactics;
- discover and promote reusable tactics when evidence says they help;
- optionally learn from human demonstrations without requiring them or being
  capped by them;
- produce cold-replayable routes whose success is the real native terminal
  predicate.

Ordon Springs is the first adequacy test, not the product. It is roughly 120
active ticks from control to the load zone. A 125-tick human route is an
ordinary baseline. Required quality gates are 124 ticks, then 123 ticks as
clear better-than-human evidence, then 120 ticks as the current target.

## Current baseline

The production `tactic-route` path already has authenticated binary save
states, sibling restore batches, persistent state graphs, generic observations,
generic action families, learned frontier selection, graph backups, online
cross-lane model sharing, tactic mining, route reconstruction, and cold replay.
The obsolete cold-root `scratch-route` entry point is retired.

The fixed-work production curve measured about 3.5 useful expansions per
second, but the first growing-corpus campaign sustained only 1.11 expansions
per second. The small curve therefore overestimated ten-minute capacity by more
than 3x. Checkpoint capture was not the dominant scale cost; repeated model
updates, evidence projection, graph scheduling, hashing, ranking, and durable
publication were.

A 64-decision, 128-proposal zero-shot preflight for seed 104729 exercised direct
non-root restores and online fitting but found no terminal. That run proves the
plumbing executes; it is far too small to prove or disprove learnability.

No task is currently blocked on missing design.

## P0 — determine whether the learner actually learns

- [x] Run seed 104729 with no incumbent or demonstration for up to 1,024
  decisions, two proposals per decision, and a hard ten-minute limit, whichever
  is reached first. Use the real `ordon_spring_load_committed` predicate and the
  current production learner unchanged for the first treatment.
- [x] Make that single campaign answer several questions at once. Report time
  to first terminal, terminals, unique authenticated states, useful branches,
  transition count, direct restores and fallbacks, action-family coverage,
  prompted-action availability/selection, frontier revisits, learned ranking
  changes, value calibration, materially distinct trajectory clusters, fastest
  ticks, and cold-replay results.
- [x] If it finds no terminal, diagnose the failure from the retained corpus
  before running more volume. Classify it as one or more of:
  insufficient proposal/action coverage, frontier/search starvation, broken
  value propagation or ranking, inadequate state features, restore/state-graph
  errors, or genuinely insufficient samples. Record evidence for the
  classification; “mine longer” is not a diagnosis.
- [ ] Re-score the same retained experience with adaptive, frozen-policy, and
  random-valid selection where possible. Compare coverage, terminal discovery,
  and ranking decisions without paying for redundant native experience. Add a
  matched native control only for questions that offline replay cannot answer.
- [ ] Fix the diagnosed learning/search defect without adding Ordon coordinates,
  authored waypoint progress, bonuses for straightness/rolling/wall contact, or
  a named route tactic. Preserve trajectory, velocity, collision response,
  action availability, and input history as observations the learner may use.
- [ ] Repeat the bounded diagnostic until at least one zero-shot terminal is
  found and reproduced by two cold replays with identical terminal identity and
  tick count.

Latest diagnostic (2026-08-04): seed 104729 hit the ten-minute learner wall at
349 decisions, 698 admitted proposals, 2,858 authenticated states, 57 coarse
spatial cells, 89 model revisions, and no terminal. Every major generic family
was available and selected, including roll, camera lock, combined lock/roll,
curves, relative headings, seek-target, prompted actions, and neutral. Native
branching completed 327 direct non-root restores with zero fallback replays.

The failure was search starvation. The closest retained state appeared at
decision 54 and was never improved during the remaining 294 decisions. In a
single-seed plan, root refresh exactly replaced every rank-zero learned frontier
slot; all other frontier choices ordered zero-expansion states ahead of learned
value. Because each expansion produced several fresh states, a promising state
could never be revisited. The fix now separates learned exploitation, broad
exploration, and root refresh, and makes learned reachability value precede
coverage count inside the exploitation partition. Its 454 orchestration tests
pass; the next bounded native treatment must verify the behavior.

Exit: a bounded production campaign either learns a cold-replayable route or
produces enough evidence to name and fix the specific learning subsystem that
failed. Raw throughput is not allowed to conceal a search or learning failure.

## P0 — establish reliability and route quality

- [ ] Run five fixed zero-shot seeds under the same ten-minute envelope. All
  five must reach the native load zone; report distributions rather than only
  the best seed.
- [ ] Require median time to first terminal to be measured in minutes, not
  hours, and retain every campaign’s graph and transition corpus.
- [ ] Confirm from graph evidence that the learner explores materially different
  approaches and learns the around-corner navigation problem instead of
  inheriting or endlessly polishing one lineage.
- [ ] Discover and cold-replay a zero-shot route of 124 ticks or less.
- [ ] With the same generic framework, reach 123 ticks or less and then 120
  ticks or less. Beating 125 is evidence of success; approaching it is not.
- [ ] Repeat with permuted seed order and equivalent budgets to rule out lucky
  initialization, scheduler ordering, and update-stream leakage.

Exit: the framework reliably discovers substantially better-than-human Ordon
routes on the local machine.

## P0 — prove learning and tactic discovery cause the result

- [ ] Compare adaptive learning against frozen-policy and random-valid controls
  over identical seeds, opportunities, and native budgets. Require a held-out
  gain in terminal rate, time to first terminal, or route ticks.
- [ ] Audit selected actions around decisive states. Show that observations such
  as maintained velocity, momentum loss, trajectory continuity, collision
  response, roll availability, and camera/input history change learned choices
  through experience rather than hard-coded reward bonuses.
- [ ] Demonstrate discovery and promotion of at least one useful multi-action
  tactic. Promotion must improve value or sample efficiency across multiple
  compatible states compared with its primitive components, and primitives
  must remain selectable.
- [ ] Demonstrate that the learner can cross a local optimum where several
  temporarily worse actions are required before reaching the load zone.
- [ ] Run an ordinary suboptimal human-demonstration ablation. Measure whether
  it improves sample efficiency while still allowing the learner to exceed the
  human route and discover tactics absent from the demonstration.

Exit: learned choices and reusable tactics, not lucky unguided search or a
hand-authored route, causally improve results.

## P0 — remove the measured growing-corpus costs

The growing-corpus campaign has now shown a real scale limiter, but checkpoint
capture is not it. Do not start another checkpoint-representation treatment
without new evidence.

- [ ] Complete the capacity envelope with production measurements for cold-root
  replay, save, restore, short branch, worker handoff, unique transitions,
  retained-node bytes, and complete cold validations. Reuse campaign telemetry
  instead of launching one experiment per metric.
- [ ] Use the retained ten-minute telemetry to remove repeated whole-corpus work
  from model fitting, evidence projection, graph projection/hash/ranking, and
  durable replay publication. Preserve exact reports and recovery semantics;
  measure the combined campaign instead of isolated microbenchmarks.
- [ ] Re-run the same bounded treatment and require materially more than 698
  useful expansions without weakening the exploitation/exploration schedule.
- [ ] Re-measure one versus two checkpoint-owning lanes. Keep a second lane only
  when it increases end-to-end unique useful experience without contamination
  or host saturation.

Exit: when volume is demonstrated to be the blocker, the framework explains
where wall time goes and increases useful experience per minute without
weakening correctness.

## P1 — generality and maintainability

- [ ] Apply the unchanged observations, action library, learning rules, tactic
  promotion, and terminal-only objective to a second native route. Route-specific
  coordinates, shaping rewards, and hand-authored tactics remain forbidden.
- [ ] Audit large and mixed-responsibility production files. Split execution,
  save-state ownership, branching, graph learning, proposal generation, tactic
  promotion, persistence, replay proof, and reporting into independently
  testable modules. New work must not grow another orchestration monolith.
- [ ] Keep persistent artifacts bounded, checksummed, atomic, and binary. Retain
  compatibility only when it is cheaper than an explicit replay-and-record
  migration to the current format.

Exit: the learner transfers beyond Ordon and its critical subsystems can be
audited, tested, and changed independently.

## Operating rules

- Every native campaign is bounded and should answer multiple hypotheses.
- Every evaluated branch contributes transitions and outcomes.
- Every promoted result is reproduced by two cold replays.
- Use at most two owned native/build workers on this machine. Manage only exact
  child processes started by this session; never enumerate or kill unrelated
  Codex processes.
- Commit and push each natural milestone. Do not leave a long-lived dirty tree.
