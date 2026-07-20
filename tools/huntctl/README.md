# huntctl

`huntctl` is the Rust orchestration and CLI layer for Dusklight automation.
Portable contracts, immutable evidence, worker communication, and read-only
world queries live in smaller workspace crates with one-way dependencies. The
game loop and per-tick work remain in native C++.

## Crate boundaries

The enforced dependency graph and ownership rules are documented in
[`crates/README.md`](crates/README.md). Existing `huntctl::artifact`,
`huntctl::tape`, `huntctl::episode`, `huntctl::client`, and world-query paths
remain compatibility re-exports; their implementations are not owned by the
root crate anymore.

## Build and test

```console
cargo build --manifest-path tools/huntctl/Cargo.toml --workspace
cargo test --manifest-path tools/huntctl/Cargo.toml --workspace --all-targets
cargo clippy --manifest-path tools/huntctl/Cargo.toml --workspace --all-targets -- -D warnings
python3 tests/huntctl_crate_boundary_test.py
```

## Native worker

The native worker is launched with `--automation-worker`. Repeat
`--worker-arg` to pass worker arguments:

```console
huntctl hello --worker path/to/dusklight --worker-arg --automation-worker
huntctl ping --worker path/to/dusklight --worker-arg --automation-worker
```

The live bootstrap contract is versioned NDJSON over stdin/stdout. Requests
contain a numeric `id` and `command`. Responses echo the ID and contain:

```json
{"protocol":{"name":"dusklight-automation","version":1},"type":"pong","ok":true,"id":2}
```

Protocol version 1 implements `hello`, `ping`, and `shutdown`. `hello` reports
the exact native build identity and honest capability flags. The client checks
protocol name, version, response type, and request correlation on every
response. Worker stderr is inherited; stdout is reserved for the protocol.

`huntctl run` and `huntctl replay` currently perform capability negotiation and
fail clearly because the native endpoint advertises no engine session or input
tape support yet.

## Persistent worker pools

Start a pool once and schedule parallel health jobs across its persistent
processes with:

```console
huntctl pool health --worker path/to/dusklight \
  --worker-arg --automation-worker --workers 8 --checks 64
```

Every worker is spawned and negotiated independently. Spawn and protocol
failures are reported per worker while compatible siblings remain usable. By
default, the pool requires the complete reported build identity to match the
first healthy worker. Pass `--allow-mixed-builds` only when deliberately
comparing builds; metadata retains each worker's identity.

Each worker owns one control thread and accepts coarse jobs through a channel.
Jobs on one worker remain ordered while different workers run concurrently.
The pool sends no per-tick IPC, and clean shutdown is attempted for every
accepted worker even if another worker fails.

## Mock worker

The binary includes a compatible mock worker for process-level tests:

```console
cargo run --manifest-path tools/huntctl/Cargo.toml -- ping \
  --worker tools/huntctl/target/debug/huntctl --worker-arg mock-worker
```

Use `huntctl.exe` in the worker path on Windows.

## Coarse batch model

`crates/worker/src/protocol.rs` defines an explicit, little-endian framed model
for the next engine-session phase: build and artifact identities, `Hello`,
`RunBatch`, `Replay`, terminal outcomes, capability bits, and strict decoders.
These types are not sent to the bootstrap-only native worker yet.

The model sends complete tapes or candidate batches. It never serializes Rust
or C++ memory layouts and never performs per-tick IPC. Large traces and images
will remain content-addressed artifacts; shared-memory blob references can be
added after profiling without changing the coarse scheduling interface.

## DUSKTAPE authoring

The tape codec emits compact DUSKTAPE v3.2 files and decodes legacy v1 and v2
tapes. V2 introduced the zstd-wrapped canonical 52-byte frame stream; v3 adds
an authenticated boot origin and optional canonical scenario descriptor while preserving
exact controller semantics. Inspect a tape
or expand its exact frames as JSON with:

```console
huntctl tape inspect run.tape
huntctl tape inspect run.tape --frames
```

Compile the TAS state DSL with:

```console
huntctl tape compile boot.tas boot.tape
```

Example:

```text
dusktape 1
rate 30/1
ports 0x0f

state neutral {}
state start { p0 buttons START }
state left_a { p0 buttons A stick -127 0 }

marker boot
repeat 60 neutral
frame start
cycle 3 { frame left_a; frame neutral }
repeat 30 neutral
```

Named `state`s make controller configurations reusable. `frame` emits one
state, `repeat` emits a state N times, `cycle` repeats an exact state sequence,
and `hold` repeats the last emitted frame. `wait` and `pulse` preserve the
legacy conditioned-input features for experiments, although checked TAS tapes
reject them. Missing pads are neutral and connected. Frame bodies support
buttons, main/sub sticks, analog triggers, connection state, error values, and
an `owned` port-mask override. `#` starts a comment. Parser errors include the
source line and column.

The binary tape has no marker channel. Compilation therefore always writes an
external `OUTPUT.tape.markers.json` sidecar, keeping the replay bytes canonical
while preserving source markers at exact tick offsets. Unknown commands or
states, invalid ports, zero counts, duplicate/empty markers, and expansions
beyond ten million frames are rejected. Leading-`{` legacy JSON programs remain
accepted for migration only.

`DUSKTAPE` v3 carries an explicit boot origin. Authored programs may use
`boot process` for normal executable startup or
`boot stage STAGE ROOM POINT LAYER` for a targeted map fixture. Add
`save SLOT` to load memory-card slot 1-3 before establishing that stage:

