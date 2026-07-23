# Route evidence coverage

`dusklight.route-planner.route-evidence-coverage/v1` reports which semantic facts
a suite of route books depends on and highlights heavily reused facts whose
authored truth is not established.

```sh
tools/route-planner/target/debug/route-planner report-route-evidence-coverage \
  --catalog composed-catalog.json \
  --route-book glitchless.route.json \
  --route-book any-percent.route.json \
  --minimum-route-count 2 \
  --output route-evidence-coverage.json
```

Every route book is independently validated against the exact composed catalog.
The census follows facts referenced by goals, route/region predicates,
constraints, directives, and selected action contracts. Transition guards and
obstructions, obligation predicates, techniques, resolvers, writer gates, and
microtraces contribute their fact dependencies. Derived facts expand recursively
to their upstream aliases and derived facts.

Each used fact retains its definition kind, authored truth, evidence-record IDs,
and the sorted route-book IDs that use it. `weak_high_usage_fact_ids` contains
only contested, hypothetical, or unknown facts used by at least the configured
number of distinct route books. Repeated steps inside one route do not inflate
the cross-route count.

The report retains the composed, fact, and mechanics catalog digests plus every
route-book digest. Inputs and output are canonical; duplicate route IDs or any
route/catalog mismatch fail closed.
