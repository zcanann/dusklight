# Active task: beat the Ordon Springs segment

The only current framework objective is:

> Starting at first Link control, automatically find a valid input sequence
> that reaches `ordon_spring_load_committed` faster than the 125-tick
> incumbent, then replay it deterministically from boot.

This is a proof task, not the complete glitch-hunting product plan.

## Invariants

- Identical tape plus identical initial state must produce identical per-tick
  state. Any disagreement is a framework bug; do not hide it by mining a more
  tolerant tape.
- Route time is simulated game ticks. Shader compilation, rendering, host I/O,
  prefix replay, and search time affect throughput but never the score.
- Ordinary playback supplies controller input and may inspect game state. It
  does not patch gameplay state.
- Checkpoint restore is allowed only as an experiment accelerator. A result is
  proven by an ordinary input-only replay from cold boot.
- The per-tick experiment loop belongs in native code. Out-of-process tooling
  may configure a batch and collect results, but it does not drive every frame.
- Add observations and actions when a concrete experiment requires them. Do
  not harvest the whole game in advance.

## Completed benchmark setup

- [x] Fix the source boundary at first controllable Link input.
- [x] Fix `ordon_spring_load_committed` as the terminal authority.
- [x] Preserve the human reference and 125-tick incumbent.
- [x] Measure the existing cold-process experiment cost.
- [x] Add authenticated route diagnostics for position, heading, collision,
  action state, and roll timing.

Source fingerprint: `ac7c32788fc3b5c59046386d95b9b5b4`

Human reference: `intro/segments/to_ordon_spring_human150.tape`

Measurements: `docs/glitch-hunting/throughput.md`

## 1. Prove a reusable source checkpoint

- [ ] Capture the complete simulation state at the source boundary.
- [ ] Restore it without relaunching or replaying the boot prefix.
- [ ] Run A/B/A suffixes and compare every tick, including controller history,
  RNG/clocks, game memory, and required host-side engine state.
- [ ] Stop at the first mismatch and identify the state component responsible.
- [ ] Compare checkpointed suffixes against fresh boot-and-prefix execution.
- [ ] Measure restore cost before attempting incremental or copy-on-write
  checkpoints.

**Pass condition:** one process executes 1,000 short checkpointed attempts with
zero A/B/A or cold-run divergence.

## 2. Run candidates in-process

- [ ] Add an episode boundary immediately before controller input is consumed.
- [ ] Accept a batch of candidate input programs once, then execute and score
  them without per-frame IPC or filesystem traffic.
- [ ] Keep routine results in memory: success, first predicate-hit tick, final
  fingerprint, and compact diagnostics for failures.
- [ ] Record the exact pad state consumed each tick so any successful attempt
  can be exported as an ordinary tape.
- [ ] Initially expose only what this route needs: Link position, velocity,
  facing/action/roll state, camera heading, collision correction, transition
  state, prior input, and restore-validation identity.

**Pass condition:** replaying an exported candidate consumes the same pad state
and produces the same state hash and predicate result on every tick.

## 3. Establish useful search baselines

- [ ] Search exact button-edge timing, held-input deletion, stick heading,
  heading duration, corner timing, and roll timing from the same checkpoint.
- [ ] Add only the smallest structured actions needed to express those trials;
  raw per-frame pad input remains authoritative.
- [ ] Compare methods under the same number of simulated candidate ticks.
- [ ] Use terminal success and first-hit tick as the objective. Any shaped
  progress signal is diagnostic and cannot declare a winner.
- [ ] Preserve materially different successful terminal states instead of
  assuming the locally fastest state is always the best continuation.
- [ ] Try a small replay/value learner only after these samples exist, and keep
  it only if it orders candidates or finds wins better than the measured
  mutation baselines.

**Pass condition:** an automated lane repeatedly finds better valid candidates
than manual frame-by-frame editing for the same simulation budget.

## 4. Promote the first win

- [ ] Export the best candidate into the absolute boot tape.
- [ ] Exhaust its obvious neighboring input timings and headings.
- [ ] Replay it from cold boot five times with identical per-tick hashes,
  first-hit tick, terminal predicate, and terminal fingerprint.
- [ ] Record the incumbent and winner scores, candidate-tick budget, attempts
  per second, checkpoint cost, methods compared, and source of the frame gain.

**Done condition:** a deterministic cold-boot tape reaches Ordon Springs at
least one tick faster than the 125-tick incumbent.

## Not current work

- Replicating the Skybook catalog. Individual documented glitches may become
  later benchmarks when explicitly selected; they are not a blind backlog.
- A general visualization workbench.
- A complete actor, object, collision, or map-information API.
- A named large learning architecture chosen before baseline data exists.
- Distributed orchestration or elaborate checkpoint machinery without a
  measured need from this benchmark.

After the proof, the next task is chosen from the bottleneck the experiment
actually reveals—not from the full eventual wishlist.
