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

The planner CLI currently owns twelve operations:

- `compose` validates deterministic layered refinement stacks and emits a
  canonical composed fact/mechanics catalog. `--pack`, `--route-overlay`, and
  `--what-if-overlay` are distinct precedence domains; disposable overlays
  cannot silently become installed knowledge. Authored obstruction selectors
  bind to concrete actions during composition, producing solver/graph
  dependencies without route-book wiring.
- `edit-route-book` applies an atomic, expected-digest-checked batch of typed
  route-book edits and emits a fully revalidated canonical revision.
- `diff-state` compares two executable states across a named boundary, retaining
  raw/component deltas and recomputed friendly-fact deltas. Binding-only changes
  are identified separately from payload changes, so identical bytes receiving
  a new semantic interpretation remain visible.
- `extract-world` converts generic canonical world artifacts into conservative
  planner facts and unresolved physical obligations.
- `inspect-state` exposes every live and serialized component store alongside
  exact-context friendly aliases and derived fact evaluations. It also retains
  ordered operation/boundary history, reports the last known writer of each live
  structured field, and groups set/clear history for every observed write gate.
- `project-graph` emits a planner-native causal graph with typed relations and
  collapsible predicate regions and optional route-book plan regions; it does
  not use TAS timeline graph schemas.
- `project-feasibility-diff` evaluates each candidate at one exact executable
  state and emits only edges whose permissive upper-bound and modeled results
  differ, including obstruction, obligation, and microtrace proof details.
- `state-from-snapshot` materializes an executable planner state.
- `validate-route-book` checks a route book's goals, predicates, action
  references, nested regions, methods, directives, and annotations against an
  exact base or composed catalog without adding mechanics.
- `solve` runs bounded causal reachability against planner-owned catalogs and
  can apply an optional route book's scoped constraints, action directives,
  ordered conditioned methods, cost/evidence thresholds, and deterministic soft
  preferences. Reached steps retain obstruction/resolver/obligation choices;
  failed searches retain a deterministic closest blocker witness per transition.
  Predicate-shaped physical obligations are recomputed from each propagated
  state, so a state write can unlock a transition without a named route shortcut.
  Physical obligations can derive required/excluded box, sphere, or cylinder
  membership; loaded-actor state; directed region connectivity; plane sidedness;
  player rotation, action, and control; and evidence-scoped temporal microtraces.
  Absent actors, geometry, or timing witnesses remain unknown. Exact matching
  temporal witnesses appear in solve proofs and auto-bind to their obligations
  in the planner graph. Composed solve reports retain each active refinement
  entry's layer, pack ID, digest, and local precedence. Reached steps and blocked
  witnesses embed all contributing rule/fact evidence and report their weakest
  evidence level, so hypothetical support cannot be hidden behind an established
  transition label.
- `solve-portable` expands a route book's exact/equivalent context scope,
  requires one explicit start state per exact context, solves each context
  independently, and reports whether the route reaches its goal everywhere.
- `serve-stdio` exposes typed validate/compose/project/solve requests as JSON
  lines for a future planner editor or other clients, including portable solves.
