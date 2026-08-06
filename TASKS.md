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
- [x] Re-score native-terminal action ranking on the retained v6 corpus using
  whole-source-state holdout and exact authenticated ticks-to-terminal. Keep
  an unproven learned action as a measured sibling instead of deploying it.
- [x] Re-score post-terminal graph scheduling against least-visited and seeded
  random-valid order on the retained final graph. Report explicitly when the
  graph lacks comparable exact outcomes instead of substituting immediate
  progress or reward for the terminal objective.
- [ ] Complete the adaptive, frozen-policy, and random-valid causal comparison.
  Reuse retained experience where it supplies identical supported
  opportunities; acquire matched native evidence only for comparisons that
  are censored in the retained graph.
- [x] Fix the diagnosed learning/search defect without adding Ordon coordinates,
  authored waypoint progress, bonuses for straightness/rolling/wall contact, or
  a named route tactic. Preserve trajectory, velocity, collision response,
  action availability, and input history as observations the learner may use.
- [x] Repeat the bounded diagnostic until at least one zero-shot terminal is
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
pass. The first matched treatment then exposed a separate Windows persistence
failure: recovery-directory publication returned access denied at decisions
143 and 255 while leaving valid prior recovery points. Recovery publication now
retries only transient Windows permission-denied renames for a bounded two
seconds and retains fail-closed behavior for every other error. The resumed
native treatment verified both fixes: seed 104729 found the real load-zone
terminal after 34.4 seconds, produced eight terminal proposals and three
selected terminal decisions, and completed cleanly at 702 useful expansions.
Its best authenticated route is 229 ticks, so discovery is now quick but route
quality remains inadequate.

Cold replay then exposed a writer/reader contract bug: the authoritative seed
result was 97.5 MiB of pretty JSON while its reader rejected anything above 64
MiB; the same content is 56.1 MiB compact. New seed results now use compact JSON
and enforce the 64 MiB bound before publication. Readers temporarily accept
existing v45 pretty artifacts up to 128 MiB for replay/migration. Full trace
descriptor deduplication into bounded binary storage remains open below.
The 229-tick route then passed two cold replays with identical first-hit tick,
controller tape, and terminal boundary fingerprint
`0f3f6ab4888746792e01a15f18465d8e`.

The first matched growing-corpus optimization (2026-08-05) stopped reloading,
rehashing, and revalidating every durable transition and tape during each
learner refit. The replay authority now retains its already authenticated
in-memory corpus and supplies the recorded transition identities directly.
This preserved the 229-tick result and 34.5-second terminal discovery, reduced
persistence from 77.5 to 47.2 seconds, but increased useful expansions only
from 702 to 718 (1.117 to 1.144 per second). That 2.3% capacity gain is not
material and does not satisfy the throughput task. Model fitting still consumed
215.8 seconds and graph scheduling 120.8 seconds. Inspection then found that
achieved-goal fitting recomputed the complete route-agnostic observation for
every `(transition, sampled goal)` pair even though only eight goal-relative
columns change. The exact feature path now computes each physical boundary's
base observation once per refit and appends only those goal-relative columns.

That second matched treatment also failed the campaign-level gate: useful
expansions fell to 686 (1.094 per second), model time was still 209.6 seconds,
and the best route remained 229 ticks. Do not spend another campaign on base
feature or replay-materialization micro-optimizations. Across all three matched
runs, the first terminal appears in 34--38 seconds and never improves during
roughly 280 later decisions. Retained trace evidence identifies a quality
search defect: a branch rooted on the authenticated terminal path is abandoned
when the ordinary four-decision branch cadence fires, even when it has executed
fewer native ticks than the incumbent continuation from that exact prefix. An
early-prefix alternative can therefore be interrupted before it has an equal
opportunity to reach the terminal. Post-terminal refinement must preserve each
candidate rollout until it reaches the terminal or consumes the incumbent's
exact remaining-tick budget; only then may ordinary broad/root acquisition
replace it.

