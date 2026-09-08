# Transition-level hindsight in the Bellman learner

## Capability change

`--value-treatment hindsight_bellman_forest` selects an experimental V3
treatment built on the [parameterized Bellman kernel](2026-09-07-parameterized-bellman.md).
It includes unfinished and unconnected attempts, not only recorded paths that
eventually reached a goal. The existing default has not changed.

The replay contains every native transition plus auxiliary versions for up to
four distinct achieved positions sampled from the recorded after-states. Each
auxiliary goal is applied across the corpus, including attempts without a known
continuation to it. A row already at that auxiliary goal before acting is omitted
from that task; waiting there does not teach arrival at a new goal.

Auxiliary success requires a present player, matching stage, room and layer,
and matching full 3D coordinates. Native success still requires the original
game predicate. Goal-kind and target-world features distinguish the learning
tasks; they do not constitute proof that predictions generalize correctly.

## Objective and task semantics

V3 uses a time-cost objective for both native and auxiliary training tasks:

`-realized_duration_ticks + discount^duration * max Q(next_state, available_action)`

The continuation is zero only when that task's goal is reached. This deliberately
differs from V2, which uses the recorded reward. Recorded rewards, physical facts,
and native terminal evidence remain unchanged. There is no added reward for
straightness, rolling, wall avoidance, or proximity.

Relabeling changes goal-relative observations and terminal feature columns,
not just the target value. A native goal stop must not disable successor actions
for an unfinished auxiliary task. The bridge clears that stop in a virtual
applicability snapshot; it never rewrites the physical transition.

Before any native-goal success, when auxiliary successes exist, predictions
query the requested coordinate in the current world as an exploration task.
After native-goal support exists, they query the authored task. This is an
explicit experimental task-selection rule, not evidence that reaching a
coordinate satisfies a loading-zone predicate. Actual completion and route
promotion still require native evidence and cold replay.

## Integration and cost

The treatment uses the existing immutable snapshot, live action selection,
frontier ranking, and unmanaged fitting-cache paths. It shares the same forest,
Bellman update, and successor action generator as V2.

Successor queries retain the common state/context prefix once per transition
and only action factors per candidate. This avoids multiplying the entire
observation vector by every candidate and auxiliary task. The explicit
64-million-scalar successor-cache limit remains; this is not a total process
memory bound. Augmentation can still increase fitting time substantially.

## Verification

- 469 learning and 502 orchestration library tests passed. A subsequently added
  focused terminal-feature relabeling test also passed.
- Tests cover native evidence preservation, auxiliary coverage of unconnected
  attempts, omission of already-satisfied tasks, world/height/player checks,
  relabeled terminal features, and nonterminal successor availability after a
  native goal stop.
- Compact successor queries produce the same kernel estimates as expanded
  state-action vectors.
- The CLI and its test targets pass `cargo check --all-targets`; existing
  unused-import/dead-code warnings in `tests/harness_cli.rs` remain.
  The focused CLI treatment-selection test also passes with the new option.
- An immutable V3 snapshot trained without native success drives executable
  action selection in the orchestration fixture. This tests wiring, not route
  discovery.

The read-only saved-experience inspection used the failed first hindsight seed
from the September 6 comparison:

- 128 native rows, zero native terminal rows.
- Four auxiliary goals, 510 auxiliary rows, four auxiliary terminal rows;
  two already-satisfied rows omitted.
- 7,325,471 retained successor feature scalars (about 29.3 MB of scalar payload,
  excluding container overhead and the rest of the learner).
- Four Bellman updates took 25.765 seconds in the unoptimized development build,
  excluding loading and ranking. The earlier V2-only diagnostic took 6.063
  seconds; these are individual diagnostics, not a controlled throughput test.
- All 74 recorded candidate descriptors received finite predictions. The top
  five included prompted-action, neutral, and seek-target descriptors. Their
  ordering is not a demonstrated route improvement, and the recorded query set
  is not a claim that every descriptor is currently applicable.

Reproduce using the checkpoint command in
[the neighbor-lookup note](2026-09-07-neighbor-lookup.md), appending
`--hindsight-bellman`. The checkpoint and replay stay binary; the inspector's
small human-facing report is JSON. The diagnostic does not execute controllers.

## Limits and next decision

The four achieved goals and fixed candidate sample bound this implementation;
neither covers every possible goal or action composition. Exact-coordinate
auxiliary goals and the pre-success task-selection rule need behavioral
evaluation. A shared model may generalize poorly across goals, and its finite
Bellman updates do not establish convergence or correct long-horizon values.

No native campaign or release rebuild was launched for this milestone. The
September 6 native comparison remains the route-quality evidence; no broad
TASKS.md capability is marked complete. Next, use a bounded comparison against
the nonlearning control to decide whether this change improves actual discovery
enough to justify its fitting cost. Do not treat successful fitting or changed
rankings as success on the game goal.
