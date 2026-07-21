# Twilight Princess Route Planner Runtime

This is the independent application boundary for the causal route planner. It
does not register commands with Huntctl and does not use the TAS timeline
workbench's graph or playback schemas.

The Rust planner engine currently lives in
`../huntctl/crates/route-planner` while the underlying evidence/world contracts
are being stabilized. That is a build-time dependency only: this tool owns its
CLI, reports, and future server/editor protocol.

```text
cargo run --manifest-path tools/route-planner/Cargo.toml -- help
```
