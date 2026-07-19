# Automation primitives

These primitives form the stable boundary between search code and the game.
Their serialized forms must be versioned and independent of C++ padding,
pointer size, and host endianness.

## Build identity

Every artifact records enough identity to reject a misleading replay:

- Dusklight commit and dirty-tree digest;
- Aurora commit;
- compiler, target, build type, and feature flags;
- game data/region digest;
- protocol and artifact schema versions; and
- fidelity profile.

The portable artifact identity also authenticates the protocol name/version and
capability set; region and language assets; scenario and predicate program; the
action and observation schemas; and all simulation-relevant settings. Digests
are SHA-256 over canonical, versioned bytes. A path, filename, JSON object order,
or native structure layout is never an identity.

### Compatibility modes

Compatibility is always selected for a concrete operation. There is no generic
"close enough" or implicit use of the currently configured game path.

| Operation | Must match | Intentionally may differ |
| --- | --- | --- |
| Replay | artifact schema; complete build; game, region, language, scenario, predicate, action/observation schemas, settings; protocol/capabilities; fidelity | artifact payload digest only |
| Trace merge | complete build; game, region, language, action/observation schemas, settings; protocol/capabilities; fidelity | scenario and predicate; payload |
| Model training | build feature digest; game, region, language, action/observation schemas, settings; protocol/capabilities; fidelity | commits, compiler/target/configuration, dirty tree, scenario, predicate, payload |
| Checkpoint restore | complete build; game, region, language, scenario, settings; protocol/capabilities; fidelity | predicate and action/observation schemas; payload |
| Cross-build comparison | game, region, language, scenario, predicate, action/observation schemas, settings; protocol/capabilities; fidelity | complete build identity and payload |
| Cross-fidelity comparison | complete build except fidelity; game, region, language, scenario, predicate, action/observation schemas, settings; protocol/capabilities | fidelity and payload |

These are conservative rules. Relaxing one requires a new identity schema or an
explicit mode revision; callers may not silently omit a field. Rejections list
every mismatched required field with expected and received values.

`huntctl identity compare --mode MODE --expected EXPECTED.json --actual
ACTUAL.json` validates that both portable identity documents are complete, then
applies the selected operation's rules. A rejection prints every incompatible
field, including both values; a successful comparison emits a small JSON
report. This is currently an explicit inspection/enforcement boundary. Wiring
the same check into every artifact-consuming command remains incremental work,
but the core paths are guarded automatically: run results and cold replays use
replay compatibility, identified episode sets use model-training compatibility,
and cross-run oracle inputs use cross-build or cross-fidelity compatibility as
appropriate. The explicit command remains useful for inspecting two identities
without invoking one of those consumers.

## Scenario

A `Scenario` describes repeatable initial conditions, not just a map:

- save image or named fixture;
- stage, room, layer, entrance/start point, and player form;
- inventory, flags, health, magic, rupees, and other relevant save state;
- initial RNG streams;
- logical video mode and language;
- game settings that can affect input or timing;
- optional actor or memory fixtures; and
- required capabilities and fidelity profile.

Scenario setup may observe a declared ready condition before the replay clock
starts, then emits an initial state hash. Once tape playback starts, readiness
observations are assertions only: they may fail a run but must never delay,
skip, repeat, or choose input.

## SimTick

`SimTick` is a monotonically increasing logical update index. All inputs,
events, observations, network schedules, and checkpoint hashes are keyed to it.
Wall-clock timestamps may be recorded for profiling but never define replay
behavior.

## InputFrame and InputTape

An `InputFrame` holds the exact game-visible state of every emulated controller
port for one tick:

- digital button bits;
- signed 8-bit main-stick X/Y;
- signed 8-bit sub-stick X/Y;
- unsigned 8-bit analog triggers;
- unsigned 8-bit analog A/B;
- connection and error state; and
- explicitly versioned port-specific extensions, if any.

Do not serialize `sizeof(PADStatus)`: its padding and PC-only fields are build
details. Use a canonical packed schema.

An `InputTape` contains build-independent metadata, an explicit process or
stage boot origin, a scenario reference, run-length encoded frames, named
markers, and optional expected hash/event checkpoints. A stage origin includes
stage ID, room, spawn point, layer, and an optional memory-card save slot;
tape tick zero is armed only after that fixture's readiness assertion succeeds.
It supports lossless splice and trim, port-owned layering, exhaustive typed
diffs, deterministic authoring-rate resampling to canonical 30 Hz, and
neutral-frame insertion. Layering replaces the complete native PAD record only
for ports owned by the overlay; all other ports and the base boot identity are
preserved. A process-boot overlay is reusable, while an explicit stage-boot
overlay must have the exact same stage and fixture identity as the base.
Reactive waits are rejected by splice, layer, and resample because they do not
have a stable absolute tick.
The tape is the final proof produced by any higher-level controller. Its input
at tick N is fixed before replay begins; it contains no observation-dependent
waits or branches.

