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
| Exact cold reproduction | Working: the learned 932-frame process-boot tape reproduced identical semantic gameplay state at all 932 boundaries across two cold runs |
| Useful route quality | Not established: the retained human Ordon segment is 126 input frames; the best attributable learned local result is 129 ticks and the from-scratch tactic policy is substantially slower |
| Reliability and generality | Not established: the published tactic-Q proof covers one seed, one start, and one terminal |
| Comparative learning value | Not established |

The first tactic-Q proof is retained at
`docs/glitch-hunting/benchmarks/ordon-tactic-q-first-proof-20260724.json`.
It proves basic competence and exact reproduction, not route quality,
repeatability, or generality.

## P0 — Make learned behavior reliably competent

- [ ] Repeat the tactic-Q campaign across independent seeds and report terminal
      success rate, time to first success, native ticks, useful decisions,
      visited states, and selected-action diversity.
- [ ] Diagnose failures using observed state/tactic transitions rather than
      adding route-specific coordinates, preferred-action sequences, or
      gameplay-state writes.
- [ ] Make the frozen exploration-free policy reproduce successful campaign
      behavior reliably instead of depending on one favorable exploratory
      trajectory.
- [ ] Exercise at least one additional exact start and authored goal through the
      same fact, tactic, checkpoint, and terminal interfaces.
- [ ] Preserve exact cold replay as the authority for every claimed success.

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

Phase-level tactic-Q timing, useful-decision counts, native ticks, and episode
rates are already recorded.

- [ ] Profile the real reliability or route-quality experiment and identify the
      phase that limits useful learning iterations.
- [ ] Benchmark worker counts appropriate to the current host if simulation or
      checkpoint restoration starves experience collection.
- [ ] Increase batching, worker utilization, or persistence efficiency only
      when the measured bottleneck prevents enough diverse state/tactic
      transitions or refinement candidates.

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
