# Witness promotion

`promote-witnessed-actions` converts fully verified observation windows into a
normal refinement pack. It supports transitions, techniques, resolvers,
writers, and microtraces.

```sh
tools/route-planner/target/debug/route-planner promote-witnessed-actions \
  --catalog composed-catalog.json \
  --validation validation.json \
  --request promotion-request.json \
  --output witnessed.pack.json \
  --receipt witnessed.receipt.json
```

The canonical `witness-promotion-request/v1` supplies pack metadata and one or
more sorted action promotions. Each promotion names its evidence-replacement
rule, its same-ID action rule, and one or more validation-window/evidence-ID
pairs. The generator rejects observations unless:

- the requested action and authored precondition evaluated true;
- every authored or intrinsic postcondition is verified (or none was authored);
- typed operation replay succeeded;
- the modeled and observed snapshot environments match; and
- component preservation is verified with no mismatched component.

Promotion changes only the action's evidence envelope. The generated action
retains the same ID, scope, precondition, operations, costs, obligations, and
other mechanics. Its evidence keeps every existing record, adds one
`route_witnessed` record per selected observation, and becomes established. A
paired `replace` rule swaps that evidence-enriched record into the catalog.
Consequently ID-bound obstructions, readers, route books, and directives keep
their meaning; no duplicate action can bypass them.

Every other action remains untouched. The canonical
`witness-promotion-receipt/v1` binds the source catalog, validation report, and
generated pack digests; records preserved and added evidence IDs; and retains
identical before/after action-ID censuses. Dependencies are generated from the
source catalog's exact refinement stack, so the pack cannot silently apply over
a different set of lower-confidence source layers.
