# Generic in-game goal learner

## Goal

Build a generic learner that uses native observations, actions, terminal
predicates, native tick cost, and its own restorable states to discover complete
solutions that are substantially better than human routes. Human replays are
optional experience, never a required route script or performance ceiling.

Ordon Springs is the first adequacy test. The real terminal is
`ordon_spring_load_committed`; the human replay reaches it in 125 active ticks.
The first pass gate is 124, convincing evidence is 123, and the current target
is 120.

## What is actually true

- Exact process-local checkpoints, graph edges, replay recovery, and terminal
  predicates work. Deterministic tests prove branching and multi-step return
  propagation, but native route quality does not yet prove useful learning.
- The latest matched zero-demonstration run reached the terminal in all four
  seeds but produced only a 257-tick best route. Goal-reachability controlled
  22 of 128 decisions; 106 decisions remained bootstrap exploration.
- Stable held-out folds fixed one invalid experiment, but authority still
  oscillates. Exact revision diagnostics show why: from replay revision 166 to
  176, 11 previously correct same-state rankings became wrong while seven
  recovered. A representative failure trusted a distant extrapolation over an
  action with ten-times-closer evidence; the extrapolated action achieved 1.2
  versus 16.1 progress per tick.
- Therefore the current blocker is policy quality and confidence, not attempt
  volume. No task is blocked on missing design.

## P0 - make retained-state learning improve decisions

- [ ] Make goal-reachability authority confidence-aware without suppressing
  uncertainty-driven exploration. Select any distance/uncertainty treatment
  using nested or otherwise leakage-free held-out evidence, not an Ordon-tuned
  constant. On exact replay revisions 166, 176, and 248, require better aggregate
  same-state ranking and regret without degrading the mature 72/113 baseline.
- [ ] Use retained checkpoints as the ordinary native search tree: choose a
  promising state, restore it directly, compare multiple legal sibling actions,
  admit every authentic outcome, and show that the next policy snapshot changes
  the action or checkpoint ranking for evidence-backed reasons. Do not replay
  every candidate from the cold root.
- [ ] Demonstrate native multi-step credit around the local optimum. A policy
  must prefer a temporarily worse first move when its retained continuation has
  lower authenticated ticks-to-terminal; immediate coordinate progress alone
  cannot solve this gate.
- [ ] Keep action discovery generic and state-aware. Movement, camera lock,
  rolling, simultaneous inputs, variable durations, and prompted actions must
  remain legal candidates; learned options must compete with their primitive
  components and be promoted only by held-out native improvement.

Exit: in a bounded zero-demonstration native run, learned ranking controls most
eligible decisions, checkpoint branching changes later choices, every seed
reaches the real terminal, and the best authenticated route materially improves
the current 257-tick result. One lucky seed or more blind attempts does not pass.

## P0 - beat and explain the Ordon route

- [ ] Discover a route of 124 ticks or less and cold-replay it twice with the
  same terminal identity and first-hit tick.
- [ ] Reach 123 ticks or less, then 120 ticks or less, without changing the
  generic objective, observations, action interface, or learning rules.
- [ ] Under one bounded envelope, run five fixed zero-shot seeds plus a permuted
  seed order. All must reach terminal; quality and discovery time must not rely
  on scheduler ordering, cross-run leakage, or one seed.
- [ ] Compare the improving learner with frozen-policy and random-valid controls
  on matched checkpoints and native-tick budgets. Attribute decisive ranking
  changes to specific experience and generic observations.
- [ ] Show one learner-discovered multi-action option improving held-out native
  ticks over its primitive components. Ablate an ordinary suboptimal human replay:
  it may accelerate learning but cannot be required or cap the final policy.

Exit: adaptive learning and reusable learned tactics causally produce
substantially better-than-human routes.

## P0 - make successful learning fast enough

- [ ] Only after the policy gates above pass, profile native execution, restore,
  fitting, graph scheduling, admission, persistence, and IPC on one fixed run.
- [ ] Optimize measured dominant costs and require less wall time to the same
  learned result. Compare one and two checkpoint-owning lanes; retain
  parallelism only when it increases unique useful experience without host
  interference or evidence contamination.

## P1 - generality and engineering quality

- [ ] Apply the unchanged interfaces and learning rules to a second native goal
  without route-specific shaping or authored tactics.
- [ ] Split mixed-responsibility production files into independently testable
  execution, checkpoint, branching, learning, proposal, tactic, persistence,
  replay-proof, and reporting modules. Files over roughly 1,000 lines require a
  concrete single-responsibility justification or decomposition.
- [ ] Keep persistent corpora, checkpoints, journals, and models bounded,
  checksummed, atomic, and binary. JSON is limited to small human-facing
  manifests, summaries, and explicit diagnostics.

## Operating rules

- Work capability-first. Do not optimize throughput while learning or execution
  semantics are invalid.
- Do not launch another native campaign until a deterministic or replay-only
  diagnosis identifies a concrete defect and its fix passes.
- Every native campaign is bounded and answers multiple hypotheses. Every branch
  contributes authentic evidence; every promoted route is cold-replayed twice.
- Use at most two owned build/native workers. Manage only exact child processes
  started by this session; never stop unrelated Codex, Cargo, emulator, or worker
  processes.
- Commit and push every natural milestone; do not leave a long-lived dirty tree.
