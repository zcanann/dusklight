# Extraction coverage reports

`dusklight.route-planner.extraction-coverage-report/v1` combines one or more
sealed fact-pack manifests into an exact-content coverage view.

```sh
tools/route-planner/target/debug/route-planner report-extraction-coverage \
  --manifest world.manifest.json \
  --manifest messages.manifest.json \
  --output coverage.json
```

Reports group manifests by their exact `ContentIdentity` digest and retain each
manifest ID and canonical digest. Every context always contains a fixed ordered
census of topology, actor placements, collision, hard guards, storage bindings,
message flows, actor lifecycle, physical feasibility, and techniques.

Coverage contributions retain the originating manifest, stable scope, status,
and detail. Counts distinguish complete, partial, and unavailable scopes.
`reported: false` means no supplied manifest made any claim for that domain; it
does not mean unavailable, incomplete, or equivalent to another context. A
complete topology scope therefore cannot silently imply complete guards,
backing stores, actor lifecycle, or physical feasibility.

Inputs must be canonical, individually valid fact-pack manifests. Output is
canonical and content-addressed; duplicate manifest IDs within one exact context
are rejected.