For a map-local test, the direct tape workflow is:

```sh
huntctl tape compile test.tas test.tape
huntctl tape run test.tape --game ./dusklight --dvd game.iso --state-root build/test-state \
  --milestone-goal arbitrary-map-goal --gameplay-trace build/test.trace
huntctl tape prove test.tape --game ./dusklight --dvd game.iso \
  --state-root build/proof-state --milestone-goal arbitrary-map-goal --repetitions 2
huntctl tape minimize test.tape minimized.tape --game ./dusklight --dvd game.iso \
  --state-root build/minimize-state --milestone-goal arbitrary-map-goal --repetitions 2
huntctl tape slice test.tape room-only.tape --start 120 --frames 300
huntctl tape layer room-only.tape correction.tape layered.tape --start 45
huntctl tape resample authored-60hz.tape canonical-30hz.tape
huntctl tape diff room-only.tape layered.tape
```

`tape prove` cold-replays one already-realized absolute tape at least twice. It
rejects reactive waits and replay-owned launch overrides, so no controller,
model, alternate tape, or altered proof target can enter the loop. Every
repetition must agree on goal reachability, simulation tick, tape frame, and
the complete terminal boundary fingerprint. The resulting
`dusklight-cold-replay-proof/v1` artifact binds the tape, executable, game data,
optional milestone program, boot origin, launch arguments, and retained trial
directory by path and content digest. If repetitions disagree on reachability
or exact boundary proof, the command retains every trial and writes a
`dusklight-replay-quarantine/v1` artifact binding the candidate, build, game
data, boot/scenario, objective, fidelity, and contradictory proofs. The error
prints that artifact path and the combination is explicitly ineligible for
promotion.

`tape minimize` treats the requested milestone goal as the run's semantic
success oracle. It first neutralizes contiguous chunks of active input with
ddmin, then retries individual active frames, and finally truncates after the
proved goal frame. Every accepted reduction must reproduce the source proof in
at least two cold runs: the same boot origin, goal simulation tick, goal tape
frame, and complete `dusklight.milestone-boundary/v4` fingerprint. The emitted
`.proof.json` retains the exact oracle/boundary evidence and all candidate-run
artifacts remain under the supplied state root. The proof also binds the source
and minimized tapes, executable, game data, optional milestone program, launch
arguments, and headless fixed-step fidelity by content digest. Reactive tapes
and replay-owned launch overrides are rejected before minimization begins.

`tape resample` treats the source as a piecewise-constant authoring signal and
samples it at the start of every canonical 30 Hz tick. Upsampling repeats exact
PAD records; downsampling selects the record active at the output tick and can
therefore discard shorter-than-one-tick authoring changes. `tape diff` reports
boot/rate metadata plus every changed frame field, including all native fields
on all four controller ports.

The text authoring DSL also has explicit stateful button transitions:

```text
dusktape 1
press p0 LEFT RIGHT
hold 2
release p0 LEFT
release p0 RIGHT
```

`press` and `release` each emit one frame derived from the preceding exact PAD
state; at the start of a program that state is neutral. `hold N` emits `N`
additional copies. Re-pressing a held bit or releasing an absent bit is an
authoring error rather than a silently redundant frame. Multiple names on one
press are one edge, and conflicting hardware-visible combinations such as
`LEFT RIGHT` remain set exactly as authored. The JSON program represents the
same operations as `{"op":"press|release","port":N,"buttons":[...]}`.

Generic minimization preserves absolute timing while neutralizing removable input frames, then
trims after the exact goal frame. It accepts a reduction only when repeated native runs reproduce
the same v4 boot-and-fixture-authenticated boundary fingerprint, simulation tick, and tape frame. An authored
milestone program may be supplied with `--milestone-program` for goals not built into Dusklight.

## ControllerProgram

A `ControllerProgram` drives search and compactly generates candidate tapes.
It is not itself a TAS proof. Initial operations may include:

- hold/release buttons for a tick range;
- set exact stick or trigger bytes;
- wait for a bounded semantic condition;
- choose a value or timing from a declared search range;
- repeat and branch on an observation; and
- invoke a parameterized motion segment.

Motion segments include exact-duration stick-space waypoint, rail, Catmull–Rom
spline, and Bézier paths plus angle/speed targets and roll spacing; bounded
state-feedback controllers cover observation-dependent movement. A successful run must materialize the
exact raw `InputFrame` sequence it actually used. Verification replays that
flattened sequence with all program waits and branches removed. A spline is a
search convenience, not a replay format.

Programs must be bounded: every wait, loop, and branch has a maximum tick count
and deterministic timeout result.

