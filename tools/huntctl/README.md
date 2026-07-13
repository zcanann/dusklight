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