The equal-budget treatment (2026-08-05) verified that contract on retained
native trace: 14 terminal-path refinement attempts ran, every nonterminal
attempt consumed at least its exact incumbent continuation budget, two reached
the terminal, and 16 broad post-terminal acquisitions still ran between
completed attempts. The treatment improved a selected 301-tick terminal route
to 254 ticks, but its final best was still worse than the prior treatment's
229-tick route because the longer refinements displaced broad attempts and the
learned continuation failed on 12 of 14 opportunities. Fair refinement
evaluation is now present; the learned refinement policy has not proved it is
worth that budget. Re-score this retained corpus against frozen and
random-valid selection before changing the exploration share or running more
seeds.

The retained causal audits (2026-08-05) show why immediate post-terminal
deployment was invalid. Of 698 transitions, only 45 have exact authenticated
terminal continuations, and only two source states have two terminal-supported
action siblings. The universal action head ranked both correctly, but its 95%
Wilson lower bound is 0.342 against a 0.5 chance rate, so it has no deployment
authority. The learner now withholds complete source states, calibrates against
exact ticks-to-terminal, and leaves the learned action in the evaluated sibling
slot until at least eight comparable groups establish better-than-chance
ranking. The gate and its evidence are bound into learner snapshots and
decision journals; `calibrate-terminal-action-ranking` reproduces the report
from a retained checkpoint without native execution.

The graph-scheduler control found 322 terminal-path interior states but only 71
were ever leased. None of 19 optimization decisions had two exact terminal
outcomes, so the retained graph cannot compare learned, least-visited, and
random-valid scheduling at all. This is outcome-coverage starvation: one-step
sibling evaluation collects observations, but almost every alternative remains
censored because it never receives a continuation to the terminal or its equal
budget.

Exit: a bounded production campaign either learns a cold-replayable route or
produces enough evidence to name and fix the specific learning subsystem that
failed. Raw throughput is not allowed to conceal a search or learning failure.

## P0 — establish reliability and route quality

- [x] Give every terminal-path refinement branch an equal native continuation
  budget: do not preempt it until it reaches the terminal or executes the
  incumbent's exact ticks-to-go from that prefix. Reconstruct this rollout
  authority across recovery, retain ordinary broad exploration between
  completed refinement attempts, and report completed versus interrupted
  refinement attempts.
- [x] Implement paired terminal-return collection from the same authenticated
  save boundary. At a terminal-path decision, choose proposal zero as the
  policy lineage and proposal one as its deterministic control before native
  outcomes exist. Continue both until terminal or the same incumbent
  ticks-to-go; freeze their learner snapshot, preserve both graph lineages,
  recover unfinished pairs exactly, and prevent observed outcomes from changing
  the selected pair.
- [ ] Acquire paired native evidence for the adaptive, frozen-policy, and
  random-valid treatments. Report completed, in-progress, terminal-supported,
  and censored pairs. A supported action comparison requires both lineages to
  hit the native terminal; equal-budget failures remain explicitly censored.
  Acquire at least eight supported matched opportunities per treatment before
  interpreting action-ranking rates, then use confidence intervals rather than
  the floor itself as the success criterion.
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

Paired-return activation proof (2026-08-05): the production runner records a
durable pair identity at the exact source checkpoint, binds it to the original
pre-outcome proposal order, and restores the control's exact graph node after
the policy rollout finishes. Both rollouts keep the same immutable learner
snapshot even while their experience is published globally. Recovery
reconstructs an older frozen snapshot without moving the shared learner head
backward. Decision journals reject detached control targets, option identities,
learner authorities, and phase transitions.

A bounded one-worker adaptive run on seed 104729 reached the real native
terminal after 374.8 seconds, completed 81 decisions and 3,197 native ticks,
and retained a 264-tick best route. It started two exact same-source pairs: one
completed with both lineages terminal-supported, while the second remained
in-progress and therefore censored at the campaign bound. The summary reported
zero learner-authority violations and passed independent report/plan/summary
validation. This proves native activation and recovery/accounting integration;
one supported adaptive pair does not establish an action-ranking rate or a
learned advantage. Frozen-policy and random-valid evidence, and at least eight
supported opportunities per treatment, remain open above.

The activation audit also closed a fail-open reporting hole: a malformed pair
could previously increment the authority-violation count while still entering
the supported total. Invalid pair identities are now retained as censored,
excluded from supported evidence, and make the causal chain incomplete. All
462 orchestration tests pass.

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