## Observation

An observation is a typed, stable value sampled on request. The first useful
set includes:

- player position, velocity, facing, animation, action/state, form, and room;
- camera and lock-on state;
- stage transition and loading state;
- controller state after normalization;
- RNG state or call counters;
- selected actor IDs, types, transforms, health, and process state;
- collision contacts and relevant polygon metadata;
- UI screen, selection, cursor, and text-entry state;
- heap usage and GC-relative offsets for watched objects; and
- canonical state hashes.

Trace channel coverage currently includes per-tick global RNG streams and call
counters, realized camera pose, Link procedure context/timers/animations,
ground/wall/roof/water contacts, collision correction vectors, and joined
local collision geometry. Channel 11 (`goal-progress`, fixed 32-byte version 1)
adds the configured goal's FNV-1a display key, overall requested/hit counts,
stable and ordered-sequence progress, and exact first-hit tick. It is sampled
after predicate evaluation at the same post-simulation boundary; the hash is a
compact feature key, not predicate identity.

Channel 12 (`selected-actors`, fixed 656-byte version 1) retains at most 16
non-player actors ordered by exact session process ID. Each entry copies actor
type, placed set/home identity, current room, health/status, position, and
current/shape angles. The header records the total observed population and an
explicit truncation bit. Lowest process IDs win independent of actor iteration
order, unused slots have canonical sentinels, and pointer values never enter
the artifact. This heavier channel is opt-in; `goal-progress` is in the default
set.

Stable actor handles combine a spawn/event identity and generation. Native
pointers are diagnostic values only and never enter portable artifacts or
hashes.

Placed actors use a portable composite identity: stage, home room, actor type,
and map-authored set ID. A runtime process ID is exact only for that actor's
lifetime in one engine session. Actor catalogs may add parameters, parent ID,
and home transform for diagnosis and disambiguation; they never use a pointer
as identity.

## InterventionTape

An `InterventionTape` is a separately gated sequence of typed game-state writes
used to test causal hypotheses, such as whether an enemy collision can push
Link across a fence. It is never folded into an `InputTape` and never presented
as an unmodified replay.

Each intervention declares its exact simulation phase, stable actor selector,
field-level operation, precondition, and bounded duration. Execution records
selector resolution plus before/written/after values. Baseline and treatment
runs retain identical scenarios and controller inputs so an oracle can attribute
the changed outcome to the declared intervention.

Intervention-built discoveries become normal glitch proofs only after the
required setup is reproduced without mutation. Until then they are useful
existence or mechanism evidence, not evidence that the setup is naturally
reachable.

## Event

Events make sparse behavior cheap to search and easy to inspect. Examples are:

- action or animation changed;
- room/stage transition requested or completed;
- actor spawned, deleted, damaged, or changed process state;
- collision contact began or ended;
- position crossed a boundary or moved discontinuously;
- input was consumed by a different UI/game state than expected;
- watched memory was written outside its intended field;
- crash, assertion, timeout, NaN, or invalid handle; and
- headful/headless state hashes diverged.

Each event records a tick, stable subject IDs, a schema version, and a compact
payload.

## Oracle

An oracle turns a run into a terminal classification and optional score. Useful
built-ins include:

- reached a target region, room, action, animation, or flag;
- crossed collision without the expected transition;
- exceeded plausible displacement, speed, or vertical range;
- entered out-of-bounds or invalid state;
- corrupted a watched field or heap guard;
- crashed or hung;
- produced a novel event/state signature; and
- diverged from a control replay or from headful execution.

Keep common oracles in C++ so every tick does not cross the process boundary.
Rust composes their results and runs expensive corpus-wide novelty decisions.

## Checkpoint

Checkpoint support is tiered because a native process is not trivially
serializable:

1. `Scenario`: reload save/stage and replay a prefix; portable and mandatory.
2. `GameState`: serialize explicitly supported game state with pointer fixups;
   faster but incomplete until proven otherwise.
3. `ProcessSnapshot`: restore memory, thread, and subsystem state; fastest in
   theory and highly platform-specific.

Every checkpoint has a build ID, parent scenario, tick, canonical state hash,
capability requirements, and a validation replay. A checkpoint that cannot
reproduce its next validation window is rejected.

## RunArtifact

A `RunArtifact` is the unit stored in the corpus. It contains:

- build identity and scenario;
- exact input tape and any source controller program;
- seed/mutation lineage;
- terminal reason, oracle results, and novelty signature;
- selected event and observation traces;
- periodic canonical state hashes;
- crash report or memory-write trace when applicable;
- optional screenshots/video from a verified headful replay; and
- performance counters that are clearly separated from logical time.

Artifacts should be content-addressed, immutable, and minimizable. A minimized
artifact retains the same semantic oracle and required fidelity capability.
