# Message-flow programs

Status: planner-owned compiler, exact-resource program-set construction, and
stage/actor entry attachment implemented; additional event-handler and cleanup
caller audits remain open.

## Boundary

`orig_extraction::ExtractedMessageFlow` is the immutable result of decoding a
retail BMG `FLW1`/`FLI1` pair. It records nodes, label entry points, branch
targets, raw dispatch indices, resolved generic handler numbers, and typed
temporary/persistent/switch accesses. It does not guess which runtime component
owns those bits.

`message_flow::MessageFlowProgram` supplies that missing, version-scoped
context:

- the exact resource digest and exact runtime/content scope;
- the active message-flow component;
- bindings for temporary, persistent, and switch backing stores;
- the byte/word layout of each switch store;
- separately evidenced contracts for other event handlers; and
- separately evidenced cleanup callers.

The program is canonical JSON and content-addressed. Compilation produces a
canonical `CompiledMessageFlowProgram` containing ordinary mechanics
transitions/readers, friendly raw aliases, label entry points, and explicit
unresolved nodes. Nothing depends on Huntctl or the TAS workbench.

## Graph compilation

Each known node receives its own flow-node guard. Message and event nodes follow
their encoded direct successor; `0xffff` becomes a stable terminal node. A
branch node produces two distinct `BranchFlow` transitions using the two
entries beginning at its encoded branch-target-table index.

For the audited `true when clear` handlers, branch outcome 1 requires the raw
bit to equal zero and branch outcome 0 requires the selected bit to be set. The
compiled reader references the same backing store as the guard, so backward
relevance can find writers without a hand-authored `text_displacement` Boolean.

Unknown node types are retained in `unresolved_nodes` and receive no invented
successor. Unknown query handlers expose both encoded outcomes only behind an
explicit unknown requirement. Unsupported event handlers similarly retain an
unknown requirement rather than silently becoming no-ops.

## Backing stores

Temporary accesses use their session/runtime binding. Persistent accesses can
use `active_runtime_file`, so the same imported handler follows save/load or
file-0 projection without naming a fixture-specific file ID. Loaded-stage
switches can use `current_stage`; room-local stores can use `current_room` or an
exact zone binding when that is what the audited handler selects.

Packed flag coordinates contain the byte offset in the high byte and a
single-bit mask in the low byte. Switch bindings describe:

- component kind and live binding reference;
- byte offset of the bit-array storage;
- bytes per word;
- whether bytes are reversed within each word; and
- the number of addressable switches.

This represents retail big-endian `u32` switch arrays without pretending every
backing is a linear byte array. For example, loaded-stage switch `0x0c` with
base `0x08`, four-byte words, and reversed word bytes resolves to byte `0x0a`,
mask `0x10`.

All generated writes use `WriteBoundRaw`. They therefore resolve the active
backing at execution time and inherit the engine's unique-component,
knownness, range, and atomicity checks.

## Event handoffs

Generic flag handlers (`event000/001`, `event010/011`, and `event014/015`) are
compiled directly from extracted typed accesses. A different handler is not
decidable merely because its event number and parameters were decoded.

`event009` is also compiled when its nonzero parameter names an exact FLI1 flow
label in the selected resource. That transition advances directly to the label
and suppresses the event node's encoded successor, matching the retail
`eventNodeProc` override. Parameter zero performs a runtime Midna/current-room
message-group selection and therefore remains an explicit unknown until that
selection is modeled. The source-inert `event012`, `event019`, and `event042`
handlers follow their encoded successor without inventing door, actor, or other
state writes.

`MessageEventContract` supplies the exact ordered operations for one such node.
It can model an item grant, a pending-item handoff, a recent-item copy, or a
source-audited flow jump using the same generic state operations as every other
mechanic. An encoded-successor contract cannot also write the flow component.
A contract-controlled handler must contain exactly one explicit flow operation
for this program, preventing an authored jump from being overwritten by an
automatic successor.

This is the intended seam for Auru-style item state: the message handler does
not receive a special `Auru duplication` capability. Its contract reads or
copies the actual pending/recent-item component, while physical interruption
and actor-trigger reachability remain independent obligations or obstructions.

## Cleanup

Cleanup is an edge, not a boundary default. Each `MessageCleanupEdge` has its
own transition identity, caller-specific activation predicate, exact sorted
packed coordinates, and evidence. An unconditional `true`/`false` activation
is rejected.

Consequently, central event completion and Ooccoo cleanup can clear the same ten
temporary bits while remaining distinct causal operations. A room load, void,
or title return does not inherit either cleanup unless its own audited rule says
so.

## Fail-closed validation

The compiler rejects:

- count, node-index, direct-target, label, or branch-table mismatches;
- typed accesses that disagree with the referenced handler or parameters;
- multi-bit packed coordinates where a single flag is required;
- missing or duplicate backing-store bindings;
- switch indices outside the authored store layout;
- contracts attached to already-decoded generic handlers;
- ambiguous/double flow continuation in event contracts; and
- unconditional, duplicate, or malformed cleanup edges.

Known friendly names compile into raw aliases over the same dynamic backing
references. Unknown coordinates remain explicit unknown requirements; they are
not assigned guessed offsets.

## Exact resource-set construction

