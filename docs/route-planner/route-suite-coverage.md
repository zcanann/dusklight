# Route-suite coverage

`dusklight.route-planner.route-suite-coverage/v1` reports semantic facts and
feasibility obligations exercised by four explicit route-suite classes:
glitchless story, 100%, Any%, and hypothetical research.

```sh
tools/route-planner/target/debug/route-planner report-route-suite-coverage \
  --catalog composed-catalog.json \
  --glitchless glitchless.route.json \
  --hundred-percent completion.route.json \
  --any-percent any-percent.route.json \
  --hypothetical research.route.json \
  --output suite-coverage.json
```

Each supplied route is independently validated against the same exact composed
catalog. Fact usage follows the same goal, predicate, action-contract, and
recursive derived-fact census as route evidence coverage. Obligation usage comes
from authored transition activation contracts and obstructions, technique
discharges/introductions, and the obstruction targeted by authored resolvers.

All four suite rows are always emitted in canonical order. A class with no
supplied route is `reported: false` with empty coverage, rather than being
silently omitted or inferred from another category. Reported rows retain exact
route-book IDs/digests and sorted fact/obligation IDs. The report also binds the
composed, fact, and mechanics catalog digests.
