# Message-flow entry contracts

A decoded BMG flow label proves that a message program has an entry point. It
does not prove which actor or cutscene can invoke it, where that caller exists,
or whether the player can activate it. Message-entry contracts provide that
separate, exact-content join.

## Source contract

A `message-flow-entry-contract-set/v2` pins both the schema and digest of one
`compiled-message-flow-set/v4`. Each entry then names:

- the exact message group, message-resource digest, and flow ID;
- the exact source stage, optional room and layer, and stage archive/resource
  digest whose STAG record selects the message group;
- either a non-actor caller, or an actor instance ID plus an exact placement
  resource and record (archive path/resource digest, `chunk_tag`, record index,
  layer, actor name, and complete raw bytes);
- the speaker stage, room, and optional resolved zone;
- additional hard guards, physical obligations, explicit unknown requirements,
  and evidence citing both the stage and message resources.

STAG selection and actor placement are independent joins because retail callers
commonly live in a room's `room.dzr` while the message group comes from the
stage's `stage.dzs`. An actor placement is evidence of a possible caller, not
evidence that talking to it is feasible. Interaction volumes, attention
selection, forced cutscene triggers, player control, timing, and other
unaudited conditions remain obligations or unknown requirements.

## Compilation

`compile-message-entries` resolves the contracts against an extracted-orig
bundle and the selected compiled message-flow set. Compilation fails when:

- the exact content identities or compiled-set digest differ;
- the stage path, stage-resource digest, or STAG message group differs;
- the separately pinned actor placement resource is absent or belongs to a
  different stage;
- an actor description matches zero or multiple placement records;
- the message resource or flow label is absent; or
- an obligation is not scoped to the exact selected runtime context.

The output is a canonical
`compiled-message-flow-entry-set/v2`. It embeds the source contracts, resolved
flow component/node/index records, and deterministically reproduced mechanics.
It therefore remains verifiable after the user's `orig/` directory is removed.
Tampering with the mechanics without changing the embedded source derivation is
rejected.

```text
route-planner compile-message-entries \
  --bundle extracted-orig.json \
  --message-flow-set compiled-message-flows.json \
  --contracts message-entries.json \
  --output compiled-message-entries.json \
  --manifest compiled-message-entries.manifest.json
```

Composition requires both artifacts explicitly:

```text
route-planner compose \
  --facts facts.json \
  --mechanics mechanics.json \
  --message-flow-set compiled-message-flows.json \
  --message-entry-set compiled-message-entries.json \
  --output catalog.json
```

The composer rejects an entry set unless its exact compiled message-flow
dependency is also selected. This prevents a valid-looking actor edge from
being attached to the wrong language or revision's flow graph.

## Runtime state

An entry transition writes the exact message group, resource digest, flow ID,
node index, and speaker context into the selected flow component before
advancing to the resolved node. A known speaker zone can then drive a projected
zone-memory binding. If the zone was not audited, the transition invalidates
that field; zone-relative handlers remain unknown and never fall back to the
player's current room.

## Bundled exact entry

`gz2e01-en-lanayru.json` is the first concrete bundled contract set. It pins
F_SP115's group-8 STAG resource, the separate R01 layer-13 `Seirei` placement,
US-English flow 21, and the loaded-stage-memory switch `0x0c` guard. Its actor
and player-control checks are an interaction obligation; the unaudited shared
attention distance, facing, input, and competing-event path remains an explicit
unknown requirement.
