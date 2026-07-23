# Route observation validation

`dusklight.route-planner.route-observation-validation-report/v1` verifies each
digest-bound route observation against its paired planner snapshots.

```sh
tools/route-planner/target/debug/route-planner validate-route-observations \
  --catalog composed-catalog.json \
  --route-book route.json \
  --matches matched-observations.json \
  --snapshot before.json \
  --snapshot after.json \
  --output validation.json
```

Use repeated `--equivalence-set` inputs when a route intentionally relies on a
proved exact-context equivalence. The default evidence policy admits only
established facts; `--research` also evaluates contested and hypothetical
facts. The policy and every equivalence-set digest remain in the report.

For each observation window, the validator evaluates the action's intrinsic
precondition and the route step's optional authored precondition on the before
snapshot. It evaluates microtrace and authored postconditions on the after
snapshot without converting unknown results to false.

The validator then applies the action's typed operations to a private clone of
the before snapshot. It compares the modeled and observed environments after
removing component provenance only: provenance says where evidence came from,
not whether payload, binding, lifetime, or serialization ownership survived.
Every component in the union of before, modeled, and observed states gets
independent modeled/observed dispositions and semantic-state digests. This
separates expected writes from unexpected changes to components the model said
to preserve.

Operations that require backing stores unavailable in a snapshot are reported
as `model_replay_status: unavailable` with the exact engine error. They do not
become passes. Canonical validation binds the report to the catalog, route book,
match report, snapshot census, evidence policy, and equivalence evidence and
recomputes all component summary lists.
