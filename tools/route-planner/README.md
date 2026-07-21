# Twilight Princess Route Planner Runtime

This is the independent application boundary for the causal route planner. It
does not register commands with Huntctl and does not use the TAS timeline
workbench's graph or playback schemas.

The Rust planner engine lives in `crates/engine`. This tool owns its schemas,
CLI, reports, and future server/editor protocol. Low-level read-only world and
evidence contracts are consumed as inputs; Huntctl does not depend on or expose
planner behavior.

```text
cargo run --manifest-path tools/route-planner/Cargo.toml -- help
```

The planner CLI currently owns four artifact operations:

- `compose` validates deterministic refinement-pack stacks and emits a canonical
  composed fact/mechanics catalog.
- `extract-world` converts generic canonical world artifacts into conservative
  planner facts and unresolved physical obligations.
- `state-from-snapshot` materializes an executable planner state.
- `solve` runs bounded causal reachability against planner-owned catalogs.
