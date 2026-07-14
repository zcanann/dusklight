# huntctl

`huntctl` is the Rust control plane for persistent Dusklight simulation
workers. It owns worker lifecycle, request correlation, capability discovery,
batch-model types, and portable artifact identity. The game loop and per-tick
work remain in native C++.

## Build and test

```console
cargo build --manifest-path tools/huntctl/Cargo.toml
cargo test --manifest-path tools/huntctl/Cargo.toml
cargo clippy --manifest-path tools/huntctl/Cargo.toml --all-targets -- -D warnings
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

`src/protocol.rs` defines an explicit, little-endian framed model for the next
engine-session phase: build and artifact identities, `Hello`, `RunBatch`,
`Replay`, terminal outcomes, capability bits, and strict decoders. These types
are not sent to the bootstrap-only native worker yet.

The model sends complete tapes or candidate batches. It never serializes Rust
or C++ memory layouts and never performs per-tick IPC. Large traces and images
will remain content-addressed artifacts; shared-memory blob references can be
added after profiling without changing the coarse scheduling interface.

## DUSKTAPE authoring

The tape codec emits compact DUSKTAPE v2 files and decodes legacy v1.0-v1.2
tapes. V2 wraps the canonical 52-byte frame stream in one zstd frame, preserving
exact controller semantics while avoiding flat per-tick storage. Inspect a tape
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
