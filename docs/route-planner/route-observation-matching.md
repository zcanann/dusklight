# Route observation matching

`dusklight.route-planner.route-observation-match-report/v1` binds planned route
steps to authenticated gameplay windows without making the planner depend on a
particular runner or trace decoder.

An author first supplies a canonical
`planned-edge-observation-manifest/v1`. Its artifact table gives every trace and
optional input tape a stable ID and SHA-256 digest. Each observation names one
route step, its before/after planner snapshot digests, an ordered simulation-tick
window, and—when a tape is present—the exact ordered tape-frame window.

```sh
tools/route-planner/target/debug/route-planner match-route-observations \
  --catalog composed-catalog.json \
  --route-book route.json \
  --manifest observations.json \
  --snapshot before.json \
  --snapshot after.json \
  --output matched-observations.json
```

The command validates the route against the exact composed catalog, resolves
every referenced snapshot by content digest, and rejects reversed sequences,
cross-content pairs, missing trace/tape artifacts, incomplete tape boundaries,
and dangling step IDs. Its report retains the catalog and route-book digests,
the complete supplied snapshot census, and one row for every planned step.
Unobserved steps remain explicit as `observed: false`; a step may retain several
independent windows, so later evidence does not overwrite an earlier witness.

The artifact deliberately records matching separately from semantic
verification. Postcondition and preservation validation consume these exact
digest-bound pairs in the next proof stage.