```text
dusktape 1
boot stage F_SP103 1 1 3 save 2
frame { p0 stick 0 127 }
```

Dusklight maps the descriptor onto its existing `--stage`/`--load-save`
startup path, holds the
declared controller ports neutral while loading, and starts tape frame zero only
after the stage, room, point, layer, and player readiness check succeeds. The
readiness wait is bounded to 60 seconds of logical startup ticks and reports
both the requested and observed fixture on failure. The decoder remains
compatible with v1, v2, and v3.0 tapes. Older tapes retain process-boot or
stage-without-save semantics as originally encoded.

V3.2 can embed one canonical, self-contained scenario descriptor in a stage-boot tape. Compile
the checked JSON descriptor alongside the TAS source:

```console
huntctl tape compile routes/samples/maps/fsp103_next_map_seed.tas \
  build/fsp103-practice.tape \
  --fixture routes/samples/maps/fsp103_next_map_seed.fixture.json
```

The `dusklight-scenario-fixture/v1` JSON schema supports form, current/maximum health, both exact
native RNG streams, video mode, inventory quantities, equipment, event/temporary/dungeon/switch
flags, and typed setting overrides. Dusklight validates native ranges up front and applies the
descriptor in engine order: settings/video before subsystem initialization, save-backed state
before the play scene, dungeon and room switches before actors, and RNG at tick zero. Unknown or
unsupported values fail the run; fields are never silently ignored. The descriptor is included in
tape identity, traces, milestone fingerprints, search partitions, and corpus metadata.

The descriptor wire form can also be compiled or inspected independently:

```console
huntctl fixture compile practice.fixture.json practice.fixture
huntctl fixture inspect practice.fixture
```

Run a compiled map-local tape without translating its origin back into launch
arguments:

```console
huntctl tape run build/fsp103.tape \
  --game build/macos-default-debug/Dusklight.app/Contents/MacOS/Dusklight \
  --dvd orig/GZ2E01/GZ2E01.iso \
  --state-root build/automation-state/fsp103-test \
  --milestone-program build/map-test.dmsp \
  --milestone-goal reached_target
```

`--milestone-goal` also requests that milestone when `--milestones` is omitted.
The command runs fixed-step and headless by default, writes structured milestone
evidence below the state root, enforces a host timeout, and reports the tape's
decoded boot origin in its JSON summary. Use `--headful` for visual practice or
manual inspection.

Add `--gameplay-trace build/run.trace` to retain per-tick evidence. The default
channels include goal progress; request the heavier bounded actor population
explicitly when needed:

```console
--gameplay-trace-channels core,stage,player-motion,player-action,rng,camera,goal-progress,selected-actors
```

For a cold proof that native learning capture does not change controller input
or gameplay, record `all` trace channels once during ordinary playback and once
during a single tape-passthrough suffix batch, then bind both traces to the
batch's episode shard:

```text
huntctl trace observation-parity off.trace on.trace \
  --learning-shard on-result.json.episodes.dseps \
  --output observation-parity.json
```

The command requires complete, unretained v5 traces; compares the exact
pre-clamp `PADRead` sequence and every retained gameplay-state channel; checks
that the v4 shard's chosen and consumed PAD agree with the traced suffix; seals
the report; and exits nonzero on the first divergence.

`selected-actors` keeps at most 16 non-player actors and reports the full
observed count plus truncation. `huntctl trace inspect build/run.trace` exposes
both new typed channels.

To extend a stage fixture with human input, record after any seed tape (often a
single neutral frame):

```console
huntctl tape record build/fsp103-seed.tape build/fsp103-walk.tape \
  --game build/macos-default-debug/Dusklight.app/Contents/MacOS/Dusklight \
  --dvd orig/GZ2E01/GZ2E01.iso \
  --state-root build/automation-state/fsp103-recording
```

Close Dusklight to finish. `huntctl` retains the native process-boot
continuation for audit, composes it after the exact seed, and writes a standalone
tape carrying the seed's stage/save origin. Existing output files and stale
continuations are refused rather than overwritten.

## Content-addressed corpus

Initialize and populate an append-only local corpus with:

```console
huntctl corpus init corpus
huntctl corpus ingest corpus --tape boot.tape --scenario boot-title \
  --build build.json --scenario-json scenario.json
huntctl corpus list corpus
huntctl corpus show corpus ARTIFACT_SHA256
huntctl corpus verify corpus
```

`build.json` contains the `BuildIdentity` fields used by run manifests:
Dusklight and Aurora commits, compiler, target, profile, SHA-256 feature/game
digests, optional dirty-tree digest, and fidelity profile. Scenario metadata is
a JSON object; omitting `--scenario-json` stores an empty object.

Canonical tape bytes live under `blobs/sha256/<prefix>/<remainder>`. Versioned
run manifests are addressed by the real SHA-256 of their canonical JSON and
stored under `runs/<sha256>.json`. Writes use a synced temporary file and
atomic rename; existing content is compared and never overwritten by the
store. Duplicate ingest returns the same artifact ID with `created: false`.

`corpus verify` checks manifest filenames and schemas, referenced sizes, every
referenced tape, and every blob hash. Corruption, malformed digest paths, and
unknown schemas are errors. Manifests contain stable digests and metadata only;
they never record native pointers or implicit host paths.
