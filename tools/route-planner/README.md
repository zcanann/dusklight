# Twilight Princess Route Planner Runtime

This is the independent application boundary for the causal route planner. It
does not register commands with Huntctl and does not use the TAS timeline
workbench's graph or playback schemas.

The Rust planner engine lives in `crates/engine`. This tool owns its schemas,
CLI, reports, input contracts, and future server/editor protocol. It has no Rust
dependency on Huntctl/TAS crates. Existing producers can exchange compatible,
content-addressed data at the wire boundary; any future TAS consumption of the
planner must be initiated downstream by that project.

```text
cargo run --manifest-path tools/route-planner/Cargo.toml -- help
```

The planner CLI currently owns ten operations:

- `compose` validates deterministic refinement-pack stacks and emits a canonical
  composed fact/mechanics catalog. Authored obstruction selectors bind to
  concrete actions during composition, producing solver/graph dependencies
  without route-book wiring.
- `edit-route-book` applies an atomic, expected-digest-checked batch of typed
  route-book edits and emits a fully revalidated canonical revision.
- `extract-world` converts generic canonical world artifacts into conservative
  planner facts and unresolved physical obligations.
- `inspect-state` exposes every live and serialized component store alongside
  exact-context friendly aliases and derived fact evaluations.
- `project-graph` emits a planner-native causal graph with typed relations and
  collapsible predicate regions and optional route-book plan regions; it does
  not use TAS timeline graph schemas.
- `state-from-snapshot` materializes an executable planner state.
- `validate-route-book` checks a route book's goals, predicates, action
  references, nested regions, methods, directives, and annotations against an
  exact base or composed catalog without adding mechanics.
- `solve` runs bounded causal reachability against planner-owned catalogs and
  can apply an optional route book's scoped constraints, action directives,
  ordered conditioned methods, cost/evidence thresholds, and deterministic soft
  preferences. Reached steps retain obstruction/resolver/obligation choices;
  failed searches retain a deterministic closest blocker witness per transition.
- `solve-portable` expands a route book's exact/equivalent context scope,
  requires one explicit start state per exact context, solves each context
  independently, and reports whether the route reaches its goal everywhere.
- `serve-stdio` exposes typed validate/compose/project/solve requests as JSON
  lines for a future planner editor or other clients, including portable solves.
