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

The tape codec is byte-compatible with native DUSKTAPE v1.1 and decodes legacy
v1.0 tapes, including v1.0's derived controller error values. Inspect a tape or
expand its exact frames as JSON with:

```console
huntctl tape inspect run.tape
huntctl tape inspect run.tape --frames
```

Compile a strict `dusktape-program/v1` JSON program with:

```console
huntctl tape compile boot.json boot.tape
```

Example:

```json
{
  "schema": "dusktape-program/v1",
  "tick_rate": { "numerator": 30, "denominator": 1 },
  "default_owned_ports": 1,
  "steps": [
    { "op": "marker", "name": "boot" },
    { "op": "repeat", "count": 60, "frame": {} },
    { "op": "frame", "frame": { "pads": { "0": { "buttons": ["START"] } } } },
    { "op": "hold", "count": 1 },
    { "op": "repeat", "count": 30, "frame": {} }
  ]
}
```

`frame` emits one exact frame. `repeat` emits `count` copies of its exact frame.
`hold` emits `count` additional copies of the last emitted frame. Missing pad
fields are neutral; missing pads are neutral and connected. `owned_ports` is a
four-bit mask and may be overridden per frame. Buttons accept either a raw
`u16` mask or names: `LEFT`, `RIGHT`, `DOWN`, `UP`, `Z`, `R`, `L`, `A`, `B`,
`X`, `Y`, and `START`/`MENU`.

The binary tape has no marker channel. Compilation therefore always writes an
external `OUTPUT.tape.markers.json` sidecar, keeping the replay bytes canonical
while preserving source markers at exact tick offsets. Unknown JSON fields,
invalid ports, zero counts, duplicate/empty markers, and expansions beyond ten
million frames are rejected.