`MessageFlowImportProfile` is the versioned seam between immutable extraction
and mutable state. It names one exact content digest, maps runtime language tags
to extracted locale bundles, supplies the active flow component and backing
layouts, and carries evidence for those mappings. The extractor does not infer
a locale from a product ID or invent a backing from a handler name.

`construct-message-flows` accepts a canonical extracted-orig bundle, runtime
configuration, and import profile. It selects the runtime language's locale
bundle and emits a canonical `message-flow-program-set/v2` with one exact-scope
program per message group. Construction fails closed when the language is not
mapped, the selected bundle is absent, a group is ambiguous, or a group exceeds
the runtime width. An extracted temporary, persistent, or switch access whose
backing is not yet audited still compiles its control-flow edge, but the
predicate or write is retained as an explicit unknown requirement; it never
becomes a no-op or guessed store.

Native observations expose both friendly label-indexed flag arrays and exact
register bytes. These are intentionally different component kinds. The friendly
arrays answer “what did each known label read as?” but are not writable backing
stores; `WriteBoundRaw` targets the unique 256-byte temporary event-register
component or the exact 32-byte loaded-stage-memory component. Persistent
event-register bytes are not yet exposed by the native observation contract, so
a profile must omit that binding until the raw store is audited and captured.

Bindings may also be projected from structured live state. A projected binding
names one component plus typed fields that produce a stage, room, zone, dungeon,
runtime-file, actor, session, or custom binding. This is the generic seam for
speaker-relative message handlers: an actor-entry contract can record the
speaker's resolved stage/zone in `message-session`, and every zone-switch reader
or writer then follows those fields. A missing component, unknown payload,
wrongly typed field, or out-of-range numeric value resolves to unknown; it never
falls back to the player's current room. Backward relevance conservatively
connects a projected store only to compatible binding classes.

Generated programs intentionally have no event contracts or cleanup edges.
Those operations depend on source-audited handlers and callers, not on the BMG
graph alone. Adding them later does not alter the extracted graph or the
profile's storage semantics.

Stage and actor callers are attached through the separate exact-content entry
contract compiler documented in
[`message-entry-contracts.md`](message-entry-contracts.md). Its compiled output
is supplied to `compose` alongside the exact message-flow set; entry mechanics
cannot silently attach to another build or language resource.

The bundled GZ2E01 English profile intentionally maps only the backing stores
that are source-audited and uniquely representable today. Persistent event
registers, dungeon-session switches, zone switches, and one-zone switches
remain unknown rather than being conflated with friendly observations,
current-stage, or current-room state. See
`gz2e01-message-import-profile.md` for the exact boundary and extraction audit.

`MessageFlowResourceOverlaySet` supplies those later contracts without editing
the generated base. Each overlay is pinned to both the import-profile digest
and the exact resource digest. A cleanup edge must retain a caller-specific
condition and established/contested evidence; an unconditional or unknown
cleanup cannot enter the exact compiled set.

`CompiledMessageFlowSet` compiles every selected resource and deterministically
merges its aliases, readers, and transitions. Duplicate IDs or invalid
cross-catalog references fail validation. `merge_into` validates a clone before
replacing caller catalogs, so a conflict cannot leave a partial merge.

```text
route-planner construct-message-flows \
  --bundle extracted-orig.json \
  --runtime-configuration runtime.json \
  --profile message-import-profile.json \
  --output message-programs.json

route-planner compile-message-flows \
  --bundle extracted-orig.json \
  --runtime-configuration runtime.json \
  --profile message-import-profile.json \
  --overlays audited-resource-overlays.json \
  --output compiled-message-programs.json \
  --manifest compiled-message-programs.manifest.json

route-planner compile-message-entries \
  --bundle extracted-orig.json \
  --message-flow-set compiled-message-programs.json \
  --contracts audited-message-entries.json \
  --output compiled-message-entries.json \
  --manifest compiled-message-entries.manifest.json
```

The compiled command uses the normal fact-pack manifest contract. It records
the selected message archives, import profile, optional overlay set, extraction
schema, exact content, coverage limits, and payload digest. The payload retains
each source program beside its compiled artifact, so it can be recompiled and
validated after the original `orig/` tree is removed.

`compose --message-flow-set compiled-message-programs.json --message-entry-set
compiled-message-entries.json` merges the generated aliases, readers, flow
transitions, and entry transitions into cloned base catalogs before applying
ordinary refinement layers. The exact message-flow dependency and context must
accompany each entry set. Any missing dependency, context mismatch, duplicate
ID, or broken cross-reference rejects the merge without mutating either base
catalog.

Entry contracts are deliberately separate from the BMG-derived graph. They
join an exact stage/STAG message group, optional exact actor placement, and flow
label, while preserving interaction geometry and other unproved activation as
obligations or unknown requirements. See `message-entry-contracts.md`.

## Remaining import work

The v2 compiler and constructor establish the state/control representation,
but production fact-pack integration still needs to:

1. publish audited import profiles and stage/actor entry-contract packs for
   additional exact builds and language mappings;
2. audit additional generic item, pending-operation, event-request, and
   cut handoff handlers;
3. emit central event and Ooccoo cleanup callers from their actual predicates;
4. compare semantic flow differences across builds and languages.
