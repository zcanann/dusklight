# Active task: make the learning framework useful

This is the sole dependency-ordered roadmap for the learning framework.
Implementation history belongs in Git and benchmark reports. Keep current
product truth and unfinished work here; delete tasks when they are complete.

## Product

Give an agent:

- an authenticated starting checkpoint;
- an authored terminal goal;
- typed observable facts and derived progress measurements;
- a finite catalog of applicable actions and multi-tick tactics; and
- enough native simulation budget to try alternatives from retained states.

The agent chooses tactics, observes their effects, learns which choices lead to
valuable future states, branches from useful checkpoints, and eventually
reaches the terminal. The terminal remains binary, while ordinary observable
measurements provide intermediate learning signal.

Tactics may be atomic or bounded programmatic compositions. Movement tactics
include reactive target seeking and static waypoint, rail, spline, and Bézier
paths. Every selected tactic emits ordinary controller input and records an
authenticated state/tactic transition with its duration, reward, checkpoint,
and exact tape range.

A human recording may optionally contribute experience through the existing
replay pipeline. It must not define a private action space, hidden observation,
fabricated state, or privileged terminal.

The operator is an LLM or another programmatic client. The browser workbench
records, inspects, and replays graphs produced by execution. It does not author
tactics, tape frames, compositions, or semantic action graphs.

## Current truth

| Capability | State |
|---|---|
| Deterministic execution and checkpoints | Working |
| Typed learner state and measurements | Working: authenticated fact snapshots, derived measures, applicability masks, bounded infodumps, and exact schema identities |
| Executable tactic catalog | Working: generic and game-specific tactics, reactive movement, static motion paths, and bounded sequential/concurrent composition share one runtime action contract |
| Semi-Markov tactic learning | Working: duration-aware replay, fitted-Q refits, reward shaping, hindsight, deterministic ranking, and selected-tactic execution |
| Checkpoint branching | Working: replayable quality-diversity frontiers, root/frontier sampling, detached-restore rejection, and collapse/connectivity diagnostics |
| Persistence | Working: checkpoint/resume, frozen greedy policies, final-result export, exact tapes, and authenticated evidence |
| Human experience | Working: native replay corpora accept demonstration episodes and existing learning modes consume them |
| Recording/playback workbench | Working: launch, pause/resume/cancel, graph projection, on-demand authenticated detail, exact edge/path playback, cleanup, and successful-tape export |
| First from-scratch proof | Working: no-demonstration seed 181081 reached the Ordon terminal after 70 exploratory decisions; its frozen greedy policy reached it in 13 decisions and 426 native ticks |
| Exact cold reproduction | Working: learned process-boot tapes for both exact Ordon starts reproduced identical terminal evidence across two cold runs |
| Useful route quality | Not established: the retained human Ordon segment is 126 input frames; the best attributable learned local result is 129 ticks and the from-scratch tactic policy is substantially slower |
| Reliability and generality | Partly established: the common learner succeeds from both the Link-control and authored exit-approach starts, but the full route remains one success in four current-build seeds and the second pair has only one evaluated seed |
| Comparative learning value | Not established |

The first tactic-Q proof is retained at
`docs/glitch-hunting/benchmarks/ordon-tactic-q-first-proof-20260724.json`.
The independent-seed result and its current-build replay proof are retained at
`docs/glitch-hunting/benchmarks/ordon-tactic-q-reliability-20260724.json`.
Together they prove basic competence and exact reproduction, but the measured
25% campaign success rate does not prove reliable competence or generality.

The second exact start/goal proof is retained at
`docs/glitch-hunting/benchmarks/ordon-exit-approach-tactic-q-second-pair-20260725.json`.
It proves the shared learner from the authenticated exit-approach boundary,
while honestly retaining the 12-frame authored suffix as the faster incumbent.

## P1 — Improve route quality

Competence and efficiency are different claims. A terminal-reaching route is
not automatically a useful route.

- [ ] Optimize elapsed native ticks and input complexity only among candidates
      that reach the authored terminal.
- [ ] Hand successful learned tapes directly to the existing bounded
      continuous/discrete refinement and exact-reduction machinery without
      introducing another tape, state, action, or evidence format.
- [ ] Use observable progress, tactic outcomes, uncertainty, and retained
      checkpoints to propose meaningful alternatives instead of relying only on
      blind frame mutation.
- [ ] Beat the retained 126-frame human
      `to_ordon_spring_q125.tape` segment from the same authenticated
      Link-control boundary and cold-prove the complete winning tape.
- [ ] Publish the winning lineage, simulation budget, exact input change,
      segment time, terminal evidence, and independent replay identities.
- [ ] Keep the retained human segment as the incumbent unless a strictly faster
      machine-generated descendant passes the same proof contract.

The Ordon result is an important concrete performance benchmark. It is not the
definition of the entire learning framework.

## P2 — Validate optional human experience

Demonstration recording, replay ingestion, and learning modes already exist.
Do not create a second demonstration format or a privileged action path.

- [ ] Compare otherwise identical cold-start and demonstration-seeded tactic
      learning by time to first terminal, final route quality, and simulation
      budget.
- [ ] Prove that removing demonstration rows removes no action, observation,
      measurement, checkpoint, terminal, or refinement capability.
- [ ] Distinguish replay training, behavior-cloning warm start, and reverse
      curriculum results instead of grouping all human-assisted runs together.
- [ ] Preserve demonstration provenance in published comparisons.

## P3 — Establish comparative learning value

- [ ] Define a compact sealed comparison using the actual tactic-Q learner.
- [ ] Compare learning against random applicable-tactic selection and
      non-learning structured search under equal useful native-simulation
      budgets.
- [ ] Repeat across enough seeds and at least one held-out start to report
      uncertainty rather than a single winner.
- [ ] Publish success rate, time to first success, best route, useful
      state/tactic coverage, and candidate diversity even when learning loses.
- [ ] Run a larger treatment matrix only if it answers a remaining product
      question.

## P4 — Optimize throughput only when measured

The four-seed reliability experiment found that evidence projection and
persistence consumes 65.4% of summed seed wall time. Native tactic execution is
the next largest phase; model fitting is not the bottleneck.

- [ ] Benchmark one, two, and four independent tactic-route workers on the
      current host under one sealed configuration.
- [ ] Make tactic-route honor the selected worker count and reduce evidence
      persistence overhead, then prove higher useful-decision throughput without
      changing campaign identities or outcomes.

More transitions per second are useful only when they improve the rate of
meaningful, independently evaluated behavior.

## Completion

The framework is useful when it:

1. reliably learns terminal-reaching behavior from more than one exact
   start/goal pair through the common fact and tactic interfaces;
2. produces exact policies and tapes that reproduce from cold boot;
3. demonstrates measurable value over appropriate equal-budget baselines; and
4. improves at least one meaningful route-quality benchmark, including a
   cold-proven attempt to beat the retained 126-frame Ordon segment.
