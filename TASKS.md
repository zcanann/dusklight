# Route optimization proof

The immediate job is narrow:

> Improve the existing Link-control-to-Ordon-Springs segment faster than a
> human can improve it frame by frame, then prove the result from cold boot.

This file is an execution queue, not a product wishlist.

## Rules

- Playback must be deterministic. A mismatch is a framework bug, never a cue
  to search for a more tolerant tape.
- Score game ticks only. Startup, rendering, shaders, host I/O, and training
  time are throughput costs, not route time.
- Normal playback may read game state and supply controller input; it may not
  patch gameplay state.
- Checkpoints accelerate experiments but do not prove a route. Every promoted
  result must replay as an ordinary input tape from cold boot.
- Keep per-tick observation and control native. Rust may configure a batch and
  collect its result, but it must not sit in the frame loop.
- Do not choose an algorithm by name. Compare methods under the same simulated
  tick budget and retain the one that produces better candidates.

## Done: freeze the benchmark

- [x] Fix the source boundary at first controllable Link input.
- [x] Fix `ordon_spring_load_committed` as the terminal predicate.
- [x] Retain the human route and the current 125-tick incumbent.
- [x] Measure cold-process throughput, prefix cost, process launches, CPU,
  simulator idle time, and artifact I/O.
- [x] Report route defects from authenticated per-tick state: excess distance,
  heading error, collision loss, corner duration, and roll timing.

The frozen source fingerprint is `ac7c32788fc3b5c59046386d95b9b5b4`.
The human reference is `intro/segments/to_ordon_spring_human150.tape`.
Benchmark details live in `docs/glitch-hunting/throughput.md`.

## Next: remove experiment overhead

- [ ] Keep one game process alive and replay the prefix once for a batch.
- [ ] Capture a checkpoint at the source boundary.
- [ ] Restore every state component that affects subsequent simulation,
  including game memory, RNG, clocks, controller state, and required host-side
  engine state.
- [ ] Prove restore correctness with A/B/A suffix runs and per-tick hashes.
- [ ] Compare checkpointed suffixes with fresh boot-and-prefix runs.
- [ ] Stop a worker at its first divergent tick and retain evidence for that
  divergence.
- [ ] Keep routine candidates and results in memory; do not create a trace,
  database, screenshot, or directory per attempt.
- [ ] Measure full checkpoint copies before building a clever incremental
  snapshot system.

**Gate:** one worker completes 1,000 short suffix attempts without relaunching
or replaying the prefix, with zero checkpoint or cold-run divergence.

## Then: evaluate decisions inside the game loop

- [ ] Add a native episode boundary immediately before controller input is
  consumed: observe, choose input, advance one tick, score.
- [ ] Upload one candidate tape or small policy program per attempt or batch;
  allow no per-frame IPC or filesystem traffic.
- [ ] Preserve the exact consumed pad state each tick so every candidate can be
  exported as an ordinary tape.
- [ ] Surface only the state needed to optimize this segment initially:
  position, velocity, facing, action/roll state, camera heading, collision
  correction, transition state, prior input, and RNG/clock identity needed to
  validate restores.
- [ ] Support exact raw pad states plus a small set of generic route actions:
  timed heading, button edge, roll, seek point/opening, and short waypoint or
  curve sequences.
- [ ] Keep actions deterministic and composable, with explicit ownership of
  sticks and buttons when they overlap.

Do not encode this route's coordinates, corner frames, or roll spacing into
the primitives.

**Gate:** a raw tape and a policy choosing the same inputs produce the same
consumed pad states, per-tick hashes, terminal predicate, and realized tape.

## Then: prove which search works

- [ ] From an identical checkpoint, compare input deletion, local button/frame
  search, structured action search, and continuous parameter search under an
  equal candidate-tick budget.
- [ ] Use terminal success and first-hit tick as authority. Keep shaped progress
  scores separate and inspectable.
- [ ] Preserve successful terminal states that differ materially; a faster
  intermediate result is not automatically the best continuation.
- [ ] Add replay/value learning only if the measured baselines leave a gap it
  can plausibly close. Judge it by candidate ordering and route wins, not loss
  curves or the presence of a DQN label.
- [ ] Permit bounded lookahead or continuation repair so an early improvement
  is not rejected solely because the old suffix no longer fits.

**Gate:** one automated method consistently finds better valid candidates than
manual/local tape mutation for the same simulated tick budget.

## Finish: promote one real improvement

- [ ] Export the best candidate as an absolute boot-to-terminal tape.
- [ ] Remove redundant inputs only when exact playback preserves first-hit tick,
  terminal predicate, and terminal fingerprint.
- [ ] Exhaust the obvious local neighborhood around its input edges, headings,
  action durations, corners, and rolls.
- [ ] Cold replay it five times with identical per-tick hashes and evidence.
- [ ] Record incumbent and winner ticks, simulated-tick budget, throughput,
  checkpoint cost, methods compared, and the decisions responsible for the
  gain.

**Gate:** a cold-replayable tape reaches Ordon Springs multiple ticks earlier
than the incumbent, with the gain attributable to tested inputs rather than
nondeterminism.

## Explicitly out of scope

- Blindly reproducing the Skybook glitch catalog.
- A general-purpose visualization or agent-observation workbench.
- Harvesting every actor, object, polygon, or metadata field in advance.
- Building DDQN, Q-learning, or any other named model before the experiment
  loop and comparison baselines justify it.
- Save-state sophistication, multi-process orchestration, or distributed
  training beyond what the route benchmark demonstrates is necessary.
- Gameplay patches presented as automation, proof, or glitch reproduction.

Future work earns a place here only when this benchmark exposes a concrete
missing capability or after this proof is complete.
