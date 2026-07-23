# Twilight Princess route planner: architecture and task plan

## 1. Mission

Build a version-aware route-planning and theorycrafting system for Twilight
Princess that can answer:

- Can a target state be reached from this exact starting state?
- Which known routes reach it?
- Why does a candidate route fail?
- Which requirement, obstruction, or missing technique is responsible?
- What becomes reachable if a proposed exploit or state transfer is assumed?
- Which facts are proven by game data, observed in a run, supplied by community
  knowledge, or merely hypothetical?

The planner must scale in both directions:

- A casual view can say `obtain Fishing Rod` or `beat the game normally`.
- A route author can expand that node into interchangeable subroutes.
- A researcher can inspect individual flags, memory-bank bindings, collision
  obstructions, wrong-state respawns, cross-file operations, and speculative
  bypasses.

This is a causal state-transition model, not a checklist of tricks and not a
hand-authored list of opportunities that each trick loses.

The first useful product is deliberately narrower than a complete game solver:
an author can assemble known transition providers, see the exact state after
every step, and receive a precise missing producer or obstruction resolution
when two steps do not compose. Full-game route books, exhaustive extraction,
and broad visual polish are later benchmarks, not prerequisites for making this
core loop useful.

The primary result of a query is one of:

1. `reachable`: the active model contains a complete feasible route.
2. `unreachable_under_model`: the search space is sufficiently closed to prove
   that no route exists under the selected rules and knowledge packs.
3. `unknown`: a route depends on unresolved geometry, incomplete behavior, an
   unaudited state transformation, or an intentionally open research question.

“No known route” must not silently become “impossible.”

---

### Current implementation status

The independent Rust backend is real and substantial: typed execution state,
component lifetimes and provenance, transition composition, obstructions and
techniques, backward relevance plus forward reachability, graph projection,
route-book mutation, state inspection, content extraction, and a typed stdio
service all exist. Selected validation cases exercise Text Displacement, Auru's
recent-item mechanism, Fanadi return-place locking, dungeon-key semantics,
Faron-twilight returns, and hypothetical component rebinding.

The first independent browser boundary now exists. A loopback-only Rust host
serves an embedded blueprint canvas and the typed planner service; the client can
open a typed project, project its authoritative graph, search transitions,
pan/zoom/fit, drag layout nodes, inspect planner-owned payloads, and export a copy
with presentation positions. A selected transition can now be assessed against
an exact project start state by the Rust evaluator; executable effects return a
typed after-state, while rejected evaluations retain their guard, obligation,
and scope classification. The browser cannot yet insert that validated step into
a propagated route, save route semantics authoritatively, or load all planned
demonstrations. Coverage is also incomplete, so the current engine is a
causal-reasoning laboratory over selected modeled mechanics rather than a
whole-game route explorer.

Current Windows health is green. Planner-owned canonical JSON and source-evidence
files are forced to LF checkout, and the workspace declares both crates as
default members. The normal planner test command now runs the engine rather than
hiding it behind a green wrapper-only suite: 233 engine tests, seven runtime
library tests, one runtime binary test, and both doc-test targets pass.

---

## 2. Acceptance stories

These stories define the architecture more usefully than a catalogue of data
types. Every implementation phase should preserve them.

### 2.1 Ordinary play

Given a fresh file, the planner can produce a normal route to a chosen milestone,
show inventory and important flags at any step, and collapse routine quest chains
into friendly nodes.

### 2.2 Fishing Rod as a replaceable subgoal

The goal `own Fishing Rod` can be satisfied by at least these plan shapes:

- Talk to the vine NPC, climb, use the hawk, obtain the cradle, return it to Uli,
  and receive the rod.
- Use a chicken to bypass the vine interaction, use the chicken to go out of
  bounds, acquire the cradle carry state, return to Uli, reload as required, and
  receive the rod.
- Mix compatible steps from those routes, such as chicken bypass followed by the
  ordinary hawk step.
- Use the session-level recent-presentation item mechanism plus Auru's broken
  grant path to obtain a rod prepared on another file. The causal machinery is
  present in the SD code; the currently known way to activate the broken path
  depends on HD's longer targeting range.

The collapsed result is simply `obtain Fishing Rod`. Expanding it reveals the
selected proof and its alternatives. The rod is not marked as “lost” by Back in
Time; it becomes unreachable only if all currently enabled producers of the rod
are unreachable.

### 2.3 Back in Time and file 0

Back in Time enters a title-origin, memory-backed runtime file commonly described
as `file 0`. It is not a fourth physical save slot and cannot be saved back under
the file-0 identity, although its serializable state can be written to slots 1–3.

File 0 nevertheless contains ordinary save-domain storage: inventory, equipment,
progression, and miscellaneous flags initialized by the title/BiT path. Those
values persist and propagate like other file state for as long as that runtime
file remains active. Its lack of card-slot backing must not make its contents
temporary flags or hand-authored effects of the BiT technique.

The model must support a route whose current state remains on that slotless
runtime file indefinitely, including a hypothetical future escape that could make
beating the game on file 0 possible. Current known continuations include:

- Save to physical slot 1, 2, or 3 after the required void/title-state handling,
  thereby projecting file-0 state into a persistent slot and ending that file-0
  lifetime.
- Use Back in Time Equipped to carry a pending King Bulblin respawn/equipped
  state into an existing file, again ending the source lifetime while preserving
  only the components that the trick actually transfers.

Saving must therefore be modeled as serialization and runtime-file reattachment,
not as “file 0 becomes slot 0.”

### 2.4 Early Master Sword and layered feasibility

An upper-bound logic pass may conclude that the master sword can be obtained and
that Hyrule Castle or Ganon is logically authorized. A real route may still be
blocked by wolf gates, twilight, collision, void planes, actors, missing movement
capabilities, or an unusable scene transition.

Known and hypothetical options may include:

- The standard Early Master Sword setup with Hero's Clothes and charge attack.
- An Epona out-of-bounds route, with its non-twilight requirement.
- A long rupee-clip path that replaces the charge-attack obligation when paired
  with the required Epona setup.

The solver must explain which obligation each technique discharges and must not
confuse a game-state permission with a physically traversable route.

### 2.5 Nominally local flags transferred to another level

The ordinary model treats the live stage-memory payload as bound to its current
stage and serializes it to that stage's saved bank. With no enabled transfer,
Forest Temple local state behaves as local to Forest Temple.

A theorycraft overlay can introduce a typed transformation that preserves that
raw payload while rebinding it to Temple of Time. The planner then:

- Keeps the bytes unchanged.
- Changes the binding and therefore the friendly interpretation of the bits.
- Re-evaluates all rules that consume Temple of Time's interpretation.
- Retains component-level provenance showing where the payload came from.
- Marks the route hypothetical unless the transformation has evidence.

Deleting the overlay restores the ordinary result. No flag is permanently
declared “untransferable”; instead the base knowledge says that no known transfer
edge exists.

### 2.6 Wrong-state respawn and BiTE

A wrong-flags or wrong-state respawn is a component splice, not a whole-file
teleport. A route may keep inventory and progression from one runtime file, a pending
respawn or equipped state from another, and location/binding from the transition
destination.

Back in Time Equipped should be the first fully modeled example. Other glitches
can reuse the same preservation/clear/rebind operators rather than adding a new
special-case state format.

### 2.7 Fanadi save-location locking

The solver must be able to discover and explain a route with this shape:

1. A save-location actor writes `PlayerReturnPlace`.
2. The player reaches the setup required to interact with Fanadi, including
   Ooccoo where the known glitch requires it.
3. Fanadi enables `NO_TELOP`, which prevents subsequent SavMem writes while the
   gate remains active.
4. The held return-place value survives locations that would normally overwrite
   it.
5. Savewarp reads the held value and enters a normally unexpected map/room/point.

Route order matters. Reaching Fanadi may itself pass writers that replace an
earlier value, commonly leaving a Castle Town-area return point. If a future
bypass reaches Fanadi without those writes, adding that bypass must automatically
make earlier held values available; the Fanadi rule itself should not change.

### 2.8 Theorycraft overlays

A user can enable, disable, add, or propose:

- A new technique that satisfies an existing obstruction.
- A typed state-component transfer.
- A candidate map transition or spawn.
- A changed assumption about collision or an actor.
- A deliberately stronger “assume this obstruction absent” rule.

The base data remains intact. Results identify which overlay assumptions they
depend on and distinguish an evidenced bypass from simply deleting an obstacle.

### 2.9 Auru recent-item grant: shared mechanism, build-specific reachability

The planner models the event controller's most recently queued presentation-item
ID as session/process state independent of the active save file. Obtaining or
showing an applicable item writes that store; changing files preserves it under
the observed lifetime; Auru's broken grant path reads it through the generic
get-item machinery and applies the selected item's ordinary grant semantics.

The broken grant is surfaced as a candidate on SD even though no known SD route
can activate it. Its spatial activation obligation is approximately:

```text
find a reachable player pose that is
  inside Auru's talk/target volume
  outside the normal cutscene-trigger volume
  while player input and the required dialogue state are available
```

HD's longer targeting range is a known resolver. SD begins as obstructed when the
non-overlap/reachability claim is evidenced, or `feasibility_unknown` while the
trigger and interaction geometry remain unaudited. A proposed SD clip, actor
displacement, trigger unload, or interaction-range change can discharge the same
obligation without redefining the item mechanism.

### 2.10 Text Displacement and Goron Mines

Text Displacement is represented as concrete temporary message-progress bits,
message-flow control state, cleanup behavior, and interruption edges—not a
`text_displacement = true` technique flag.

The Goron Mines entrance story must compose causally:

1. One of several dialogue/interruption routes produces the required temporary
   bit pattern.
2. Gor Coron's flow reads that pattern and follows a displaced branch.
3. That branch writes the actual event/switch state used by the entrance actors.
4. Invisible-wall, elevator, and NPC actor state update or reconstruct according
   to their own rules.
5. Any remaining live NPC/collision obstruction must still be resolved, perhaps
   by movement or a room reload.
6. Only then does the encoded Goron Mines transition become executable.

The solver can work backward from the consumer's raw bit predicate to every known
producer, including Coro, Auru, Yeta, Ooccoo, and hypothetical interruption
routes, while preserving their distinct spatial and timing requirements.

### 2.11 Exact content and language-dependent routes

Given a recognized disc/data extraction, the planner resolves an exact content
identity from metadata and digests rather than trusting a user-entered label such
as `PAL`. The selected message language and other mutable configuration remain
part of the execution environment because one disc can contain several resource
variants and a route may be able to change the active selection.

Generated facts for that identity can be cached and distributed as a derived fact
pack where licensing permits, so ordinary planner users do not necessarily need
an `orig/` directory. Supplying `orig/` is an opt-in path for reproducible
extraction, verification, unsupported revisions, and new research.

A comparison of PAL language variants should expose the commonly reported French
cannon-payment divergence through the extracted message-flow graphs and their
actual reads, writes, and branches. If the French flow reaches the launch path
without the ordinary 300-rupee guard or debit, the solver derives that cheaper
route; it does not consume a hand-authored `french_cannon_skip` Boolean. Any
timing, positioning, or interruption requirement that data alone cannot prove is
still an explicit obstruction or unknown obligation.

### 2.12 Reachability composition and subgraph encapsulation

Given a state at a known place, the author can insert any transition provider
whose entry contract is satisfied there. Providers include physical travel,
doors and loading zones, cutscene/event changes, savewarp, game over, void-out,
Ooccoo warp, wolf warp, title/file operations, and evidenced or hypothetical
techniques. Applying one produces a new typed state; it never merely draws a line
between place names.

Selecting any node in an active query or authored plan shows the complete state
at that point and its diff from the preceding node. If the next desired provider
does not apply, the attempted join remains invalid and exposes one or more of:

- an unsatisfied state prerequisite and the known transitions that can produce it;
- an active physical/interaction obstruction and its known resolvers;
- an unknown activation obligation requiring evidence or a refinement; or
- an exact-context mismatch for which no valid provider exists.

The author connects the route by inserting a real producer, resolver, or explicit
hypothetical refinement. The editor must not offer an untyped force-connect edge.

There is one reachability graph. `Beat Lakebed Temple in one trip`, `Beat Lakebed
Temple in multiple trips`, and `Complete the east circuit` are ordinary subgraphs
someone can construct from its states and transitions. They are not child
implementations of a semantic `Beat Lakebed` operation.

A user may name, collapse, nest, save, copy, or reference a selected region of
the graph purely for encapsulation and complexity management. Grouping does not
create a new transition, invent pre/postconditions, or alter reachability. When a
saved region is placed after a different state, its enclosed transitions are
reevaluated normally and any invalid boundary remains visible.

---

## 3. Core semantic laws

These are non-negotiable design constraints.

### 3.1 Never author derived losses

Do not encode:

```text
BackInTime loses FishingRodOpportunity
```

Encode only causal changes. If Back in Time changes origin, active runtime file,
scene, form, inventory, or flags, ordinary access to the rod quest disappears
because the solver can no longer reach its producers. If Auru's recent-item grant
becomes feasible through the known HD setup or a hypothetical SD resolver, the
rod becomes reachable again through a different producer without editing the
Back in Time definition.

### 3.2 Tricks are transitions, not flags

A technique has prerequisites, operations, postconditions, evidence, version
scope, cost, and reliability. It is never represented as a permanent Boolean like
`did_bit = true` unless the game itself retains a corresponding state.

### 3.3 Physical storage, logical binding, and semantic meaning differ

“Forest Temple flag 7” is a friendly interpretation, not the underlying storage
identity. Model:

```text
component payload + component kind + current binding + build = interpreted facts
```

The same payload can acquire different meaning after a rebind. “Local” describes
normal reachability and ownership, not an immutable property of the bytes.

### 3.4 State has component-level provenance

After cross-file, wrong-respawn, or duplication tricks, one state may contain
components originating from different files, scenes, or moments. Provenance must
attach to components and writes, not only to the entire snapshot.

### 3.5 Writes and reads are ordered operations

Important values such as return place are not just facts. They have candidate
writers, guards, write suppression, clearing, serialization, and consumers.
A lock preserves the previous value; it does not conjure the desired value.

### 3.6 Game authorization and physical feasibility are separate

Scene-change data and flag checks produce candidate transitions. Geometry,
actors, voids, forms, movement, and other physical conditions determine whether
the transition can actually be activated.

### 3.7 Obstructions are first-class and directional

An obstruction says why a particular approach cannot currently complete. A
bypass resolves that obstruction for a stated approach and scope. It does not
erase the underlying world fact for every route.

Authored obstructions bind declaratively to candidate actions. An author may
name a stable action ID, or select an action structurally by action kind,
source location, destination scene, approach, and exact-context scope. Its
`active_when` predicate narrows the obstruction to the relevant runtime states.
Catalog composition resolves that selector to concrete candidate-action
IDs and emits ordinary `blocks` dependencies automatically. The solver never
depends on a route author remembering to wire the same obstruction into a route
book.

This behaves like a typed rewrite of traversability:

```text
authorized(action, state)
  -> authorized(action, state)
     AND every active bound obstruction has an applicable resolution
```

`active_when` remains a state predicate, so one physical edge may be blocked in
twilight or from one side and open otherwise. Exact-one selectors fail on zero
or multiple matches; explicitly plural selectors expand deterministically and
retain the authored selector and source provenance. Changing the extracted
catalog reruns binding and changes the composed-catalog digest, preventing stale
bindings from silently surviving a build update.

### 3.8 Evidence and truth are separate

A fact can be extracted from a build, observed in a trace, reported by a route
pack, or proposed in a what-if overlay. Its truth status and provenance must be
visible and queryable.

### 3.9 Exact content identities are the primitive

Rules target concrete platform/region/revision content identities plus any
relevant runtime configuration predicate. Friendly groups such as `GCN`, `Wii`,
or `HD` expand to exact identities; language is selected independently where the
content permits it. Shared Auru item-state mechanics may apply broadly while the
long-range activation setup remains HD-specific. Context scope belongs on each
fact, obstruction, and resolver rather than only on the friendly trick name.

### 3.10 Collapse preserves proofs; it does not merge states

Two routes that both display as `obtain Fishing Rod` may leave different flags,
positions, carry states, timers, or inventory. They can share a display node only
when the continuation does not care about their differences. Otherwise the UI
must expose the branch or retain separate hidden frontier states.

### 3.11 User assumptions are overlays

Refinements and what-if rules compose on top of immutable base facts. Removing
an obstruction, satisfying it with a technique, and bypassing its approach are
three different operations.

### 3.12 Extracted destinations are inert until activated

A map reference, SCLS destination, or door destination is not itself a traversable
edge. It is a candidate whose activation contract may include actor existence,
interaction side, key or item checks, switches, cutscene state, collision access,
and transition-trigger execution. The solver may traverse it only after every
known hard guard and physical obligation is satisfied; unaudited obligations make
the candidate unknown rather than implicitly usable.

### 3.13 Obstructions apply to state-producing actions

An obstruction may guard talking to an actor, avoiding a trigger, advancing one
message node, interrupting cleanup, performing a one-frame input, or reloading an
actor—not only walking through a final map transition. A route cannot manufacture
the required state merely because a downstream rule exists.

### 3.14 Temporal control flow is state

Message node, cutscene phase, pending cleanup, accepted input window, and player
control are modeled at the granularity needed by a technique. Normal flow and an
interrupted flow are alternative ordered transitions with different resulting
stores. A verified microtrace may summarize frame-level behavior, but it must
declare its exact pre-state, timing obligation, preserved state, and post-state.

### 3.15 Exceptional cutscene flow preserves only executed writes

A cutscene-driven scene change is an ordered control-flow program, not one
atomic milestone effect. Resource or actor-archive load failure, actor
corruption, skip logic, interruption, and fallback branches may execute a prefix
of its writes and omit a suffix. The resulting state must preserve every write
known to have executed, retain prior values where a known writer was skipped,
and mark unaudited branch effects unknown rather than choosing “all normal
effects” or “no effects.”

This also prevents misleading shortcut edges. If actor corruption skips the
post-Zelda tower sequence and the normal return-place overwrite does not run,
Castle Town remains in the return-place backing component. The later Zelda's
Tower-to-Castle-Town movement is then an ordinary save-warp reader consuming
that retained value—not a special authored warp. The exceptional cutscene edge
only explains the resource-load outcome, flow phase, scene change, flags that
did execute, and writers/cleanup that were bypassed.

### 3.16 Universality must be demonstrated

Raw offsets, actor parameters, map records, message node IDs, and flow graphs are
facts about exact content, not universal truths. Stable semantic names may bind
to different raw representations in different builds or selected languages.

A rule can be promoted to a shared build group only after equivalence is
established across a declared set of exact identities. Observing one build leaves
the others unsupported, not implicitly equal. Implementations may store common
bases plus regional, revision, and language deltas, but queries always resolve a
complete exact context before search.

### 3.17 Connections are state transformations, not topology annotations

A place is context in a state, not a node that can be connected freely to another
place. Every graph edge is an executable transition with an entry predicate and
typed effects. Location-changing providers include normal
movement and loading zones as well as savewarp, game over, void-out, Ooccoo,
wolf, event, cutscene, title, and technique-defined transitions.

When consecutive providers do not compose, the gap is data. A missing hard
prerequisite requires a state-producing transition; an active obstruction
requires a scoped resolver or alternate approach; an unknown obligation requires
evidence or an explicitly hypothetical refinement. None may be repaired by an
unvalidated visual edge.

The state before and after every edge is immutable and inspectable. A collapsed
or nested subgraph hides visual detail only: its edges remain ordinary transitions
and its boundary states remain the same states in the reachability graph.

---

## 4. Repository audit and reusable foundations

### 4.1 Existing assets

The repository already contains useful foundations:

- World extraction for placements, spawns, SCLS scene changes, and collision.
- A read-only milestone model.
- Save, stage, event, actor, and title-flow source needed to ground mechanics.
- Existing TAS route/editor output may serve as non-authoritative examples and
  visual references, but is not an implementation foundation or prerequisite.

The planner owns every production contract and tool it needs. Source knowledge
and extracted facts become its knowledge base through planner-owned adapters;
concrete routes remain planner route books. Existing UI may provide visual
precedent for nested plan regions, but its timeline tree, playback authority,
and graph schema are not the planner's causal model. No planner milestone waits
on Huntctl becoming stable or usable. A generic primitive may be extracted later
only after a clean domain-independent boundary is demonstrated and without
making it a planner dependency.

### 4.2 Source-grounded save/runtime findings

The current code shows:

- Physical save UI/card slots are 1–3; title-origin “file 0” is a slotless,
  memory-backed runtime file with real save-domain contents, not another card
  slot.
- `dSv_info_c` owns persistent save data plus live current-stage, dungeon, zone,
  temporary, restart, inventory, and related state.
- `dSv_info_c::getSave(stage)` copies a selected per-stage payload into the live
  `mMemory` bank.
- `dSv_info_c::putSave(stage)` copies the live bank back into that stage's saved
  entry.
- Planner `StageBank` owners are keyed by both runtime-file identity and stage;
  two files can therefore hold different payloads for the same stage without
  aliasing. The executable `commit_load_stage_bank` operation verifies the
  active runtime, current scene, exact source/destination owners, semantic
  bindings, and stage-load lifetime before atomically committing and restoring.
- Planner persistent-file images preserve explicitly selected runtime components
  plus nested stage-bank stores. `save_runtime_to_slot` seals that projection in
  physical slot 1–3; `load_runtime_from_slot` verifies the complete manifest,
  retires the active runtime, restores a fresh card-backed runtime, and preserves
  unrelated session-owned state. A separate, explicit, disjoint carry manifest
  can rekey selected runtime-lifetime metadata into the destination without
  pretending it came from the card image; omitted runtime metadata is removed.
  Initial `getSave(stage)` activation and scene location change remain separate
  ordered operations.
- The same per-stage `dSv_memory_c`/`dSv_memBit_c` payload contains chest bits,
  switches, item bits, a small-key count, and dungeon-item bits including the boss
  key. Key and boss-key semantics therefore derive from the bound backing store,
  not from the provenance of the pickup that supplied them.
- The SavMem actor writes `PlayerReturnPlace` when its event/switch guards pass.
- SavMem returns before writing while the temporary `NO_TELOP` bit is active.
- Fanadi code sets and later clears the same `NO_TELOP` bit.
- Game start/savewarp consumes `PlayerReturnPlace` to choose the return stage,
  room, and point.
- `dEvt_control_c::mGtItm` stores the most recently queued presentation/get-item
  ID. Present-demo and chest-demo creation overwrite it; ordinary event completion
  and removal clear `mPreItemNo` but do not clear `mGtItm`.
- Link's get-item initialization consumes `mGtItm` when the demo parameter uses
  the generic `0x100` item selector. This is the shared causal core of Auru
  duplication even though this codebase does not contain an executable TPHD
  build.
- Message-flow event `010` writes parameterized temporary flags and event `011`
  clears them. Event cleanup clears the shared message-progress subset on only
  particular completion paths, while NPC actors may also clear individual bits.
- The decompilation enumerates platform/region/revision builds separately, while
  runtime disc detection currently recognizes `GZ2E` and `GZ2P`; a friendly disc
  code alone therefore does not establish every revision or extracted-data fact.
- Existing world-context generation already requires a game-data SHA-256, giving
  the planner a content-addressed foundation for reproducible derived facts.
- PAL code paths also read the selected PAL language at runtime. Region and
  language must therefore be separate axes even when many rules are shared.
- Auru's actor separates proximity behavior, a pending presentation actor ID,
  normal memo creation, and generic `DEFAULT_GETITEM` execution. These are
  distinct state sites and ordered actions.
- Native learning observation v14 records `mDataNum` and `mNoFile` as
  separate raw values, a separately statused backing attachment, and three
  distinct physical-slot descriptors. It does not infer a slotless file from a
  nonzero `mNoFile`, because the PC command-line load path overloads that field.
- The same v14 channel records exact `PlayerReturnPlace`, restart state,
  `mPreItemNo`, `mGtItm`, item-partner identity, event-control flags, exact
  running-event name when bounded, `NO_TELOP`, and player-control state. Generic
  message-flow/node/cut and pending-cleanup discovery remain explicitly scoped:
  v14 can read the active flow and node for the observed Auru and Goron-child
  NPC families without advancing dialogue, records the cut independently as
  `Unavailable`, and leaves pending cleanup `Unavailable` until its ownership is
  established. Other actor families remain `Unavailable` rather than being
  cast to a guessed layout.
- V14 also recognizes loaded SavMem (`KYTAG14`) actors and records their decoded
  return-stage target (current stage plus configured room/point), event-set,
  event-unset, switch-set, and switch-unset selectors, the room binding used for
  the switch reads, `NO_TELOP` gate state, each evaluated predicate, and the
  conjunction that makes the actor eligible to write at that boundary. This is
  actor-instance evidence, separate from the independently observed held
  `PlayerReturnPlace`; an eligible writer is not fabricated into a state change.
- Physical slot contents remain `NotSampled`: the live process exposes the
  active runtime and selected slot number, not trustworthy simultaneous payloads
  for all three card slots. A future card-boundary observer must populate those
  descriptors without copying the active runtime into them.
- Planner snapshot schema v2 losslessly projects v14 native observations into
  independently bound runtime, stage, room/zone, temporary, inventory, restart,
  return-place, and event-handoff components. Every projected component carries
  trace provenance; raw banks carry byte-knownness masks; unavailable structured
  channels retain their capture status instead of receiving zero values.
- SavMem observations project into room-lifetime live-world objects and
  actor-bound components with trace provenance. The target, raw predicate
  selectors, evaluated guard values, and eligibility remain distinct fields so
  later source/extracted rules can connect them to their real backing stores.
- Snapshot schema v8 keeps observed slot descriptors separate from verified
  serialized slot contents, permits unknown runtime origin/backing and
  player-control state, retains ended/suspended runtime-file lifetimes, and
  diffs those lifetimes plus slot observation/content changes independently.
- Native snapshot sequences accept an explicit incoming boundary kind, emit
  semantic/component/raw-byte diffs, and seal each snapshot into a contiguous
  digest-linked chain. This makes room/stage/save/load/void/title/BiT/BiTE test
  captures comparable without inferring the boundary label from coincidental
  state changes; representative captures for each boundary remain outstanding.
- Extracted world-facts schema v7 now compiles an exact content identity,
  runtime configuration, authenticated `WorldContext`, and its complete set of
  canonical world inventories into a content-addressed planner payload. The
  planner-owned `route-planner extract-world` command emits both that payload
  and a sealed fact-pack manifest. The planner owns the compatible input
  contracts, their validation, and the import implementation; it does not link
  Huntctl crates or register planner behavior there.
- The compiler imports recognized static placements and player spawns with raw
  records and source bindings. Every SCLS destination remains an encoded-exit
  fact. An SCLS record with no collision activation join does not become a
  transition; each collision/SCLS join becomes a separate exact-context
  upper-bound candidate with a typed scene-location effect, an unresolved
  physical approach obligation, and an explicit unknown while the collision
  activation semantics remain inferred.
- For the exact audited GZ2E01 fingerprint, the same importer recognizes the
  `fpcNm_L1BOSS_DOOR_e` name family and joins each usable-side placement's
  decoded exit index to one same-room SCLS record. The candidate reads the boss
  key from current-stage dungeon memory, writes the decoded memory-switch bit,
  and retains interaction and actor/event/collision phases as unknown
  obligations. It does not generalize this source behavior to lookalike builds
  or other boss-door families.
- Refinement-pack schema v14, refinement-stack schema v2, and composed-catalog
  schema v15 now live entirely in the planner workspace. `route-planner compose`
  validates canonical packs, dependency digests, conflicts, deterministic
  layer/pack precedence, explicit
  replacement/disable operations, and all resulting cross-references before it
  emits a canonical catalog. The output seals the base fact/mechanics digests
  and the ordered pack stack, so removal of a pack can recompute consequences
  instead of relying on handwritten `loses` lists.
- Composition can add obligations, obstructions, resolvers, techniques,
  writers/gates/readers, reconstruction rules, witnessed microtraces, goals,
  aliases, and derived facts. What-if component transforms compile to ordinary
  techniques; writer suppression compiles to an ordinary gate; assume-absent
  compiles to an explicitly hypothetical resolver. `route-planner solve` can
  consume the composed artifact directly and records its active refinement
  stack in the solve report.
- Planner graph schema v7 is an independent, canonical projection of fact and
  mechanics catalogs. It exposes typed fact, goal, transition, obligation,
  obstruction, resolver, technique, writer/gate/reader, reconstruction, and
  microtrace nodes with causal edge kinds. Every nested predicate is projected
  as an ordered `all`/`any`/`not`/fact/comparison tree inside its own collapsible
  region, so the editor can summarize requirements without flattening or losing
  their interchangeability. The planner-owned `route-planner project-graph`
  command emits this artifact from either base or composed catalogs.
- Mechanics-catalog schema v15 makes writer, gate, and reader records executable
  solver inputs rather than graph-only annotations. Reader proofs retain their
  exact raw source value and optional friendly interpretation; an unresolved or
  evidence-disallowed source makes its consuming transition unknown instead of
  inventing a default. The same schema distinguishes portal, void/death reload,
  title return, wrong-state respawn, and actor-driven transitions and provides
  typed operations for form, mount, control, and action changes.
- Message-flow-program and compiled-message-flow-program schemas v1 turn an
  exact extracted `FLW1`/`FLI1` graph plus explicit backing layouts into ordinary
  message-action transitions, raw readers/writers, label entry points, and
  friendly aliases. Known generic temporary/persistent/switch handlers compile
  exactly; unknown node types have no invented successor, unsupported handlers
  retain unknown requirements, and separately evidenced event/cleanup contracts
  can express item handoffs, controlled jumps, and caller-specific cleanup.
  Exact-language program-set construction, digest-pinned overlays, sealed
  compiled fact packs, and transactional base-catalog merge are implemented;
  audited production profiles, actor entry contracts, and further handler
  audits remain open. See
  `docs/route-planner/message-flow-programs.md`.
- Planner service schema v28 provides a typed JSON-lines transport owned by the
  standalone planner runtime. `route-planner serve-stdio` accepts refinement and
  route-book validation/editing, catalog composition, graph projection, state
  inspection, exact-context solve, and portable multi-context solve requests;
  every response retains its request ID and
  returns either a typed payload or a structured field/detail error. It imports
  no Huntctl CLI, TAS timeline, WorkbenchGraph, playback, or browser-state types.
- Feasibility-graph-diff schema v1 evaluates the same catalog twice at one exact
  executable state: permissive authorization and modeled feasibility. It emits
  only differing or physically annotated transitions, retaining hard guards,
  obstruction IDs, discharged/unknown obligations, and temporal witnesses. The
  planner-owned `project-feasibility-diff` command and service expose it without
  changing the base graph or route book.
- State-inspection schema v10 preserves the full execution-state document—live
  components, serialized owner stores, bindings, lifetimes, provenance, gates,
  cleanup, runtime-file identity, physical slots, execution process, retained
  world location, pending world load, and player state—
  while evaluating every friendly alias and derived fact under the selected
  exact context and evidence policy. `route-planner inspect-state` and the
  service protocol expose the same projection, so raw inventory/flag bytes,
  their semantic names, ordered mutations, last field writers, and gate history
  remain inspectable together.
- State-inspection-diff schema v11 combines the raw/component boundary diff with
  before/after friendly fact evaluations. It classifies binding-only changes,
  payload changes, direct derived-fact dependency changes, relevant gate reads,
  runtime-context changes, serialized owner-store changes, and sealed persistent-
  file image changes separately. When the active runtime ends, it additionally
  derives every source-owned live component/store fate, unchanged or changed
  outside-lifetime components, and preserved or changed physical images from the
  two concrete states. An unchanged payload digest and empty raw-byte delta
  therefore remain visible when rebind alone changes an alias; common-prefix and
  divergent history suffixes identify exactly which ordered operations separate
  two execution states.
  The standalone `diff-state` command and service expose the same report.
- Route-book schema v5 is a validated, exact-context-scoped preference layer over
  mechanics. It can name goals and path constraints, reference ordered actions,
  define alternative methods and nested plan regions, request pin/ban/prefer
  behavior, and attach non-semantic annotations. It deliberately has no effects
  or loss fields: every referenced action and predicate must validate against
  the fact/mechanics catalog. `route-planner validate-route-book` and the typed
  service validate books without composing them into mechanics.
- Planner graph schema v7 can optionally project a route book as distinct plan
  region, method, and reference-step nodes connected to the underlying causal
  actions. Region outcomes and step pre/postconditions remain nested predicate
  graphs. A book's collapse policy is surfaced, but the catalog projection does
  not mark a region collapsed before a solver proof establishes continuation
  equivalence or supplies residual-state differences.
- Route-book edit-batch schema v5 provides revision-checked authoritative
  mutations. Each atomic batch names the expected book digest and can update
  goals, constraints, directives, steps, methods, regions, selection/collapse
  policy, and annotations. Rust applies edits to a clone, sorts canonical sets,
  revalidates all graph references and per-step/per-method context scopes, and
  emits a new canonical revision only if the entire batch succeeds.
- The bounded forward solver now consumes active, exact-context route-book
  require/forbid predicates and techniques plus pin/ban transition, resolver,
  and technique directives. Required actions are part of search identity, so a
  state reached without a pinned action does not erase a later compliant path.
  Planner CLI/service solve requests accept an optional route book and seal its
  digest into solve-report schema v8. Standalone writer rules are now ordinary
  searchable actions: activation and all bound gates are reevaluated at each
  state, their typed operation executes transactionally, and reached or blocked
  proofs retain writer/gate evidence. Microtrace execution outside an exact
  temporal obligation still fails closed rather than being silently invented.
- Selected and pinned route-book methods now compile to ordered action
  subsequences; banned methods prune a path when their ordered subsequence is
  completed. Sequence progress participates in search identity, so revisiting
  identical game state at a different point in a method remains distinct.
  Method/action contradictions are rejected before search.
- Method step pre/postconditions are evaluated at their actual action boundary.
  Resolver and technique setup operations execute separately before the target
  transition, preserving intermediate states instead of treating setup as one
  opaque mutation. Unknown hard method conditions produce `unknown`, and an
  uncertain banned-method match cannot be reported as a known reachable route.
- Solve-report schema v8 retains active/unknown obstructions, selected
  resolvers/techniques, discharged/outstanding/introduced obligations, and
  semantic state identities on successful steps. Failed searches keep the
  deterministic closest witnessed blocker state for each transition, including
  guard truth and unknown requirements. Failed writer actions likewise retain
  activation, active/unknown gates, state identity, and weakest evidence;
  successful transition steps retain every in-scope reader's exact source value
  and interpretation, and missing/disallowed readers fail unknown. Portable
  solve-report schema v7 embeds those per-context proofs. Each result also
  embeds its backward-relevance proof and says whether it pruned forward
  actions.
- Predicate-backed feasibility obligations derive satisfied, unsatisfied, or
  unknown status from the exact propagated snapshot and evidence policy. Search
  re-evaluates them after state-producing actions, so an ordinary state write can
  open a later transition without naming that obligation in a technique.
  Interaction obligations evaluate evidenced box, sphere, and vertical-cylinder
  required/excluded volumes against player position and combine them with player
  rotation, action, control, form, mount, or other pose predicates. The addressed
  actor must be a loaded live instance. Directed region observations derive
  geometry; plane equations derive exact sidedness/void results. Missing actors
  or spatial observations remain unknown.
- Temporal requirements name an action plus an allowed frame/input window.
  Applicable evidenced microtraces must contain a matching interrupt operation
  whose witnessed window is contained by the requirement. Missing or evidence-
  disallowed witnesses remain unknown, and supporting microtrace IDs survive in
  reached and blocked solver proofs. Matching microtraces also auto-bind to the
  obligation as graph `demonstrates` dependencies.
- Mechanics-catalog schema v15 includes explicit cutscene scene-change and resource-
  load-failure transition classes, the reload/warp/actor transition classes above,
  player-state operations, and masked raw-knownness invalidation. This supports
  partial execution records that preserve confirmed prefix bytes while marking
  only unaudited suffix effects unknown; extracting concrete cutscene phase
  programs remains open.
- Soft action and method preferences use deterministic lexicographic search:
  minimize action depth first, then maximize total preference weight among
  equal-depth routes. Each directive contributes at most once, preference and
  method progress participate in search identity, and the result reports both
  the score and satisfied directive IDs; loops therefore cannot farm weight.
- `CostAtMost` constraints accumulate every executed technique's authored
  `RouteCost` axes, retain the totals in search identity, prune paths exceeding
  the strictest active per-axis maximum, and report the reached route's totals.
  Transitions and resolvers currently have no cost field in mechanics schema v15,
  so no unmodeled cost is invented for them.
- `EvidenceAtLeast` accepts only `established`, `contested`, or `hypothetical`
  and intersects that threshold with—never relaxes—the runtime evidence mode.
  The effective policy is used for transitions, techniques, resolvers,
  obstructions, facts, and conditioned route steps, and the result records the
  active minimum.

Primary source anchors:

- `include/d/d_save.h`: save object layout and bank accessors.
- `src/d/d_save.cpp`: `getSave`, `putSave`, card serialization, and return-place
  handling.
- `src/d/actor/d_a_kytag14.cpp`: guarded SavMem writer and `NO_TELOP` early return.
- `src/d/actor/d_a_npc_shaman.cpp`: Fanadi's `NO_TELOP` set/clear behavior.
- `include/d/d_save_temp_bit_labels.inc`: `NO_TELOP = 0x1301`.
- `src/d/d_s_play.cpp` and `src/d/d_menu_save.cpp`: title/file/save lifecycle.
- `include/d/d_event.h`, `src/d/d_event.cpp`, and
  `src/f_op/f_op_actor_mng.cpp`: `mGtItm`, its writers, event cleanup, and session
  lifetime.
- `src/d/actor/d_a_alink_demo.inc`: generic get-item consumption of `mGtItm`.
- `src/d/d_msg_flow.cpp`: message nodes that write and clear temporary flags.
- `src/d/actor/d_a_npc_rafrel.cpp`: Auru proximity, talk, message-flow, pending
  presentation actor, and get-item handoff.
- `include/global.h`, `include/dusk/version.hpp`, and `src/dusk/version.cpp`:
  compile-time build catalogue and current runtime disc detection.
- `tools/route-planner/src/main.rs`: planner-owned world-fact extraction and
  manifest construction.
- `tools/route-planner/crates/engine/src/world_data.rs`: planner-owned,
  wire-compatible world-context and world-inventory input contracts.
- `src/d/d_file_sel_info.cpp`: an example of PAL behavior depending on selected
  language in addition to region.

This evidence establishes the generic storage and write-gate mechanics. The exact
player-facing exploit sequence, duration, version support, and Ooccoo setup still
need trace or route evidence and should not be inferred solely from these writes.

### 4.3 Audit gaps

Before claiming completeness, inventory:

- Every state bank and its actual lifetime/reset/serialization behavior.
- Stage-to-bank indices and build-specific flag aliases.
- Small-key pickup, delayed award, consumption, and door-unlock writes for every
  relevant door/gate actor family.
- Every writer and reader of return place, restart place, last stage, room, layer,
  and spawn point.
- All SavMem actor placements and their event/switch guards.
- Normal stage load/commit order and failure/void/reload paths.
- BiT/BiTE preservation and clearing behavior per build.
- Known wrong-flags respawns beyond BiTE.
- Exact Auru broken-flow branch, `mGtItm` initialization/lifetime per executable
  build, normal memo overwrite point, and the HD versus SD interaction/trigger
  geometry. Treat imported HD documentation as external mechanics evidence, not
  as an executable HD build supplied by this repository.
- Message-flow asset format, node graph, branch predicates, temporary-bit
  writes/clears, terminal cleanup, and actor-specific flow selection.
- Exact disc/executable/resource digests, available language bundles, language
  selection lifetime, and semantic equivalence across supported revisions.
- The PAL French cannon-payment flow versus other PAL language flows: exact graph
  divergence, rupee guard/debit behavior, entry conditions, and whether any
  remaining exploit-specific obligation exists.
- Known dialogue interruption windows, input/control constraints, and which
  cleanup operations each interruption avoids.
- Goron Mines entrance actor placements, invisible-wall/elevator/NPC control
  state, and the exact raw text-bit predicates and downstream writes used by Gor
  Coron's displaced branch.
- Twilight-dependent transition suppression and form/mount restrictions.
- Actor reconstruction rules: which behavior comes from placement parameters,
  persisted switches/items, live actor fields, or room/scene lifecycle.

---

## 5. Architecture

### 5.1 Artifact layers

Keep five artifacts separate:

1. **Build facts** — extracted maps, actors, SCLS, collision, item placements,
   flag labels, and source-audited mechanics.
2. **Knowledge packs** — techniques, obstructions, semantic aliases, route costs,
   and community-verified behavior.
3. **Route books** — curated normal, category, historical, or showcase plans.
4. **Refinement overlays** — versioned community or route-local corrections and
   hypothetical transformations.
5. **Query configuration** — start state, goal, allowed techniques, risk/cost
   policy, evidence threshold, and active overlays.

Route books consume the knowledge base but do not define reachability truth.

#### 5.1.1 Content identity and generated fact packs

Separate immutable input identity from mutable runtime selection:

```text
ContentIdentity
  platform / region / revision
  disc and executable identifiers
  executable digest
  game-data/resource-manifest digest

FactPackIdentity
  content identity
  extractor and schema versions
  output digest
  coverage manifest

ExecutionContext
  content identity
  selected language and configuration
  active runtime state
```

Detection reads disc metadata and hashes the relevant inputs. A user-supplied
version name is a selection hint that must agree with those observations, not an
authority that overrides them. An unknown digest either requires a local
extraction/audit or remains unsupported; it must never silently inherit a nearby
build's facts.

The extractor consumes user-provided `orig/` data and emits deterministic,
content-addressed derived facts. The planner can consume those generated packs
without access to the original game assets when the pack's contents may be
redistributed. Packs record provenance down to the source artifact where
practical, distinguish unavailable extraction coverage from false facts, and can
be compared structurally across builds and languages. Original assets remain
outside the pack and repository.

The system may compact storage as `common base + exact deltas`, but this is an
implementation detail. A solver query sees one fully resolved context. A
portable-route query uses only semantic rules proven equivalent across its
declared contexts and validates the resulting plan independently in every one;
it is not a union of convenient rules.

### 5.2 Execution environment

A search node is an execution environment, not merely a set of progression flags:

```text
ExecutionEnvironment
  content identity and selected language/configuration
  active runtime file
  persistent slot set
  current scene / room / layer / spawn
  player form, mount, pose, and movement state
  state components
  instantiated actors and other live world objects
  pending operations and respawns
  active write gates / latches
  session/title state
  proof and component provenance
```

Runtime-file identity and backing:

```text
RuntimeFile
  runtime_id
  origin: TitleFile0 | NewFile | LoadedSlot(slot_id) | Other
  backing: MemoryOnly | CardBacked(slot_id)
  allowed_serialization_targets
  lifecycle
```

Do not name a memory-backed runtime `slot 0` in the type system. The UI may show
“file 0” as community terminology. Also do not equate `MemoryOnly` with temporary
game state: the runtime may contain persistent-domain inventory and flags and may
remain active indefinitely.

Keep these axes independent:

- **Semantic lifetime:** frame, room, stage, session, runtime file, or serialized
  save.
- **Residency:** live memory, card image, extracted trace, or symbolic value.
- **Backing attachment:** no slot or physical slot 1–3.
- **Serialization policy:** what can be written, to which target, and with what
  normalization.
- **Runtime identity:** which currently executing file the state belongs to.

### 5.3 State components

Represent state as typed components with independent lifetimes:

```text
StateComponent
  component_kind
  payload
  binding
  lifetime
  serialization_owner
  provenance
  interpretation_catalog
```

Candidate component kinds include:

- Inventory and equipment.
- Global persistent event/progression bits.
- Stored per-stage memory entries.
- Live current-stage memory bank.
- Dungeon state.
- Zone and room switches.
- Temporary/event-runtime flags.
- Session event-control state such as the most recent presentation item.
- Current message-flow node, branch state, pending cleanup, and dialogue/cutscene
  phase where route-relevant.
- Player return place and restart place.
- Scene, room, layer, spawn, form, and mount.
- Pending respawn, transition, cutscene, or actor state.
- Title/file selection state.

Do not flatten world objects into persistent flags. Keep four related layers:

```text
static placement definition
+ selected room/layer
+ persistent control state
+ transient actor-instance state
= current world behavior and collision
```

For example, a gate's placement and parameters identify its actor behavior; a
stored switch may say it was permanently opened; the live instance may currently
be unloaded, animating, displaced, or collision-disabled. A normal reload
reconstructs the instance from the first three inputs. Actor unload and wrong-state
techniques operate on the live layer unless they explicitly write a backing store.

Candidate lifetimes include:

- Frame/action.
- Room load.
- Stage load.
- Session.
- Runtime-file identity and backing attachment.
- Save serialization.
- Physical slot.

Lifetime and storage are empirical properties with evidence. A component may be
normally cleared on a boundary yet preserved by a technique-specific transform.

### 5.4 Binding and interpretation

The live stage bank has one storage identity and a current binding, for example:

```text
payload: bytes[0x20]
binding: Stage(ForestTemple)
```

Friendly facts are views:

```text
interpret(content_identity, runtime_config, component_kind, binding, offset, mask)
  -> FriendlyFact
```

After a hypothetical rebind:

```text
payload: unchanged bytes[0x20]
binding: Stage(TempleOfTime)
```

Forest Temple aliases cease to be the active interpretation and Temple of Time
aliases become active. Raw offsets remain inspectable, including unknown bits.

Bindings may be stage-, room-, zone-, dungeon-, runtime-file-, or actor-contextual.
Aliases must be exact-content and language aware and may be incomplete or
contested. Language selection is modeled as state when the game can change it,
not frozen permanently into the build label.

### 5.5 Component transformations and state splices

All ordinary and glitched transitions use a small operation vocabulary:

- `write(component, field, value)`
- `clear(component | field)`
- `initialize(component, policy)`
- `copy(source, destination)`
- `move(source, destination)`
- `preserve(component)` across a boundary that normally changes it
- `serialize(component, owner)`
- `restore(owner, component)`
- `bind(component, context)`
- `rebind(component, new_context)` while retaining payload
- `project(source_runtime_file, destination_runtime_file, component_set)`
- `consume(pending_operation)`
- `set_gate(gate)` / `clear_gate(gate)`
- `advance(flow, node)` / `branch(flow, edge)`
- `schedule(cleanup)` / `cancel_or_avoid(cleanup)`
- `interrupt(action, temporal_window)`

A transition declares its behavior per component. Unmentioned behavior is not
implicitly preserved: it is inherited from a named boundary policy or marked
unknown. This prevents accidental whole-state copying.

Example boundary policies:

- Normal room transition.
- Normal stage transition.
- Void/reload.
- Savewarp.
- Title return.
- Load physical slot.
- Save slotless runtime file to a physical slot.
- BiTE wrong-respawn transfer.
- File load that preserves session event-control state.
- Interrupted dialogue/cutscene microtransition.

This operation language is also the safe boundary for theorycrafting. Users may
propose typed transfers without being given an unconstrained “write arbitrary
memory” primitive unless an explicit research mode requests it.

### 5.6 Save as projection, serialization, and rebind

Saving from file 0 should be described as:

1. Select destination physical slot 1–3.
2. Project the serializable component set from the active, memory-backed runtime
   file.
3. Apply save-time normalization/clearing rules.
4. Serialize into the chosen persistent file.
5. Clear the no-file/title-origin state as observed.
6. Reattach or reload the active runtime as a card-backed file according to the
   actual route.

The source runtime identity and destination card file have different backing and
potentially different identity/lifetime rules.
Any component that does not serialize is lost because of the projection policy,
not because an authored route says it is lost.

Back in Time itself should therefore primarily select/enter the title-origin
runtime file. Facts such as Hero's Clothes, an Ordon weapon, or miscellaneous
progress flags derive from that file's initialized backing stores. The exact
initial inventory and flag image is a build-specific evidence task; it is not an
effect list copied onto every BiT route.

### 5.7 Writer, gate, latch, and reader semantics

For state whose history matters, define mechanisms rather than final facts:

```text
WriterRule
  target
  value expression
  activation location/event
  guards
  priority/order
  evidence

GateRule
  blocked operations
  active predicate
  set/clear mechanisms
  lifetime

ReaderRule
  source
  consuming transition
  interpretation
```

Fanadi save locking then emerges from ordinary rules:

- SavMem is a writer of `PlayerReturnPlace`.
- Its event and switch checks are guards.
- `NO_TELOP` gates that write.
- Fanadi is one setter/clearer of the gate.
- Savewarp/game start reads the last value that successfully wrote.

The solver must retain the concrete held value in state. A Boolean
`save_location_locked` is acceptable only as a derived display fact, never as a
replacement for the gate and value.

This machinery also supports last-writer analysis, actor unload effects, delayed
writes, and hypothetical ways to reach a gate setter without crossing a writer.

### 5.8 Ground facts, derived facts, and friendly names

Ground facts are directly stored or observed:

- Raw bank bits and values.
- Inventory slots and counts.
- Scene/room/layer/spawn.
- Carrier and physical slot identities.
- Pending operations.
- Gate state.

Derived facts are rules over ground facts:

- `has_hero_clothes`
- `can_charge_attack`
- `faron_in_twilight`
- `save_location_write_suppressed`
- `ordinary_ordon_rod_quest_reachable`

The fact catalogue maps raw locations to friendly names, descriptions, exact
content/runtime scope, confidence, and citations. Unknown raw bits remain
addressable. Friendly names never replace raw identity in saved data or proofs.

### 5.9 Transition pipeline

Build traversability in layers:

```text
game authorization
  -> candidate transition
  -> approach-specific feasibility obligations
  -> active obstructions
  -> technique/bypass resolutions
  -> executable transition
  -> witnessed transition (optional stronger evidence)
```

Maintain three graph projections:

- **Upper-bound logic graph:** everything permitted by known internal logic,
  ignoring unresolved physical blockers.
- **Modeled-feasible graph:** candidates whose known obligations are discharged.
- **Witnessed graph:** transitions supported by traces, tests, or sufficiently
  strong route evidence.

A transition can therefore be `feasible`, `obstructed`, or
`feasibility_unknown` without corrupting its logical authorization.

The interactive graph is a materialized chain/frontier of these evaluations,
not a place-to-place sketch. Inserting a provider evaluates it against the exact
selected-node state and either:

1. creates the resulting immutable state node and typed state diff; or
2. creates a rejected-join diagnostic naming missing producers, active
   obstructions/resolvers, unknown obligations, and context mismatches.

Every accepted node remains selectable while the graph is being edited or solved.
Inspection is backed by the solver snapshot/proof, not reconstructed from labels.

### 5.10 Candidate transitions

Candidate producers include:

- SCLS exits and doors.
- Spawn/restart/savewarp readers.
- Actor scripts and event transitions.
- Item acquisitions and NPC rewards.
- Cutscene and boss completions.
- Form/mount changes.
- Save/load/title operations.
- Technique-defined wrong warps or component transfers.
- Interaction, message-node, interruption, cleanup, and actor-reload actions that
  produce state required by later transitions.

SCLS data is evidence that a destination is encoded, not proof that Link can
activate it from a given approach.

Each candidate uses an explicit activation contract:

```text
TransitionCandidate
  encoded destination
  activation mechanism / actor
  hard authorization guards
  approach and interaction obligations
  state operations on success
  build and layer scope
  evidence / unknown fields
```

Hard authorization guards are evaluated before the edge becomes executable. A
small-key door that checks a nonzero key count is therefore not traversable merely
because its destination is known. A technique may still avoid the door, reach the
destination trigger another way, or manipulate the guard, but it must name the
specific obligation or alternate activation it supplies.

#### 5.10.1 Dungeon keys as backing-store semantics

Treat keys as ordinary state operations over the bound per-stage store:

```text
small-key pickup
  write live_stage_memory.key_count += 1
  write the pickup/chest persistence bit when applicable

keyed door activation
  require live_stage_memory.key_count > 0
  consume/decrement according to the actor's real behavior
  write the door's persistent unlock state when applicable
  enable or perform the transition

boss-key door activation
  require live_stage_memory.dungeon_items.boss_key
```

The planner does not care which chest or actor originally produced the current
key count. It cares about the value in the store the door actually reads. Normal
stage-bank binding confines keys to their dungeon. A discovered or hypothetical
preserve/rebind/copy bug can expose additional routes automatically, with the
same transfer evidence and provenance rules as any other storage component.

Key count, pickup persistence, door-unlocked state, live door state, and encoded
destination remain separate. Door actor families may implement them differently,
so extraction should preserve actor-specific unknowns rather than inventing one
universal unlock effect.

#### 5.10.2 Interaction and trigger geometry

Represent an interaction obligation as a conjunction over world and player state:

```text
InteractionObligation
  actor instance and interaction mode
  accepted player pose/volume/facing
  excluded trigger or collision volumes
  player-control and form predicate
  current actor/cutscene/message phase
  temporal window where applicable
  exact content/configuration/layer scope
```

The authoring system may initially encode a qualitative relation such as “no
known reachable point lies inside talk range while outside this trigger.” Later
geometry extraction can replace or strengthen it with exact shapes and radii.
Changing target radius, moving either participant, unloading the trigger, or
reaching an unexpected pose are scoped resolvers, not edits to the downstream
dialogue rule.

#### 5.10.3 Message-flow and interruption graph

Compile relevant message/cutscene data into microtransitions:

```text
MessageFlowInstance
  actor and flow identity
  current node / cut
  branch inputs and choices
  temporary-bit reads and writes
  persistent effects
  pending item/event handoffs
  scheduled cleanup
  player-control/input state
```

Each node advance declares its reads and writes. Normal termination executes its
cleanup policy. An interruption edge exits between named operations and preserves
only what that path actually leaves behind. Frame-exact tricks can be represented
as witnessed microtraces with a temporal obligation instead of forcing every
planner query to simulate every frame.

Text Displacement is then the family of states and routes in which shared
message-progress bits survive and are consumed by another flow. Friendly labels
may summarize a useful bit pattern, but raw flags and their producer/consumer
proofs remain authoritative.

#### 5.10.4 Cutscene scene changes and load-failure branches

Represent cutscene transitions as phase-level actions when any intermediate
state can affect a later route:

```text
CutsceneFlowInstance
  event / cut / phase identity
  requested actor and resource archives
  resource-load result: success / failure / unknown
  ordered pre-load and post-load writes
  scene-change request and destination
  return/restart-place writers
  scheduled cleanup and fallback branch
```

Normal completion, intentional skip, interruption, and archive-load failure are
separate candidate edges from the phase where they occur. An actor-corruption
technique may establish a failed-load predicate or corrupted actor/resource
identity; it does not directly grant the downstream map or return place. Source
or witnessed microtrace evidence records the last confirmed operation before
the branch. Native learning observation v26 can now witness complete object and
stage resource-slot occupancy plus mounting, structural readiness, or failure
before a usable archive/resource table exists. It does not bind an archive slot
to a particular cutscene phase and cannot distinguish failures after the table
was allocated, so those planner results remain `unknown` until source or trace
evidence establishes the requested archive and exact failure point.
the branch. When that boundary is unknown, affected flags and cleanup state stay
unknown while unrelated backing components keep their previous values.

Because the event may start from a pending cutscene or actor-load request, this
edge can become enabled at an unusual point in a route. Search identity therefore
includes the flow phase, pending resource requests, and scheduled cleanup—not
only the current room and progression flags.

### 5.11 Obstructions and feasibility obligations

An obstruction definition contains:

```text
id
scope: exact approach / transition / region / build
kind
activation predicate
blocked capability or transition
feasibility obligations
known resolvers
evidence and confidence
```

Kinds may include:

- Collision, wall, gate, ceiling, or ledge.
- Void plane or kill volume.
- Actor/body blocker.
- Twilight barrier.
- Wolf/human/mount restriction.
- Loading-zone reachability or activation direction.
- Camera, animation, or interaction constraint.
- Forced cutscene or state rewrite.
- Unknown geometry/physics.

Obstructions are separate from hard game-state prerequisites such as a door
checking a key flag. The former explain physical realizability; the latter belong
to authorization logic. Some mechanics create both and should expose both layers.

Resolution kinds:

- Satisfy the required ordinary capability.
- Bypass the geometry on that approach.
- Reposition to another side.
- Unload or alter the obstructing actor.
- Avoid the approach through another transition.
- Hypothetically assume the obstruction absent.

### 5.12 Techniques

A technique is a transition provider:

```text
Technique
  id and friendly name
  exact content/configuration scope
  input state predicate
  spatial setup / approach
  component operations
  discharged obligations
  introduced obligations or risks
  cost and reliability metadata
  evidence status
```

Techniques should be authored in a validated typed IR. Sources may include Rust,
generated build data, checked data packs, a DSL, or a visual editor, but all must
compile to the same representation.

Examples:

- Back in Time enters a memory-backed, title-origin runtime file whose initialized
  save-domain stores supply inventory, equipment, and progression facts.
- Back in Time Equipped splices selected pending/equipped/respawn state into a
  loaded persistent file.
- Epona OOB discharges named geometry obligations and requires a non-twilight
  mounted setup.
- A rupee clip discharges a particular wall obligation, not “all walls.”
- Auru's broken grant reads session-level recent-item state. HD's targeting range
  resolves its known spatial setup; the same causal transition remains visible but
  obstructed or unknown on SD until another resolver is supplied.
- A sidehop/backflip dialogue interruption discharges a named one-frame temporal
  obligation and preserves the exact message bits/trigger state observed by its
  microtrace.

### 5.13 Goals and path constraints

Separate end-state goals from path constraints.

Goals:

- Enter Hyrule Castle.
- Defeat Ganon.
- Own Fishing Rod.
- Reach Ordon Spring while Faron remains in twilight.
- Finish on the slotless title-origin runtime file.

Path constraints:

- Never save to a physical slot.
- Preserve Faron twilight.
- Remain on file 0.
- Do not use a named technique family.
- Use only witnessed transitions.
- Permit hypothetical component rebinding.
- Minimize resets, difficulty, or elapsed time.

The same goal can have many constraint-dependent answers.

### 5.14 Solver

The state graph is an AND/OR graph:

- OR: alternative producers for an item, flag, position, or subgoal.
- AND: simultaneous prerequisites for a transition or technique.
- Ordered AND: writer/gate/reader sequences and other history-sensitive setups.

Search operates over concrete-enough environment states, with:

- Exact content identity, runtime configuration, and enabled packs in the cache
  key.
- Component payload/binding/provenance where relevant.
- Dominance checks that preserve continuation-relevant differences.
- Cycle detection across save/load, void, title, and cross-file operations.
- Multi-objective costs such as time, difficulty, resets, evidence quality, and
  hypothesis count.
- Backward relevance analysis from the goal plus forward feasibility from the
  start—the desired “outside-in” behavior.
- Partial proofs when a feasibility obligation is unknown.

The solver must never prune two states solely because their friendly milestone
labels match. It may merge them only after proving equivalence for the remaining
continuation.

### 5.15 Explanations

For each result, retain a proof object capable of answering:

- Which producer satisfied each requirement?
- Which obstruction stopped a candidate?
- Which technique discharged it?
- Which state components were copied, preserved, cleared, or rebound?
- What is the origin of the current value?
- Which last writer established a latched value?
- Which build fact or overlay supplied each edge?
- What minimal missing assumptions would change the result?

Derived lockouts should be presented as cuts in the graph:

```text
Fishing Rod unreachable under this model
  ordinary quest producer: Ordon quest state unreachable
  chicken/cradle producer: required Ordon approach unreachable
  Auru recent-item producer: causal grant exists, but no enabled GCN setup
  resolves talk-without-trigger and interruption obligations
```

That is more useful than `fishing_rod_opportunity = false`.

### 5.16 Route books and graph regions

A route book is a curated selection and annotation layer over the causal graph,
not a second mechanics database or a hierarchy of goal implementations. Ship or
support books such as:

- Normal completion.
- Glitchless story reference routes.
- Versioned glitchless 100% reference routes.
- Standard Any% variants.
- Back in Time research.
- HD cross-file/item-transfer routes.
- Hypothetical/theorycraft showcases.

Plan regions are named selections of states and transitions. They can be nested,
collapsed, copied, and saved to manage complexity, but they never become atomic
solver actions. Alternate ways through a region are simply alternate paths in the
same reachability graph.

```text
Fishing Rod routes
  ordinary cradle path
  chicken/cradle path
  Auru recent-item path
```

```text
Lakebed research
  one-trip subgraph
  multiple-trip subgraph
  east-circuit-only subgraph
```

A saved graph region contains:

- stable references to its enclosed states, transitions, and boundary edges;
- optional labels, annotations, display state, costs, evidence, and assumptions;
- stable identity/versioning so it can be forked, copied, or referenced;
- expansion back to the complete graph detail at any time.

Reusing a region means reusing the enclosed graph fragment, not invoking it as a
macro edge. Every transition is reevaluated from the actual attachment state. A
proof recorded for one placement is evidence, not authorization for another.

The view offers:

- Collapse to outcome.
- Expand selected proof.
- Compare alternatives.
- Pin a method.
- Ban a method.
- Expand only unresolved or hypothetical dependencies.

If alternatives have continuation-relevant differences, the collapsed node shows
them as residual effects or keeps separate frontier badges.

### 5.17 Refinement and theorycraft stack

Apply data in a deterministic stack:

1. Extracted/source-audited base facts.
2. Curated built-in knowledge.
3. Enabled community packs.
4. Route-book constraints/preferences.
5. Route-local overrides.
6. Ephemeral what-if overlay.

Every refinement has an ID, exact content/configuration scope, author/source,
version, confidence, and replacement/conflict policy.

Supported hypothesis operations:

- Propose a bypass that resolves named obligations.
- Add a candidate technique.
- Add a typed component transfer/rebind.
- Add a candidate writer or suppress a writer under a stated predicate.
- Assume an obstruction absent in a precise scope.
- Supersede a disputed fact while retaining both records.

The result UI lists all active assumptions. A hypothetical edge must not become a
silent fact after export or sharing.

### 5.18 UX surfaces

#### Independent planner editor and optional visual precedent

The planner may use screenshots or interaction conventions from the existing TAS
Route Workbench as visual inspiration:

- The same general dark graph-canvas vocabulary, compact node cards, curved
  edges, selection treatment, details pane, and restrained status colors.
- Pan, zoom, fit, breadcrumbs, double-click-to-enter a subgraph, and
  multi-selection/grouping interactions where they retain the same meaning.
- Stable machine IDs beneath editable human labels.
- A projected browser graph backed by authoritative Git-owned/runtime schemas,
  rather than browser state becoming the source of truth.
- Revision-checked edits, explicit previews for consequential mutations, and
  recoverable local drafts where appropriate.

This is optional visual/product continuity, not a requirement to build, run, or
extend the current
`WorkbenchGraph` or timeline schema. The TAS workbench projects a mostly
parent-linked concrete playback history. The planner projects a potentially
cyclic AND/OR causal graph with alternative producers, requirements,
obstructions, hypothetical edges, and multiple continuation-distinct states.
Forcing either domain into the other's schema would erase important semantics.

Ownership and dependency direction are non-negotiable: the planner is its own
tool and has no build-time or runtime dependency on Huntctl/TAS crates. It owns
its engine, schemas, CLI/service, graph projection, and eventual editor. If the
TAS tooling later chooses to consume a stable planner interface, that is a
separate downstream integration decision by its maintainers; this plan neither
implements nor presumes it. Huntctl availability, stability, or completion is
never a planner acceptance criterion.

Give the planner its own versioned schemas and Rust domain crate/server surface.
Rust owns fact-pack resolution, validation, semantic state transitions, solving,
proof construction, graph projection, and authoritative edits. The browser layer
owns rendering and direct manipulation, sending typed commands rather than
reimplementing reachability or state propagation. CSS tokens, layout ideas, and
truly generic graph-camera or selection utilities may be extracted later if that
reduces drift without coupling the models.

#### Route canvas

- Pan/zoom graph with collapsible plan regions.
- Distinct styling for logical, obstructed, feasible, witnessed, and hypothetical
  edges.
- Alternative-producer branches and selected proof.
- Route costs and version badges.
- Familiar Route Workbench navigation, while making AND requirements, OR
  alternatives, obstruction edges, and proof status visually unambiguous.

#### State inspector

At every selected node show:

- Inventory/equipment.
- Friendly global and local flags.
- Raw component bytes/bit positions.
- Current bindings and lifetimes.
- Scene/room/layer/spawn/form/mount.
- Runtime-file identity/backing and physical slots.
- Pending operations.
- Session recent-item value, message-flow node/cut, pending cleanup, player-control
  state, and temporal-window obligations when relevant.
- Return/restart values, last writers, and active gates.
- Bound dungeon key count/items, key provenance, and consumed/unlocked door state.
- Static placement versus persisted control versus live actor-instance state.
- Component provenance and active hypotheses.
- Diff from the previous node.

#### Build and language inspector

- Show detected platform, region, revision, disc/executable/data digests, selected
  language, fact-pack digest, extractor version, and extraction coverage.
- Compare two resolved contexts by semantic rule while retaining their distinct
  raw bindings, message graphs, actors, geometry, and unknowns.
- Warn when a route or refinement uses a fact observed on only one build through
  an over-broad selector.
- Offer exact-context, proven-portable multi-context, and explicit theorycraft
  query modes; never silently fall back to a neighboring build.

#### Requirement/obstruction inspector

- Friendly explanation.
- Raw predicate.
- Producers or resolvers.
- Why each alternative is currently unavailable.
- Evidence and exact context scope.

#### Theorycraft editor

- Clone a route/query into a sandbox.
- Toggle packs and individual assumptions.
- Draw a proposed edge or choose a typed component transform.
- Preview a component under a different binding before enabling it.
- Compare reachability and proofs before/after.
- Export a small reviewable refinement pack.

#### Authoring workflow

1. Pick start and target states.
2. Ask the solver for candidate plans.
3. Pin, ban, or prefer methods.
4. Collapse uninteresting regions.
5. Expand unusual proofs and inspect state diffs.
6. Add a route annotation or create a refinement if mechanics data is missing.
7. Validate exact context scope, evidence, and regressions.

### 5.19 Automation and human refinement boundary

The system should extract or source-derive the factual skeleton wherever
practical:

- Maps, rooms, layers, placements, encoded destinations, and spawns.
- Backing-store layouts and ordinary reads/writes.
- Actor parameters and recognizable hard guards such as key, boss-key, item,
  switch, event, form, or twilight checks.
- Message-flow nodes, branch parameters, temporary-bit operations, item handoffs,
  and normal cleanup where their data formats are understood.
- Pickup effects, key counts, and persistent unlock writes where code/data makes
  them decidable.
- Ordinary actor reconstruction and transition activation mechanisms.
- Collision and approach facts that can be established mechanically.

Human refinements supply knowledge that extraction cannot prove reliably:

- Geometric reachability and approach-specific blockers.
- Interaction-volume versus trigger-volume feasibility where exact shapes or
  reachable poses cannot be proven automatically.
- Frame/timing windows and verified interruption microtraces.
- Actor unloads, clips, alternate loading-zone activation, and other techniques.
- Timing, reliability, difficulty, and practical route cost.
- Poorly understood flags, actor parameters, and lifecycle behavior.
- Cross-version observations, wrong-state preservation matrices, and hypotheses.

Coverage is tracked per fact and obligation, not as one percentage for the whole
game. Extraction may cover most topology while a small number of unknown physical
obligations still dominate route validity. Human refinements are versioned,
evidenced overlays over the generated base rather than silent edits to it.

---

## 6. Dependency-ordered implementation plan

### Immediate stabilization gate

- [x] Enforce checkout-independent canonical bytes for planner-owned JSON
      artifacts, preferably with repository LF attributes plus a parser/error
      contract that makes accidental newline conversion obvious. Planner data
      JSON plus C/C++ source-evidence files now use explicit LF attributes.
- [x] Restore the complete planner engine suite to green on Windows and retain a
      regression proving exact-build recognition and audited actor-transition
      imports survive a normal checkout. All 233 engine tests pass, including
      exact build recognition and the keyed actor/door imports formerly hidden
      by the failed registry decode.
- [x] Run both the runtime and engine manifests in the normal planner test entry
      point so a green wrapper suite cannot hide failing dependency-crate tests.
      The planner workspace now declares both packages as default members.

### Phase 0 — Evidence inventory and terminology

- [ ] Catalogue exact supported builds, revisions, disc/executable/resource
      digests, available language bundles, and stable content IDs.
  - [x] Register the locally reproduced GZ2E01 GameCube USA revision-0 tree as
        `gcn-us-1.0-gz2e01` with complete executable, normalized game-data, and
        resource-manifest digests. Other retail builds and language/configuration
        coverage remain open.
- [ ] Audit how language/configuration is selected, persisted, changed, and used
      to select message or other resource variants.
  - [x] Separate external console language, the serialized-but-nonauthoritative
        player-config language byte, and concrete mounted message-resource
        identity. Prove GZ2E01's fixed `Msgus` group path from exact DOL bytes
        and source-audit the GameCube PAL `Msguk`/`Msgde`/`Msgfr`/`Msgsp`/`Msgit`
        selection table and mount boundaries. See
        `docs/route-planner/runtime-language-selection-audit.md`.
  - [x] Add canonical bounded DOL range evidence for constants outside named
        functions. The extractor resolves virtual addresses through exactly one
        loadable text/data section and rejects zero, oversized, cross-section,
        overlapping-section, or truncated ranges; the two GZ2E01 `Msgus` path
        artifacts are now replayable examples.
  - [ ] Reproduce exact PAL, Wii, and HD executable/resource identities and bind
        every supported runtime language to its actual base and numbered message
        archives; source-family branches alone must not enable planner facts.
- [x] Define the supported-input policy: pre-generated known fact packs for normal
      use, optional `orig/` extraction for verification/new inputs, and explicit
      unsupported behavior for unknown identities.
- [x] Inventory extracted world-data schemas and their missing fields.
  - `docs/route-planner/world-data-schema-inventory.md` separates the native
    orig bundle, compatible world inventory, imported planner facts, and diff/
    cache artifacts; records exact GZ2E01 coverage; and lists missing fields by
    identity, topology, geometry/live state, messages/events, and storage.
- [ ] Catalogue all save/runtime components and reset boundaries.
- [ ] Audit title, no-file, save-slot, load, void, death, and savewarp flows.
  - [x] Audit and model the exact GZ2E01 successful reset-to-opening prefix:
        GCN reset/menu/fader guards, restart-room parameter zeroing, the
        `PROC_OPENING_SCENE` handoff, and its pending F_SP102 load. Execution
        state now distinguishes non-world processes from loaded maps, so a
        title/file-select process cannot make the retained last map traversable.
        The exact opening/file-0 initializer now requires an observed phase-4
        scheduler state, completes the pending load without authorizing world
        traversal, resets the canonical inventory/event/temporary/stage-memory/
        return-place projections, and invalidates active-runtime stored stage
        banks without touching unrelated inactive stores or physical images.
        A non-title runtime now enters a fresh memory-backed title-file-0
        lifetime atomically at that same phase boundary. Unprojected save
        members, the file-select save-time suffix, void, and death remain open.
        The
        source-audited title A/Start input, pending `PROC_NAME_SCENE` request,
        and normal GCN file-select creation are now separate guarded actions;
        the latter repeats the save-domain initializer and writes
        `mNewFile = 0` and `mNoFile = 0` only after the name process/create phase
        is independently observed. See
        `docs/route-planner/gz2e01-title-boundary-audit.md`.
  - [x] Source-audit the next GZ2E01 file-select decisions before promoting only
        the evidenced portions to executable rules: blank-slot selection,
        existing-slot `card_to_memory`, no-save/no-card initialization, header
        writers, load-time normalization, and the pending play-scene request are separated
        in `docs/route-planner/gz2e01-file-select-branches.md`. The branch and
        backing-store subset plus pending play-scene requests are now executable.
        Exact GZ2E01 DOL artifacts seal `card_to_memory` and `setLineUpItem`,
        and typed operations execute the conditional 12-life floor, dungeon-6
        key reset, hookshot migration/lineup rebuild, saved vibration, and
        displayed return-place stage. Exact clock-derived save timestamps stay
        open.
  - [x] Seal and execute the exact GZ2E01 new-file name-entry suffix. Player and
        horse confirmations copy observed encoded byte strings into runtime-file
        player info; the default-horse/fade chain, ordinary/no-card player Back,
        and the two-phase horse Back path remain explicit. Horse confirmation
        now writes `selection_end` without fabricating a physical save. Four
        exact-DOL function artifacts and the source audit are recorded in
        `docs/route-planner/gz2e01-file-select-branches.md`.
  - [x] Seal and execute the exact GZ2E01 successful physical-save result and
        all observed lantern/event projection branches. A dynamic active-runtime save derives
        its persistent identity after arbitrary prior loads, commits every
        available stage bank, changes only the selected slot, writes
        `mDataNum`/`mNoFile` only on SaveSync result 1, and leaves result 2
        slotless. Projection-only event clearing and missing-lantern/backed-oil
        repair leave live state unchanged. Clock-derived save timestamps are
        explicit unknowns pending native clock evidence.
  - [x] Source-audit the distinct GZ2E01 void/hazard collision-exit and held
        restart-room branches, lethal diversion, four death-continue destination
        families, return-to-title reset handoff, and pre-restart live mutations.
        The seven top-level branch functions are exact-DOL sealed; callee
        decoding and witnessed traces remain open. See
        `docs/route-planner/gz2e01-void-death-source-audit.md`.
- [ ] Audit SCLS and actor-driven transition consumers.
- [ ] Audit message-flow assets, generic node handlers, shared temporary progress
      bits, normal/abnormal cleanup, and item/event handoffs.
- [ ] Audit interaction/attention volumes, forced cutscene triggers, player-control
      gates, and temporal windows for representative NPC setups.
- [x] Audit keyed door/gate actor families, their key/boss-key guards, consumption,
      persistent unlock writes, and alternate activation paths.
  - [x] Audit the exact-GZ2E01 `DOOR20` Forest Temple small-key representative,
        including the front-side key guard, accepted-event switch write, queued
        key decrement/commit, keyhole, collision, scene change, reload
        reconstruction, and OOB non-mutation boundary. See
        `docs/route-planner/gz2e01-forest-temple-small-key-door-audit.md`.
  - [x] Audit the GZ2E01 `fpcNm_L1BOSS_DOOR_e` family, including its actor-name
        aliases, boss-key and usable-side hard guards, parameter decoder,
        interaction bounds, unlock switch, event/collision/scene-change phases,
        and Forest Temple representative placement. See
        `docs/route-planner/gz2e01-boss-door-audit.md`.
  - [x] Audit the exact-GZ2E01 `fpcNm_L5BOSS_DOOR_e` family, including its
        human-form and boss-key guards, positive-local-Z usable side,
        interaction bounds, stage-type-dependent keyhole/switch behavior, and
        both retail placements. See
        `docs/route-planner/gz2e01-l5-boss-door-audit.md`.
  - [x] Complete the exact gameplay key-read/decrement source census and
        79-stage placement census for mini-boss doors, key shutters, Koki gates,
        rider gates, caravan gates, the guard-only Lakebed bridge demo, and the
        zero-placement generic `bdoor` family. Record memory/zone/event-bit,
        transient-pair, destructive, external-switch, and `STAFF_SHUTTER`
        alternate paths without generalizing them into one door rule. See
        `docs/route-planner/gz2e01-keyed-door-gate-family-audit.md`.
- [ ] Inventory static placement, persistent control, and transient instance state
      for representative actor families.
- [ ] Audit SavMem placements, guards, and all return/restart-place writers.
- [ ] Record known BiT, BiTE, Auru duplication, wrong-flags respawn, Fanadi lock,
      Text Displacement, and Ordon/twilight route evidence without prematurely
      encoding conclusions.
- [x] Establish a glossary: build, runtime file, backing, slot, component, payload,
      binding, fact, technique, obstruction, obligation, refinement, route book,
      proof. See `docs/route-planner/glossary.md`; it also fixes the meanings of
      runtime configuration, candidate transition, writer/reader/gate/latch,
      plan region, and evidence status.

Deliverable: an evidence index and a list of explicit unknowns.

### Phase 1 — Typed semantic IR

- [x] Define exact content identities, mutable runtime locale/configuration,
      selectors, groups, and proven-equivalence sets.
- [x] Define deterministic fact-pack manifests with input, executable, resource,
      extractor, schema, coverage, and output digests.
- [x] Define execution environment, runtime-file identity, backing attachment,
      and serialization policy independently.
- [x] Define typed components, bindings, lifetimes, and serialization owners.
- [x] Define raw/friendly fact catalogue and derived-rule IR.
- [x] Define component operation and boundary-policy IR.
- [x] Define transition, writer, gate, reader, technique, obstruction, and resolver
      schemas.
- [x] Require every candidate transition to carry an activation contract with
      hard guards, physical obligations, effects, and explicit unknown fields.
- [x] Define static world-object, persisted control, and live actor-instance
      representations plus reconstruction rules.
- [x] Define message-flow instance, node/branch, scheduled cleanup, player-control,
      temporal-window, and witnessed-microtrace representations.
- [x] Define interaction obligations over required/excluded volumes and poses.
- [x] Define goals, path constraints, costs, and evidence status.
- [x] Define refinement pack manifests, conflicts, and deterministic precedence.
- [x] Prevent a raw binding or fact observed for one exact context from becoming
      universal without an explicit, evidenced equivalence declaration.
- [x] Add strict schema validation and stable IDs.

Deliverable: one validated runtime representation independent of authoring format.

### Phase 2 — Observation, snapshots, and diffs

- [x] Extend the current tape/trace format with runtime-file identity and backing
      attachment.
- [x] Capture physical slots separately from the active runtime.
- [x] Snapshot typed components plus unknown/raw regions where possible.
- [x] Record binding changes and component provenance.
- [ ] Record return/restart values, gates, and relevant actor writes.
  - [x] Observe held return/restart values and loaded SavMem writer targets plus
        exact `NO_TELOP`/event/switch guard evaluations (native observation v14).
  - [ ] Audit and observe other return/restart writers and produce traces that
        distinguish eligible SavMem execution from an actual value change.
- [ ] Record `mGtItm`, `mPreItemNo`, current flow/node/cut, message-progress bits,
      pending cleanup, item partner, event name, and player-control transitions.
- [ ] Produce semantic and raw diffs across room load, stage load, save, load,
      void, title, BiT, and BiTE boundaries.
- [x] Make unsupported/unobserved fields explicit rather than defaulting false.

Deliverable: replayable state evidence that can validate transition rules.

### Phase 3 — Base mechanisms and upper-bound graph

- [x] Build the deterministic `orig/` extraction pipeline and cache/reuse its
      content-addressed derived fact packs without requiring original assets at
      planner runtime.
  - [x] Compile canonical world inventories into a strict exact-context payload
        and sealed manifest, and cache both in the immutable content store.
  - [x] Add planner-owned, bounded Yaz0/RARC resource extraction plus BMG
        message-flow and DZS/DZR actor-placement decoding with archive/resource
        digests; orchestration, input discovery, and sealed derived-pack output
        remain open.
  - [x] Add full input discovery/version verification and one-command extraction
        from a supplied `orig/` tree.
    - `scan-orig` reads the disc header, hashes a normalized complete file
      manifest, rejects ambiguous roots and symlinks, and never trusts directory
      labels. `extract-orig` requires an exact content identity, verifies the
      complete fingerprint, and emits a canonical decoded stage/message bundle
      plus sealed fact-pack manifest without host paths or original bytes. See
      `docs/route-planner/orig-discovery-and-extraction.md`.
    - Message archives are classified from their RARC resources rather than
      filename inference: group 0 is retained, GZ2E01's empty `bmgres99.arc` is
      represented explicitly, and ambiguous/malformed candidates fail closed.
  - [x] Install and reload canonical payload/manifest pairs through an immutable
        manifest-digest cache; materialization re-verifies both artifacts and
        needs no `orig/` tree.
- [x] Auto-detect and verify supported inputs; reject label/digest disagreement
      and represent unknown inputs as unsupported rather than guessing.
  - The canonical supported-build registry accepts only complete exact
    fingerprints. The planner bundles the audited GZ2E01 GameCube USA identity;
    `identify-orig` and `extract-orig` automatically select an exact match, emit
    or fail explicitly for unknown bytes, and treat `--content-id` only as a
    checked selection hint. Populating the remaining audited retail entries is
    the separate Phase 0 evidence-catalogue task.
- [x] Import world-context stages, room/layer bindings, player spawns, static
      placements, raw SCLS records, and collision/SCLS activation joins.
  - [x] Own the compatible world/native observation input contracts inside the
        planner and remove all planner-to-Huntctl crate dependencies.
- [ ] Import actor-driven transitions and any remaining map/room metadata not
      represented by the current world inventories.
  - [x] Import exact-GZ2E01 L1-family boss-door candidates by joining actor
        parameters to the unique same-room SCLS record. Reverse-side, ambiguous,
        non-audited-build, and unmodeled-switch-domain cases remain encoded facts
        without invented transitions.
  - [x] Import the exact-GZ2E01 L5 boss-door candidates with their additional
        human-form guard and distinct dungeon-side versus stage-type-3 boss-room
        unlock effects. Interaction geometry and actor/event/restart phases
        remain explicit obligations.
  - [x] Import exact-GZ2E01 front-side keyed mini-boss first-open/reopen door
        branches, internally checked key-shutter actions, the Lakebed boss
        shutter's zero/normal/high-small-key outcomes, and layer-sensitive type-0
        Koki-gate unlock actions. V9 requires encoded-map/door transitions to
        reference exactly one SCLS record and forbids actor-driven actions from
        fabricating one. Rider/caravan/external-switch families remain explicit
        audited boundaries pending their missing state domains.
- [ ] Model ordinary item/NPC/event producers.
- [x] Implement normal bank commit/load and binding changes.
  - [x] Execute typed serialize/restore/bind/rebind operations against independent
        owner stores atomically.
  - [x] Scope every stage-bank owner by runtime file plus stage and execute one
        checked commit/load operation that commits the outgoing live payload,
        restores the destination entry, and applies its explicit semantic
        binding atomically. A map transition remains a separate authored effect.
        See `docs/route-planner/backing-store-boundaries.md`.
- [x] Derive bound small-key counts and dungeon items from per-stage memory.
  - `bound_raw_bits` resolves numeric values and bit masks from exactly one raw
    component with the selected kind and binding; missing, ambiguous, out-of-range,
    or partially unknown backing fails to unknown. `adjust_bound_raw_unsigned`
    atomically increments/decrements a uniquely bound known count, rejecting
    underflow and overflow. Native projection leaves the source-audited key count
    and dungeon-item byte only in the stage-bank payload instead of duplicating
    them into runtime inventory. See
    `docs/route-planner/bound-stage-memory-semantics.md`.
  - [x] Stop projecting dungeon-local keys/items into runtime inventory, retain
        the raw stage-bank bytes, and resolve validated raw byte/mask references
        by component kind plus exact current binding. Ambiguous components or
        unknown selected bits fail unknown; build-specific friendly aliases and
        imported consumers remain open.
  - [x] Execute checked unsigned adjustments against the unique raw component at
        that binding, allowing pickups/doors to mutate the same backing-derived
        count without naming a fixture-specific component ID; unknown bytes,
        ambiguity, and underflow/overflow fail atomically.
  - [x] Let imported predicates, aliases, and bound mutations resolve the active
        runtime-file, current-stage, or current-room binding at evaluation time.
        Components retain concrete bindings and serialization owners; only rule
        references are dynamic, so a reusable actor rule follows a location or
        file change without retaining its authoring context.
  - [x] Add binding-sensitive masked raw writes and knownness invalidation for
        persistent flags, dungeon items, and stage/room switches. They require
        exactly one raw component at the resolved backing and fail atomically on
        missing, ambiguous, non-raw, or out-of-range targets.
- [ ] Import hard door/actor guards and their state operations where decidable.
  - [x] Import the GZ2E01 L1-family boss-key/current-stage guard and memory-switch
        write, while retaining interaction geometry and actor/event/collision
        phases as separate unresolved obligations.
  - [x] Import the GZ2E01 L5 human-form/boss-key guards and conditional
        memory-switch write while retaining its usable-side geometry and
        keyhole/event/collision/restart phases as separate unresolved
        obligations.
  - [x] Import exact-GZ2E01 current-stage small-key, boss-key, memory-switch,
        source-room/layer, first-open/reopen, switch-write, and key-adjustment
        semantics for the audited mini-boss, key-shutter, Lakebed boss-shutter,
        and Koki-gate placements. Preserve event, geometry, keyhole, collision,
        pushing, and restart work as named obligations, and exclude non-memory,
        transient, destructive, bypassed, external-switch, and unaudited types.
- [ ] Import message-flow graph nodes, temporary-bit reads/writes, branch
      predicates, normal cleanup, and item/event handoffs from the selected
      language resources where decidable.
  - [x] Add a planner-owned message-flow program/compiler that turns extracted
        nodes and known generic temporary/persistent/switch handlers into
        binding-sensitive transitions, readers, and friendly aliases. Preserve
        unknown node/handler semantics explicitly; compile caller-specific
        cleanup edges and exact node-level event handoff contracts without a
        special-case glitch capability. See
        `docs/route-planner/message-flow-programs.md`.
  - [x] Construct one canonical program per message group from every resource in
        an exact runtime-language selection. Versioned import profiles own the
        language-to-bundle mapping and audited backing layouts; missing language
        mappings and ambiguous groups fail closed, while unaudited switch stores
        remain explicit unknown requirements. The standalone
        `construct-message-flows` command emits a canonical program set. See
        `docs/route-planner/message-flow-programs.md`.
  - [x] Compile selected program sets into deterministic exact-context fact and
        mechanics catalogs. Resource/profile-digest-pinned overlays add audited
        event and cleanup contracts; duplicate IDs and invalid references fail
        transactionally instead of partially merging. The standalone
        `compile-message-flows` command emits the compiled set.
  - [x] Seal each compiled set with the normal fact-pack manifest and source/
        coverage records, and let `compose --message-flow-set` merge one or more
        sets transactionally into ordinary base catalogs before refinements.
  - [x] Keep extracted accesses with unaudited backing stores as blocking unknown
        requirements, and separate label-indexed observation arrays from the
        unique writable temporary event-register and loaded-stage-memory
        backings in state snapshots.
  - [x] Resolve backing references from typed fields on live components, with
        unknown-on-missing evaluation, atomic write failure, and conservative
        relevance matching. This permits later actor-entry contracts to bind
        speaker-relative zone stores without assuming the player's current room.
  - [x] Define and compile exact stage/actor message-entry contracts. Pin raw
        actor placement records and exact flow labels, retain interaction
        obligations and unknowns, emit a portable deterministic artifact, and
        require its exact compiled message-flow dependency during composition.
        See `docs/route-planner/message-entry-contracts.md`.
  - [ ] Publish audited exact-build import profiles and concrete stage/actor
        entry packs; import additional decidable item/event/jump handlers and
        real cleanup caller predicates.
    - [x] Publish the exact GZ2E01 English partial profile for temporary event
          registers and current-stage save switches. Persistent event registers,
          dungeon-session, zone, and one-zone stores remain explicit unknowns
          pending distinct backing and speaker-zone modeling. See
          `docs/route-planner/gz2e01-message-import-profile.md`.
    - [x] Publish and exact-compile the first concrete GZ2E01 stage/actor entry
          pack: F_SP115 STAG group 8 plus the separate R01 layer-13 `Seirei`
          placement into flow 21, with switch `0x0c`, actor/control obligation,
          and unaudited shared-attention conditions kept distinct.
    - [x] Compile nonzero `event009` flow jumps to exact selected-resource FLI1
          labels without also applying their encoded successors. Keep dynamic
          jump-zero group selection unknown, and treat source-inert handlers
          12/19/42 as encoded-successor edges without invented state effects.
    - [x] Resolve event successors through their single shared target-table entry
          while keeping message-node direct successors and branch-node paired
          target entries distinct. Real node 315 now reaches its encoded terminal
          target instead of being misread as direct node 201.
    - [x] Compile `event008` into explicit flow-component `event_id`/`item_id`
          handoff writes without granting the item or creating its actor. Keep
          event-27 fundraising state explicit as an unimported side effect.
    - [x] Project the exact 256-byte persistent event-register backing separately
          from label diagnostics, bind it to the active runtime file, and compile
          audited `F_0615` reads/writes without confusing nearby `M_033`.
    - [x] Model item ownership as per-item raw backing metadata, project the exact
          five-byte light-drop payload, and compile Lanayru query 22/event 17 to
          the same runtime-file Vessel bit without an abstract inventory flag.
    - [x] Compile an actor-backed presentation-request consumer from exact
          speaker/event/item fields to the session recent-item store, while
          leaving item-actor execution and grant as distinct later transitions.
    - [x] Resolve each requested item's exact ownership backing and compile one
          shared generic get-item consumer whose presentation-actor execution
          is an auto-bound actor-state obligation, not an inferred success.
- [ ] Import cutscene phase graphs, embedded scene changes, return/restart-place
      writers, actor/resource archive requests, load-failure/fallback branches,
      and ordered cleanup where decidable.
  - [x] Add a planner-owned cutscene-program schema/compiler whose phase and
        resource guards auto-compile into ordinary transitions; normal, skip,
        interruption, scene-change, and load-failure branches retain distinct
        transition kinds and exact ordered effects. See
        `docs/route-planner/cutscene-phase-programs.md`.
  - [x] Add bounded planner-owned `event_list.dat`, REVT, and LBNK extraction;
        prove the exact GZ2E01 two-wrapper chain: tower `demo07_01` normally
        enters R_SP301, whose `demo07_02` normally enters Castle Town; both
        retain authored Zelda-tower skip destinations. JStudio phase decoding
        and exceptional failure semantics are tracked separately. See
        `docs/route-planner/gz2e01-zelda-cutscene-source-audit.md`.
  - [x] Join a named event's REVT/LBNK/SCLS records to its linked
        `event_list.dat` staff/cut/data paths in a canonical planner-owned
        topology artifact. Keep JStudio phases, resource-failure flow, and
        return-place writers typed as unresolved rather than compiling guessed
        transitions.
- [ ] Represent partial cutscene execution conservatively: preserve confirmed
      prefix writes, retain values whose writers are confirmed skipped, and mark
      effects beyond an unknown failure/interruption boundary unknown.
  - [x] Add cutscene scene-change/resource-load-failure transition kinds and a
        masked raw-knownness invalidation operation, so a modeled exceptional
        branch can retain confirmed bits while invalidating only unaudited ones.
  - [x] Add structured-field invalidation and compile authored uncertain suffix
        targets separately from confirmed prefix operations. A skipped writer is
        absent rather than replaced, so retained return-place state remains an
        input to the ordinary savewarp reader.
  - [ ] Import concrete ordered phase branches and affected-bit masks from
        evidence instead of authoring guessed suffix effects.
- [ ] Produce semantic/raw build and language diffs, with explicit unknown or
      uncovered fields rather than assumed equivalence.
  - [x] Compare canonical extracted stage/message records and ignored archive
        candidates by raw and decoded digests; explicit locale pairing reports
        per-side group counts and one-sided coverage. Broader decoded domains
        and rule-level semantic equivalence remain open.
- [ ] Reconstruct live actor behavior from placement, layer, persisted state, and
      instance lifecycle.
- [ ] Implement save/load/title/runtime-file operations.
  - [x] Add transactional primitives for serialization, restoration, explicit
        component projection between runtime-file bindings, and location changes.
  - [x] Model persistent-file images, populated physical slots, active-runtime
        retirement/backing attachment, exact projection manifests, and concrete
        executable save/load/stage-activation operations with a file-0 sequence
        fixture. File 0 can be projected into slots 1–3 and then end when a fresh
        card-backed runtime loads; session-owned state is independent and the
        loaded runtime's future save targets remain explicit. See
        `docs/route-planner/backing-store-boundaries.md`.
  - [ ] Model concrete title-return/void handling and build-specific save-time
        normalization/clearing as evidenced transition programs.
    - [x] Model the exact GZ2E01 reset-to-opening prefix as a guarded title-return
          transition with an explicit process context and pending world-load
          request; do not conflate that request with a completed map load.
    - [x] Model the route-relevant exact title-origin-file-0 opening projection
          as mutually exclusive direct and enter-new-lifetime transitions. They
          require an explicit phase-4 scheduler observation, replace complete
          known component payloads, and invalidate only active-runtime stored
          stage-bank projections; unrelated inactive runtimes and physical
          images remain independent.
    - [x] Add a generic runtime-lifetime handoff that ends the incoming runtime,
          derives a fresh identity, and rekeys all of its live and serialized
          backing stores without copying session state or mutating card images.
    - [x] Model the source-audited GZ2E01 title key input, pending normal
          name-scene request, and file-select create projection without treating
          `fopScnM_ChangeReq` as completed process activation. The create step
          requires independent `PROC_NAME_SCENE`/phase observation, repeats the
          audited save-domain reset, and writes `mNewFile = 0` and `mNoFile = 0`.
    - [x] Model mutually exclusive GZ2E01 blank-slot, existing-slot-menu, and
          no-card decisions over observed file-select control state. Blank slots
          write zero-based `mDataNum`/`mNewFile = 128` without creating an image;
          no-card initializes three explicit custom session buffers and copies
          entry 0 into the memory-only live runtime; existing Start derives the
          selected sealed manifest, fresh runtime identity, and explicit
          header/restart/temporary carry set. Exact DOL evidence, a typed
          unsigned clamp, and a parameterized item-layout operation execute the
          full route-relevant post-copy normalization: life floor, dungeon-6 key
          reset, hookshot migration/lineup rebuild, saved vibration, and
          displayed return-place stage.
    - [x] Model fixed new-file and return-place-derived existing-file play-scene
          requests after independently observed selection completion. Both retain
          active `PROC_NAME_SCENE`; the pending destination does not make the
          retained world traversable or prove `PROC_PLAY_SCENE` activation.
      - [x] Represent all three no-card `dFile_select_c::mSaveData` entries as
            independent process/session-owned custom stores, copy entry 0's
            exact modeled payload projection into the live runtime without
            changing its ownership, and keep both stores distinct from physical
            slots and persistent-file images.
    - [x] Source-audit void/hazard and death selection without collapsing
          collision exits, held restart-room state, captured actor exits,
          boss/special-stage branches, and title reset into one reload edge.
          Seven top-level GZ2E01 functions are exact-binary sealed; executable
          transition programs and deeper callee decoding remain open.
- [x] Implement writer/gate/reader evaluation and last-writer provenance.
  - [x] Evaluate scoped/evidenced writer activation separately from active and
        unknown blocking gates, resolve reader source values separately from
        friendly interpretations, and append the responsible transition to every
        component mutation.
  - [x] Retain a queryable, ordered operation/boundary history across search
        states and expose current-field last writers plus per-gate event history.
        History is part of full state identity and inspection but excluded from
        semantic search dominance; failed atomic batches cannot leak events.
  - [x] Execute standalone writer rules as gated solver actions and retain
        writer/gate evidence plus blocked-writer witnesses in solve reports.
  - [x] Add atomic multi-field structured writes and a backing-driven dynamic
        location load. One `Savmem` action can now replace stage, room, and
        player-status/spawn together, and ordinary savewarp can read those
        fields without enumerating destination-specific transitions.
  - [x] Execute ordered writer/gate/read programs from imported mechanics.
    Writer records are searchable `writer` actions rather than techniques. Search
    reevaluates their activation and every blocking gate at each state, applies
    the typed operation transactionally, and retains blocked-writer witnesses.
    Reader records remain attached to their consuming transition: their exact
    source value and optional friendly interpretation appear in the transition
    proof, while a missing or disallowed in-scope reader makes the transition
    feasibility unknown. Writer, gate, and reader evidence all contribute to the
    step's weakest-evidence result.
- [x] Generate the upper-bound authorization graph.
  - [x] Add exact-context, evidence-aware tri-state predicate evaluation and
        per-transition upper-bound assessment; unknown raw bits, absent values,
        unsupported equivalence scopes, and disallowed evidence remain unknown.
  - [x] Materialize and traverse the authorization graph from evaluated
        snapshots.
    - Added the canonical `dusklight.route-planner.authorization-graph/v1`
      artifact and `project-authorization-graph` command. The bounded
      breadth-first traversal roots every transition, writer, and stateful
      technique, binds exact state, snapshot, catalog, refinement-stack,
      equivalence-set, evidence-policy, and search-bound identities, and leaves
      any reached-but-unevaluated frontier explicit.
    - Regression coverage proves `A -> B -> C` materialization, permits an
      unresolved physical obligation only in upper-bound mode, keeps an
      explicitly unknown activation requirement non-executable, reports the
      unknown transition, validates bounded frontiers, and round-trips the
      canonical graph through both the engine and file-bound CLI.
- [x] Keep extracted destinations non-executable until their activation contracts
      are discharged.
  - [x] Classify hard-guard failure, unresolved requirements, and outstanding
        physical obligations separately; modeled feasibility cannot treat an
        outstanding obligation as executable.
  - [x] Reject any extracted encoded-map transition that changes location but
        carries neither a physical obligation nor an explicit unknown
        requirement. Importer and catalog-validation regressions prove a decoded
        SCLS destination alone cannot become a validated executable edge.
- [x] Mark candidates whose activation physics remain unknown.
  - Authorization edges now retain outstanding and evaluated-unknown physical
    obligation IDs. The graph also emits sorted candidate-level markers that
    distinguish explicit unknown activation requirements, unresolved obligation
    definitions, and obligations that evaluated unknown at reached states.
    Tests cover all three causes while proving that permissive traversal does
    not erase them or turn an explicit unknown requirement into an edge.

Deliverable: the intentionally permissive logic graph with honest uncertainty.

### Phase 4 — Component transfers and state splices

- [x] Implement per-component transition policies.
- [x] Implement project/preserve/clear/copy/rebind operations.
- [x] Prevent accidental preservation of unspecified components.
- [x] Support mixed provenance after cross-file operations.
  - Typed operation batches are atomic, retain copied provenance, append the
    responsible transition, and keep serialized owner stores separate from the
    visible snapshot. Masked raw writes establish only the bits they actually
    write, while checked relative adjustments model counters such as key use
    without replacing them with route-specific Booleans. Preservation requests
    are explicit one-boundary overrides. The boundary executor applies exactly
    one matching rule or the declared default to every live component, rejects
    overlapping selectors, and fails the whole atomic transition on `Unknown`.
- [ ] Encode an evidence-backed BiTE preservation matrix.
  - [x] Add a generic checked carry manifest to physical-slot load. Only named,
        runtime-lifetime, source-owned, non-stage-bank components are rekeyed;
        the manifest must be sorted, unique, disjoint from the card image, and
        every omitted source-runtime component/store is removed atomically.
        Concrete BiTE component membership and activation evidence remain open.
- [x] Encode the shared Auru recent-item store/writer/consumer mechanism separately
      from build-specific activation feasibility and external HD evidence.
  - Native event observations project the recent get-item ID into its own
    session-lifetime component instead of conflating it with pending
    `mPreItemNo`. Generic value-copy and item-bit operations demonstrate file A
    writing an item ID, file B retaining it across a load boundary, and the
    generic grant consuming that value. This proves the causal mechanism only;
    Auru interaction geometry and HD/SD activation evidence remain separate
    open obligations in 11I.
- [x] Add a hypothetical local-bank rebind refinement for testing.
  - [x] Compose a hypothetical preserve/rebind technique, keep its raw payload
        byte-identical, derive the destination-stage alias and downstream
        transition from the new binding, and prove removing the overlay removes
        the route without a handwritten obligation discharge.
- [x] Add diagnostics for aliases that change under a binding.

Deliverable: one generic system for known and proposed wrong-state transfers.

### Phase 5 — Physical feasibility and obstructions

- [ ] Derive approach geometry from collision and spawn data where possible.
- [ ] Define obligations for reaching/activating each candidate transition.
- [ ] Define obligations for state-producing interactions and interruptions, not
      only final map transitions.
- [x] Support required talk/attention volumes, excluded cutscene-trigger volumes,
      facing/control predicates, and temporal windows.
  - [x] Validate evidenced axis-aligned world volumes and derive required-inside,
        excluded-outside, player rotation/action/control pose results from the
        exact propagated snapshot; require the addressed live actor to be loaded,
        and keep missing actor/volume observations unknown.
  - [x] Derive sphere and vertical-cylinder membership plus witnessed temporal
        windows with exact action/input/frame requirements and proof provenance.
  - [ ] Add any actor-family-specific oriented or compound shapes found by the
        interaction-volume audit rather than assuming one universal primitive.
- [x] Import authored obstructions without mutating build facts.
  - [x] Evaluate obstruction activation and resolver applicability as separate
        scoped/evidenced rules; a resolver discharges named obligations but does
        not delete or falsify the underlying obstruction.
  - [x] Load those records from canonical refinement packs into deterministic,
        base-digest-bound composed runtime catalogs.
  - [x] Add authored obstruction selectors and a deterministic composition pass
        that binds them to candidate actions by stable ID or structural
        source/destination/approach/context matching, emitting graph `blocks`
        dependencies without route-book wiring.
  - [x] Require explicit exact-one versus plural cardinality, fail closed on
        unmatched or ambiguous selectors, retain selector/provenance in the
        compiled binding, and add stale-binding regression tests across catalog
        digest changes.
- [x] Support direction, form, mount, twilight, actor, void, and layer scope.
      These resolve through typed player/location/actor values, semantic facts,
      and plane-side observations rather than named route claims.
- [x] Classify candidates as feasible, obstructed, or unknown.
  - [x] Implement the loss-aware per-snapshot classification primitive, including
        a distinct inapplicable scope and hard-guard-blocked result.
  - [x] Derive discharged obligations from applicable resolvers and techniques,
        retain active/unknown obstruction IDs, and keep introduced obligations
        outstanding.
  - [ ] Evaluate authored obstruction/resolver catalogs and obligation details to
        derive discharged-obligation sets rather than accepting them as input.
    - [x] Derive predicate obligations from propagated state with distinct false,
          unknown, unsupported-scope, and disallowed-evidence outcomes; recompute
          after setup operations before transition assessment.
    - [x] Derive required/excluded volume, direction, geometry, witnessed timing,
          and void-plane obligations from their typed details and observations.
      - [x] Derive exact AABB membership plus authorable rotation/action/control
            predicates and loaded-actor state; fail closed when a referenced
            actor or volume is absent.
      - [x] Derive directed region connectivity, sphere/cylinder membership,
            plane-sidedness/void state, and evidence-scoped witnessed timing.
    - [x] Derive predicate-only obligations (and interaction pose when it is the
          complete obligation) from tri-state snapshot evaluation; re-evaluate
          after propagated operations and retain unknown obligation IDs in proofs.
    - [x] Derive required/excluded volume, geometry/region, and witnessed temporal
          obligations from their typed evidence instead of explicit discharge
          claims.
      - [x] Required and excluded AABB observations derive discharge/obstruction
            directly; no named technique claim is required.
      - [x] Directed region/plane observations and matching microtrace witnesses
            derive discharge directly and retain source IDs in solver proof.
- [x] Expose upper-bound versus modeled-feasible graph diffs.

Deliverable: flag-permitted nonsense is visible but no longer reported as a
verified route.

### Phase 6 — Technique and refinement packs

- [x] Build validated pack contracts for techniques and obstruction resolvers.
- [x] Encode exact setup, component operations, discharged obligations, and cost.
- [x] Allow a pack to supply a witnessed microtrace with exact pre/post state and
      timing rather than a global named Boolean.
- [ ] Add built-in packs for ordinary movement and selected sequence breaks.
- [x] Add route-local and ephemeral what-if overlays.
  - Refinement-stack entries identify `enabled_pack`, `route_local`, or
    `ephemeral_what_if` provenance. Layer order dominates a pack's local
    precedence, duplicate IDs across layers fail closed, and an earlier layer
    cannot depend on a disposable later layer. CLI and service composition take
    the layers separately; solve reports retain every layered stack entry and
    digest. Removing a what-if layer deterministically restores the route-local
    result, and removing both restores the enabled-pack result.
- [x] Distinguish satisfy, bypass, avoid, supersede, and assume-absent resolver
      operations, plus explicit record replace/supersede/disable operations.
- [ ] Add complete import/export and conflict diagnostics.
  - [x] Add strict canonical pack import, deterministic composition, canonical
        composed-catalog export, dependency-digest checks, and fail-closed
        conflict/replacement errors to the planner-owned CLI.
  - [ ] Add structured multi-error diagnostics, pack export authoring, and editor
        fix suggestions.

Deliverable: researchers can extend the model without editing core code.

### Phase 7 — Solver

- [x] Implement backward relevance expansion from goals.
  - `BackwardRelevance` computes a deterministic catalog-only fixed point from
    the goal's state dependencies through every matching transition/writer/
    technique producer, then through guards, readers, writer gates,
    obstructions/resolvers, obligations, reconstruction rules, and witnessed
    temporal actions. It retains unresolved frontier dependencies and all
    relevant IDs, handles OR producers and cycles, excludes unrelated mechanics,
    and deliberately makes no forward reachability claim.
- [x] Combine it with forward stateful feasibility from the start.
  - [x] Implement bounded forward state search over exact snapshots, typed
        transition effects, action-local resolver/technique selections, and
        modeled versus upper-bound feasibility.
  - [x] Add backward relevance pruning for catalog-goal solves. Unrelated
        actions are not explored, and the result retains the relevance proof.
        Required route predicates, pinned actions, and every action plus
        pre/postcondition in a required/selected method are independent roots,
        so route-book solves use the same pruning without losing authored work.
  - [ ] Add the remaining route/path constraints before treating this as the
        production solver.
- [x] Support OR producers, AND requirements, and ordered writer/gate/read setups.
  - Backward expansion retains every matching producer and terminates through
    causal cycles. Forward search evaluates nested predicates and preserves
    exact writer, gate, and consuming-reader order in the reached proof.
- [x] Implement state hashing, dominance, cycles, and continuation-safe merging.
  - [x] Add semantic search-state hashing that includes backing stores, bindings,
        gates, preservation, and pending cleanup while excluding snapshot labels
        and proof-only provenance; use it for cycle/duplicate suppression.
  - [x] Add resource dominance and continuation-safe merge proofs.
        Forward search now maintains a Pareto frontier over action depth and
        every accumulated route-cost axis for each exact continuation identity.
        The identity includes semantic live state, required-action completion,
        required/banned/preferred method cursors, satisfied preferences, and
        unknown route-condition state, so a cheaper label cannot erase a
        continuation-distinct route. Each strict prune retains a validated
        `ContinuationMergeProof` with the shared identity and both resource
        labels; incomparable cost vectors remain independently explorable, zero
        axes are canonicalized away, and proof retention is bounded by the
        declared state limit rather than becoming an unbounded side channel.
- [ ] Support exact content/language, evidence, technique, runtime-file, and path
      constraints; validate portable plans independently over every selected
      context using only appropriately scoped/equivalent rules.
  - [x] Apply scoped require/forbid predicates and techniques plus pin/ban
        executable-action directives during forward search, with route-book
        identity retained in standalone reports.
  - [x] Track ordered pin/ban/selected method subsequences as part of search
        identity.
  - [x] Evaluate method pre/postconditions at separate resolver, technique, and
        transition action boundaries.
  - [x] Apply non-repeatable action/method preference weights as the secondary
        objective after route depth.
  - [x] Execute cumulative per-axis technique-cost thresholds and strict
        route-level evidence thresholds.
  - [x] Validate portable multi-context route books independently in every
        selected exact context, requiring one start state per expanded context
        and returning per-context proofs plus a fail-closed aggregate status.
- [ ] Add multi-objective cost and K-alternative plan search.
- [ ] Keep graph-region encapsulation outside solver reachability semantics.
  - [ ] Preserve stable state/transition/boundary identities so solved or authored
        regions can be selected and projected without synthesizing macro edges.
  - [ ] Reevaluate every enclosed transition when a saved region is copied or
        referenced after a different state; grouping must grant no authority.
  - [ ] Support cycles and leave-and-return paths in the underlying graph without
        interpreting nesting as recursive goal decomposition.
  - [ ] Preserve every continuation-distinct state even when its containing region
        is visually collapsed.
- [x] Return reachable, unreachable-under-model, or unknown.
  - [x] Expose canonical fact/mechanics/execution-state artifacts through a
        standalone planner runtime boundary, isolated from huntctl's TAS CLI and
        workbench implementation.
- [ ] Report minimal missing obligations/assumptions where practical.
  - [x] Retain a deterministic closest blocker witness per transition with guard
        truth, active/unknown obstructions, selected setup, discharged and
        outstanding obligations, unknown requirements, and source-state digest.
  - [ ] Compute minimal failed-producer cuts across multiple upstream actions.
        Exhaustive bounded solves now emit validated `FailedProducerCut` records
        for concrete state dependencies when every catalog producer is a
        transition or writer, none executed, and each owns a retained blocked
        witness. A cut keeps all alternative producer actions, their typed
        failure classifications, and exact source-state identities; two-way OR
        producers are covered, while any executed technique/resolver/
        reconstruction producer or a hit search limit suppresses the claim.
        Extending the cut algebra through those unsupported producer families
        and nested goal boolean structure remains open.
- [ ] Add bounded suspicious-state queries and retain complete proof objects for
      model-bug versus research-lead triage.

Deliverable: a headless query API and deterministic fixture suite.

### Phase 8 — Proofs and explanations

- [ ] Retain causal proof objects for every result.
  - [x] Retain the selected transition, resolver/technique choices, and source/
        result semantic state identities for each reached-path step.
  - [ ] Retain full guard, obligation, operation, and evidence derivations for
        reached and failed alternatives.
- [ ] Explain derived lockouts as failed producer cuts.
- [x] Explain active/unknown obstructions and the resolver/technique chosen for
      each reached approach; retain the closest unresolved witness on failure.
- [x] Show component transformation and provenance histories.
- [x] Show last-writer and gate history for latched values.
  - Execution-state schema v10 records every typed operation and every resolved
    per-component boundary disposition with contiguous application-local order,
    affected component IDs, source sequence, and result snapshot. State
    inspection exposes the full log, a direct last-writer result for each live
    structured field, and all set/clear events for each gate. A Fanadi-shaped
    fixture verifies that a held return-place value retains its earlier writer
    while `NO_TELOP` is set and changes only after clear plus a later write.
- [x] Label all hypothetical and low-confidence dependencies.
  - Every reached step and blocked-transition witness carries canonical evidence
    dependencies for its transition, recursively referenced facts, active or
    unknown obstructions, obligations, selected resolvers/techniques, supporting
    microtraces, and unknown requirements. Each dependency retains its complete
    `RuleEvidence`, and `weakest_evidence` makes contested, hypothetical, or
    unknown support directly filterable without rejoining the catalog.
- [ ] Generate concise collapsed summaries and fully expanded research views.

Deliverable: every route and failure is inspectable rather than magical.

### Phase 9 — Independent planner UX and authoring

The first UI release is a deliberately small blueprint-style web application,
not the complete research workbench described by every later task. It must make
the already implemented transition/state engine usable before adding broad
visual polish or whole-game authoring features.

#### First web slice

- [x] Add a local planner web host around the typed Rust service and serve a
      versioned browser client without coupling it to Huntctl or TAS timeline
      schemas. `serve-web` binds only to loopback, embeds the static client,
      exposes the existing service at `/api/service`, applies request-size and
      timeout bounds, and serves a restrictive no-cache response policy.
- [x] Build a straightforward application shell with new/open/save/save-as,
      a searchable transition palette, an infinite pan/zoom canvas, and a bottom
      properties/state panel. The Rust-owned workspace lists built-in and saved
      projects, supplies a validated blank project, confines IDs to one project
      root, and performs revision-checked flushed atomic replacement. The browser
      tracks dirty layout state, warns before discard, persists through Save or
      Save As, and retains explicit JSON import/export.
- [ ] Let an author drag a transition onto the canvas and connect it to an exact
      predecessor state. Rust recomputes every downstream state and remains the
      only validation and mutation authority.
      The v29 service now evaluates one catalog transition against an exact
      execution-state document, resolves modeled obligations and evidence, and
      applies effects transactionally only for `executable`; the browser exposes
      this on selected transitions for projects with a start state. Persisted
      step insertion and downstream route propagation remain open.
- [ ] Render accepted connections distinctly from rejected or unknown joins.
      Selecting a rejected join must show missing producers, active
      obstructions/resolvers, unknown obligations, or exact-context mismatch;
      there is no force-connect operation.
- [ ] Save route semantics and presentation metadata through revision-checked
      route-book edits. Node positions, viewport, and visual grouping may persist
      but must never affect solver reachability.
- [ ] Selecting a state or transition shows exact before/after location,
      inventory, flags/components, bindings, provenance, effects, requirements,
      and evidence in the bottom panel.
- [ ] Support named visual regions with collapse and double-click/breadcrumb
      navigation. Regions are flat-graph encapsulation only: grouping never
      creates a goal, macro transition, alternative implementation, or new
      reachability semantics.
- [ ] Ship several small editable demonstrations from already modeled mechanics:
      a keyed door, Fanadi return-place locking, Auru recent-item transfer, Text
      Displacement toward Goron Mines, and a clearly hypothetical component
      rebind. Each must load, validate, edit, save-as, and visibly change its
      propagated state when a transition is removed or replaced.
      The exact GZ2E01 Fanadi return-place/savewarp mechanics and audited opening
      and file-selection flow now ship as read-only built-ins that can be saved
      into the editable workspace; transition editing and the remaining three
      demonstrations are still open.
- [ ] Add one browser-driven acceptance test that opens a demonstration, removes
      or replaces a transition, observes the changed downstream state/rejection,
      saves it, reloads it, and obtains identical semantic identities.

#### Extended editor

- [x] Define an independent versioned planner graph-projection schema and Rust
      crate/server API; do not overload timeline `WorkbenchGraph` or playback
      segment semantics.
  - [x] Establish `tools/route-planner` as a separate Cargo workspace, library
        API, CLI, and versioned solve-report owner without registering anything
        in Huntctl.
  - [x] Add the planner-specific canonical graph projection with typed causal
        relations and ordered, collapsible predicate subgraphs.
  - [x] Add a planner-owned typed stdio server transport for validation,
        composition, graph projection, and solving.
  - [x] Add revision-checked, atomic authoritative route-book edit commands to
        the planner CLI and typed service.
  - [x] Add the browser's loopback HTTP adapter over the same typed service
        envelope; no WebSocket is needed for the current request/response flow.
- [ ] Define the planner's own visual grammar and navigation conventions for
      colors, spacing, node anatomy, camera controls, breadcrumbs, selection,
      grouping, and detail panes. Existing TAS screenshots may inform this work,
      but no source import, running Huntctl instance, or shared component is
      required.
- [ ] Implement nested proof/plan regions with familiar subgraph navigation while
      preserving planner-specific AND/OR, cycles, and continuation-distinct
      frontier states.
  - [x] Define and validate nested route-book regions, alternative methods,
        ordered reference steps, and typed action references; project them into
        the planner graph without adding mechanics or inferred losses.
  - [ ] Project solver proofs/frontiers into those regions and prove collapse
        safety from continuation equivalence or explicit residual differences.
- [ ] Keep fact resolution, validation, solver/proof work, graph projection, and
      authoritative mutations in Rust; keep the browser a typed rendering and
      command surface.
- [ ] Share or extract UI/infrastructure code only after a domain-independent seam
      is demonstrated; visual consistency does not depend on shared graph models.
- [ ] Add route canvas, alternatives, pin/ban/prefer, and collapse controls.
  - [x] Define validated route-book directives and collapse policies and expose
        plan alternatives through the planner graph/service contracts.
  - [x] Add revision-checked mutation commands for goals, constraints,
        directives, steps, methods, regions, selections, collapse policies, and
        annotations.
  - [ ] Add pin/ban/prefer and alternative-selection browser interactions after
        the first drag/connect/save slice is usable.
- [ ] Make transition insertion the primary authoring operation.
  - [ ] From a selected state node, list applicable physical, event, warp,
        reload, title/file, and technique transitions.
  - [ ] Permit searching providers by desired destination or postcondition while
        retaining the actual entry contract and effects.
  - [ ] Reject invalid joins and show the missing state producers, active
        obstruction resolvers, unknown obligations, or exact-context mismatch.
        Do not implement a force-connect operation.
  - [ ] Allow an author to insert a suggested producer/resolver chain or create
        an explicitly hypothetical refinement from the rejection.
- [ ] Add nested subgraph authoring and browsing as graph-view encapsulation.
  - [ ] Group selected states/transitions into a named region without changing
        their solver identities, effects, or connectivity.
  - [ ] Enter/exit nested regions with breadcrumbs; one-trip and multiple-trip
        dungeon routes are separately constructed subgraphs, not child methods.
  - [ ] Show every incoming/outgoing boundary state and edge before a saved region
        is copied or referenced elsewhere.
  - [ ] Support version, fork, copy/reference, replace, and usage inspection for
        saved regions; defer arbitrary rewiring until transition-safe edits exist.
- [ ] Add inventory/flag/component state inspector with before/after diff.
  - [x] Add a planner-owned headless state-inspection projection for live and
        serialized stores, raw/structured payloads, bindings, provenance, and
        exact-context friendly/derived fact evaluations.
  - [x] Diff serialized owner stores and sealed persistent-file images by stable
        owner/file identity, payload digest, component manifest, and source
        runtime, alongside active/ended runtime and physical-slot deltas.
  - [ ] Add the visual inspector and before/after route-step diff interaction.
  - [ ] Keep it available on every accepted node and closest rejected-join
        witness in the active graph, including nodes inside expanded regions.
- [ ] Add raw flag catalogue search and friendly aliases.
- [ ] Add obstruction and requirement inspectors.
- [ ] Add theorycraft component-transfer and bypass editor.
- [ ] Show active packs, overlays, exact content identity, mutable language/config,
      provenance, coverage, confidence, and route costs.
- [ ] Add semantic build/language comparison and prevent silent closest-build
      fallback.
- [x] Keep route annotations in route books as non-semantic targets/text,
      separate from mechanics refinement packs.

Deliverable: a planner-specific UI suitable for both simple routes and deep
research, recognizably related to the TAS Route Workbench without inheriting its
timeline assumptions.

### Phase 10 — Evidence and proof integration

- [ ] Match planned edges to trace/tape observations.
- [ ] Validate postconditions and component preservation against snapshots.
- [ ] Promote witnessed edges without erasing lower-confidence alternatives.
- [ ] Attach source, extraction, trace, video, or community citations.
- [ ] Add tools to identify facts used by many routes but supported weakly.
- [ ] Report which facts and obligations are exercised by glitchless story, 100%,
      Any%, and hypothetical route suites.
- [ ] Report extraction coverage separately for topology, hard guards, backing
      stores, actor lifecycle, and physical feasibility.

Deliverable: route confidence is mechanically explainable.

### Phase 11 — Validation catalogue

These are incremental regression targets and research examples, not a demand to
finish every named route before the transition composer is useful. Promote a
slice into release criteria only when it exercises a missing core semantic law.

#### 11A. Fishing Rod

- [ ] Model ordinary vine/hawk/cradle/Uli producers.
- [ ] Model chicken vine bypass, OOB, cradle carry state, reload, and Uli reward.
- [ ] Permit compatible mixing where real predicates allow it.
- [ ] Model Auru's session-level recent-item grant as an alternate producer, with
      HD activation evidence and a separately obstructed/hypothetical SD setup.
- [ ] Prove GCN BiT does not author a rod loss; it merely lacks reachable producers.
- [ ] Collapse all methods into `obtain Fishing Rod` with residual-state safety.

#### 11B. Back in Time/file 0

- [ ] Enter the slotless title-origin runtime file and import its exact initialized
      persistent-domain stores per build.
  - [x] For GZ2E01, model the exact opening guard and audited initialized
        projection: empty item slots, selected Hero's Clothes, Ordon Sword and
        Hylian Shield, health, rupees, Epona-tamed bit, initial return place,
        temporary event bytes, and current stage-memory fields. Equipment
        selection does not fabricate first-item acquisition bits, while the
        sword/shield setters' distinct collection masks are retained;
        unprojected reset fields remain open rather than inheriting pre-title
        values.
  - [x] Enter that GZ2E01 title-origin runtime from a loaded or new non-title
        runtime as one atomic phase-4 lifetime/reset transition; the source
        lifetime ends and the derived file 0 remains slotless.
  - [x] Continue the GZ2E01 title flow through observed A/Start input and a
        pending name-scene request, then apply the second exact save-domain
        initialization only after the active name process/create phase is
        independently observed. This exposes the distinct pre-file-select and
        file-select-open file-0 payloads rather than conflating them.
  - [x] Branch file-select-open state into blank slot 1–3, existing-slot Start,
        or no-card initialization without conflating the process buffer, active
        runtime, and physical slot. Verify branch exclusivity, zero-based header
        writes, slotless blank/no-card behavior, selected image restoration,
        source-lifetime retirement, and explicit non-card metadata carry.
  - [x] Derive populated-slot availability without treating not-sampled state as
        absence, and keep new/existing play-scene destinations pending until
        independent process/world activation.
  - [x] Execute player/horse confirmation and Back paths from observed name
        bytes, timer, reset, and no-file state; reach `selection_end` through the
        actual horse-confirmation writer rather than a test-state injection.
  - [x] Connect the active runtime to physical slot 1–3 only after an
        exact successful save-menu completion; infer the active runtime and all
        available stage banks at execution time, apply exact lantern/event
        projection on a private clone, preserve live and unselected-slot state,
        and prove the failed command changes no slot/header.
- [x] Show physical slots 1–3 separately.
  - Populated slots seal distinct persistent-file images; slot 0 is rejected.
- [ ] Model void/title-state handling and save projection to a chosen slot.
  - [x] Model the chosen-slot projection/load mechanics independently of the
        still-unmodeled void/title-state preconditions and normalization rules.
  - [x] Enumerate the source-backed void, lethal, death-continue, and title-reset
        branches and the backing each consumes. Exact runtime programs remain
        open until restart-parameter decoding and binary/trace evidence land.
- [ ] Model BiTE as a selected component splice into an existing file.
  - [x] Implement the generic selected runtime-component splice into a freshly
        loaded existing file, with destination ownership and mixed provenance.
        The evidence-backed BiTE preservation matrix and setup transitions remain
        open.
- [ ] Allow an unsaved file-0 goal and hypothetical escape overlay.
- [x] Explain exactly which components die when a file-0 lifetime ends.
  - State inspection diffs derive the complete ownership cut from the actual
    before/after states: every source-owned live component and serialized store
    is classified as absent, represented by an equivalent or different
    destination payload, retained illegally by the ended source, or moved
    outside the expected destination. Outside-lifetime components and sealed
    physical images are independently reported as preserved or changed.

#### 11C. EMS to Hyrule Castle/Ganon

- [ ] Produce upper-bound logic path.
- [ ] Introduce geometry/twilight/form/mount obstructions.
- [ ] Encode standard EMS setup.
- [ ] Encode Epona OOB non-twilight constraint.
- [ ] Encode rupee clip as a scoped replacement for the charge-attack approach.
- [ ] Show how route results refine as obstruction knowledge is enabled.

#### 11D. Local-bank rebind

- [x] Snapshot a Forest Temple-bound payload.
- [x] Add hypothetical preservation and Temple of Time rebind.
- [x] Verify raw bytes remain identical while aliases change.
- [x] Derive downstream effects only from the new interpretation.
- [x] Display mixed provenance and hypothesis dependency.
  - The state projection retains the original extracted/observed provenance plus
    the transform action, while the solve report names both the active
    refinement pack and the hypothetical technique step. These are headless
    planner surfaces; the corresponding visual treatment remains a Phase 9 UI
    task.
- [x] Remove overlay and verify base reachability returns unchanged.

#### 11E. Fanadi save-location lock

- [x] Model SavMem writer, event/switch guards, and placements.
- [x] Model `NO_TELOP` as a write gate with observed lifetime.
- [x] Model Fanadi setter/clearer and Ooccoo/setup prerequisites.
- [x] Retain the exact last successful `PlayerReturnPlace` write.
- [x] Model savewarp as a reader of the held value.
- [x] Search for setup orderings and explain intervening writes.
- [x] Add a hypothetical Fanadi-access bypass and verify earlier return locations
      become usable without modifying the core lock mechanism.
  - A source-shaped acceptance fixture separates the decoded Castle Town SavMem
    placement, its inside/event/switch activation, the `NO_TELOP` writer gate,
    Fanadi's setter and normal clearer, Ooccoo ownership, and two savewarp
    consumers. The normal route must execute the Castle Town writer before the
    lock and its savewarp proof reads `CASTLE_TOWN`. Without an overlay, the
    earlier `ORDON_SPRING` return is unreachable. A removable hypothetical
    direct-Fanadi technique avoids only the intervening placement; the unchanged
    setter, gate, and savewarp reader then preserve and consume `ORDON_SPRING`,
    with the hypothetical evidence retained in the route proof.

#### 11F. Faron-twilight return research

Source-audit checkpoint: `docs/route-planner/faron-twilight-return-audit.md`
records the GZ2E01 scene identities, portal-table route, form/twilight blockers,
reload and file-lifetime distinctions, and an exhaustive upper-bound scan of
target-naming `SCLS` records from every room archive. The acceptance fixture
keeps executable, obstructed, feasibility-unknown, and hypothetical candidates
distinct. It also corrects the route premise that normal BiT save/load enters
Ordon Spring: its destination is `F_SP108`, while `F_SP104` requires an
independent portal, return-place, restart, scene-change, or transfer producer.

- [x] Define goals for Goats map, Ordon Village, outside Link's house, Link's
      house, and Ordon Spring while Faron remains in twilight.
- [x] Enumerate SCLS, spawn, savewarp, void, death, title, cutscene, actor, and
      technique-provided incoming transitions to each target.
- [x] Apply twilight, form, collision, and activation obstructions per approach.
- [x] Include BiT/BiTE, held return place, wrong-state respawns, OOB, and proposed
      component-transfer hypotheses where scoped plausibly.
- [x] Report reachable, blocked, and unknown candidates with exact missing
      obligations instead of flattening them into “no.”

#### 11G. Lanayru spirit and Vessel of Light

Source-audit checkpoint: `docs/route-planner/lanayru-spirit-vessel-audit.md`
now decodes the supplied GZ2E01 resources exactly. `F_SP115/R01` layer 13 has
`Seirei` parameters `0x0000c102` and a colocated type-0 `SwAreaC` writer for
F_SP115 stage-memory switch `0x0c`; the room argument does not make switch IDs
below `0x80` room-local. Normal layer selection derives 13 from uncleared
Lanayru twilight plus `M_032` (`0x0880`). Message group 8 flow 21 checks
`F_0615`, then item `0xa3`, requests presentation through event 1, and on the item-owned
follow-up writes `F_0615`, reasserts the Vessel bit, and sets save-switch 105.
The acceptance fixture preserves each of those steps, rejects a wrong layer in
established mode, admits an explicit hypothetical layer-13 respawn only in
research mode, and proves that transferred Vessel and `F_0615` bits affect
different branches. Other builds still require original-data comparison.
The audit also records the exact persistent byte/mask coordinates, the
switch-area creation/outside clearers, the exhaustive committed writer/consumer
set, and the distinct 16-tear `KYTAG04` -> switch 13 -> point-20 twilight-clear
sequence.

- [ ] Locate the spirit actor/event flow and every build-specific placement.
  - [x] Pin the exact GZ2E01 F_SP115 STAG, R01 layer-13 `Seirei`, US-English
        group-8 resource, and flow-21 label in a compiled message-entry contract.
        Other build/language placements remain unverified.
- [x] Identify the raw event bits, temporary bits, room/layer, form, twilight,
      approach, and cutscene prerequisites for the spirit to appear.
- [x] Separate appearance, interaction, cutscene start, Vessel grant, and post-grant
      state into distinct transitions rather than one milestone.
- [x] Identify all writers and consumers of the Vessel and tear-count state.
- [x] Test whether alternate entrances, wrong layers, wrong-state respawns, or
      component transfers can satisfy or bypass individual prerequisites.
- [x] Produce both a friendly explanation and the exact raw predicate for each
      supported build, with unknown conditions called out explicitly. GZ2E01 is
      currently the sole registered supported identity; the audit now separates
      load, visibility, interaction, offer, grant, and follow-up predicates and
      labels shared attention/presentation execution as unresolved obligations.

#### 11H. Keyed dungeon transition

- [x] Extract one representative small-key door's encoded destination without
      making it immediately executable.
- [x] Derive the bound dungeon key count from the live per-stage backing store.
- [x] Model key pickup provenance independently from the fungible count.
- [x] Audit and model the door actor's guard, key consumption, persistent unlock
      write, live animation/collision, and reload reconstruction.
- [x] Verify any key from the same bound dungeon store can satisfy the door.
- [x] Add a hypothetical key-store preserve/rebind overlay and verify it opens
      routes only through backing-store semantics, with hypothesis provenance.
- [x] Verify an OOB route that avoids the door does not falsely mark the door
      unlocked or consume a key.
  - Fact-catalog schema v5 adds unambiguous bound structured-field and raw-bit
    references: component kind plus exact backing binding, followed by either a
    field or byte range/mask. This lets extracted stage-memory bytes derive the
    same semantics without a hard-coded live component ID. The keyed-door fixture
    uses the structured form for `small_keys` and persisted unlock state, rejects
    zero-key and wrong-bank states, and fails unknown when more than one component
    claims the same backing. Two independently identified pickups feed the fungible count; the
    door consumes one key and writes persisted unlock and live actor state. A
    hypothetical dungeon-bank rebind enables a different dungeon's door only
    through the changed binding, while the OOB avoidance edge mutates neither
    the count nor unlock state.
  - Mechanics schema v15 adds binding-sensitive unsigned adjustment and masked
    raw write/invalidation operations. The
    raw stage-memory regression uses the audited `dSv_memBit_c` key byte, proves
    pickup/consumption history, and rejects wrong-bank, unknown, underflowing,
    and ambiguous targets atomically. See
    `docs/route-planner/bound-stage-memory-semantics.md`.
  - The GZ2E01 source audit binds Forest Temple `Door[1]` to the raw
    `yodoor` placement (`0x6c102201`, switch `0x0b`) and decodes its room-1 to
    room-2 adjacency without granting traversal. Its acceptance fixture keeps
    event offer, action-8 switch write, transient key delta, persistent key
    flush, keyhole completion, collision release, open/cross/close animation,
    and reload reconstruction separate. See
    `docs/route-planner/gz2e01-forest-temple-small-key-door-audit.md`.
  - Native learning observation v27 now makes the same distinction at runtime
    for every loaded `DOOR20`: authored rooms/options/switches/events remain
    separate from live lock, action, side, collision-release, open/close,
    stopper and debounce state. The reader recomputes the authored decoding
    from the raw retained placement, while the actor-catalog parity walk makes
    profile-offset drift observable. This is evidence for future activation
    and reconstruction checks, not a rule that all door families share
    `daDoor20_c` semantics.
  - Native actor view v9 and its independently selectable learner projections
    now preserve that exact profile-bound state with explicit component and
    nested-switch masks. The direct complete-set adapter exposes it only as
    `actor_door20`, so held-out family ablations can measure its contribution
    without encoding a preferred door or treating unrelated door profiles as
    layout-compatible.

#### 11I. Auru recent-item grant

- [x] Model `mGtItm` as a session/process storage site separate from save-file
      inventory and `mPreItemNo`.
- [x] Enumerate presentation/chest/show-item writers and prove which boundaries
      preserve or reset the value.
- [x] Model file A writing an item ID, file load preserving session state, and file
      B consuming it through generic get-item semantics.
- [x] Decompose Auru's normal memo path, pending item actor, `DEFAULT_GETITEM`
      handoff, and broken path that avoids the memo overwrite.
- [x] Author the talk-volume/outside-trigger/player-control obligation.
  - The interaction fixture requires Auru's live actor, inclusion in the talk
    volume, exclusion from the cutscene trigger, player control, and the talk
    action; missing actor or geometry observations remain unknown.
- [x] Mark the known HD targeting resolver as external build evidence; keep the SD
      candidate surfaced as obstructed or unknown rather than absent.
- [x] Add a hypothetical SD geometry/interaction resolver and verify arbitrary
      recent-item producers become usable without editing Auru's grant rule.
  - The solver fixture labels the HD resolver as community/external evidence,
    leaves the SD transition in its blocked frontier, and adds a removable
    hypothetical SD refinement. The unchanged `SetBitFromValue` grant reaches
    both Fishing Rod (`0x4a`) and Auru's Memo (`0x90`) goals depending only on
    the session recent-item producer.
- [x] Model the optional memo-preservation sidehop/backflip interruption as a
      separate frame-exact microtransition.
  - A synthetic acceptance fixture now keeps the pending `mPreItemNo` handoff,
    session `mGtItm`, and generic inventory grant distinct. The established
    path overwrites `mGtItm` with Auru's Memo before the shared grant; a
    removable, explicitly hypothetical one-frame interrupt witness preserves
    the prior item instead. Research mode can use that witness, the default
    evidence policy cannot silently promote it, and deleting it leaves the
    temporal obligation in the blocked frontier. The exact retail frame and
    action still require source/runtime evidence rather than being claimed by
    this fixture.
  - The source/boundary audit finds only the shared present-demo and
    treasure-box-demo helpers writing `mGtItm`; show-item/catch paths instead
    write `mPreItemNo`. The acceptance matrix preserves the session component
    across every modeled in-process room, stage, reload, save/load, title,
    wrong-state, and dialogue boundary; later presentations overwrite it and an
    explicit fresh-process boundary reinitializes it. See
    `docs/route-planner/auru-recent-item-store-audit.md`.

#### 11J. Text Displacement to Goron Mines

- [x] Extract raw shared message-progress bits and their generic flow-node writers,
      readers, and cleanup paths.
  - The planner-owned BMG extractor preserves the raw `mQueryList` index and
    separately resolves the numbered query handler, derives event010 set,
    event011 clear, and query011 branch-when-clear accesses, and attaches the
    exact packed backing coordinates for shared flow-control A–J. The source
    audit also records both normal event cleanup and Ooccoo-warp cleanup. See
    `docs/route-planner/text-displacement-message-state-audit.md`.
- [x] Model at least Coro, Auru, Yeta, and Ooccoo producer routes as distinct
      interruption/advancement proofs where evidence exists.
  - The acceptance fixture writes the same source-audited raw A/B bits through
    four independent causal programs: Coro and Yeta require their own one-frame
    microtraces, Auru requires two interactions inside the talk volume but
    outside the cutscene trigger, and Zombie Ooccoo requires a one-time death
    pull followed by a second advancement. Removing the Coro witness blocks only
    that producer; overlapping Auru's trigger fails the spatial obligation. See
    `docs/route-planner/text-displacement-producer-model.md`.
- [x] Identify Gor Coron's exact displaced-branch predicate and downstream
      persistent event/switch writes.
  - GZ2E01 group 3 flow 6 first requires M031 clear. A set C jumps to flow 9;
    otherwise a set B writes C and ends through event 6/cut 4. A later flow-9
    pass sets A when clear, and the following pass reaches node 190 to write
    persistent label 62/M029 before nodes 189 and 208 clear A/B/C. Thus a
    normal B-bit producer feeds three ordered talks, not one synthetic
    `A && (B || C)` rule. The extractor emits persistent-bit and switch accesses
    as typed records.
- [x] Model invisible wall, elevator authorization, live NPC blockers, and room
      reload reconstruction independently.
  - Separate rules cover live GRA_WALL deletion, the type-4 guide Goron's
    switch-0x6f gate walk, the `dmele` actor's independently detected heavy
    pressure/event movement, room-reloaded Goron state, and the witnessed
    roll-past alternative. M029 does not directly authorize the elevator actor.
- [x] Start with the Goron Mines encoded transition visible but non-executable;
      discharge each authorization and physical obligation causally.
  - The R_SP110 SCLS 0 edge remains a first-class encoded transition while a
    blocked witness names its wall and live-Goron obstructions. The separate
    elevator approach can block reaching the hall, but is not falsely attached
    to SCLS 0.
- [x] Verify the solver can work backward from the entrance to all enabled
      producers of the required text-bit pattern.
  - Backward relevance from D_MN04 includes all four Coro/Auru/Yeta/Ooccoo raw-bit
    producers and all three ordered Gor Coron consumer actions.
- [x] Verify removing one producer or adding a hypothetical new interrupt changes
      reachability without changing the Goron consumer or entrance rules.
  - Isolating then removing each enabled producer blocks the route. A new
    hypothetical B-bit writer restores it only under research evidence policy,
    with byte-for-byte-equal consumer and entrance records. See
    `docs/route-planner/text-displacement-message-state-audit.md`.

#### 11K. Glitchless story route

- [ ] Author one reasonable glitchless route from a fresh file through final-boss
      completion using ordinary movement, quests, dungeon progression, and combat.
- [ ] Require every room transition, NPC interaction, cutscene, item acquisition,
      key expenditure, boss-key door, and dungeon exit to replay through the same
      causal model used by theorycraft routes.
- [ ] Model Forest Temple monkey rescues individually, including the current
      rescued/following set and every gate or traversal consequence that consumes
      it; expose `rescue required monkeys` only as a collapsible subgraph.
- [ ] For each dungeon, account for the selected small-key pickups and every
      consumed key rather than jumping directly between milestone rooms.
- [ ] Compare every propagated snapshot against expected inventory, flags, key
      count, room/layer, and relevant actor state.
- [ ] Treat any apparently skippable mandatory step as a missing guard/obstruction
      until proven to be a real alternate route.

#### 11L. Glitchless 100% route

- [ ] Define an explicit versioned 100% completion contract rather than assuming a
      universal category definition.
- [ ] Author a reasonable completion order while allowing the solver to expose
      equivalent reorderings.
- [ ] Include all category-required collectibles, upgrades, quests, dungeon items,
      small keys, locked doors, optional rooms, and persistent completion flags.
- [ ] Use aggregate goals only as derived views over individually witnessed
      acquisitions and consumptions.
- [ ] Verify no collectible is double-counted, no consumed key survives, no opened
      door relocks incorrectly, and no one-time reward can be collected twice.
- [ ] Use the route as a coverage report for unaudited writers, guards, actors,
      message flows, and transitions.

#### 11M. Any% graph and suspicious-state audit

- [ ] Author a versioned standard Any% route book only after the glitchless route
      can replay coherently.
- [ ] Represent alternate known sequence breaks as graph branches sharing the same
      underlying facts, obstructions, and technique transitions.
- [ ] Compare solver-selected plans with known category routes and explain every
      divergence in cost, enabled technique, or missing knowledge.
- [ ] Run bounded queries for milestones, items, maps, layers, and flag combinations
      that should appear implausibly early or mutually inconsistent.
- [ ] Classify each suspicious result as a genuine route, hypothetical dependency,
      incomplete feasibility evidence, extraction error, or authoring/model bug.
- [ ] Preserve promising unexplained results as reproducible research cases rather
      than automatically suppressing them.

#### 11N. Build/language fact-pack comparison

- [ ] Generate, serialize, reload, and deterministically reproduce fact packs for
      at least two exact content identities from verified `orig/` inputs.
- [ ] Generate separate resolved contexts for PAL English and PAL French resources
      without pretending they are different discs or universally equivalent.
- [ ] Audit the cannon-payment message flows and express any route divergence as
      graph guards/writes/effects plus remaining feasibility obligations, not a
      named version-skip Boolean.
- [ ] Verify an exact PAL French query can use only its evidenced divergence, an
      exact PAL English query cannot inherit it, and a portable multi-context
      query excludes it or supplies a distinct valid witness for every context.
- [ ] Remove the source `orig/` inputs and verify the planner can reproduce the
      same queries from the derived packs and their recorded provenance.
- [ ] Perturb an input digest, fact-pack schema version, and extractor version in
      fixtures; each mismatch must invalidate or migrate explicitly rather than
      reuse stale facts.

#### 11O. Zelda tower cutscene failure and retained return place

- [ ] Trace the post-Zelda cutscene as ordered phases, including its Castle Town
      scene change, actor/archive loads, event writes, return/restart-place
      writers, and cleanup.
  - [x] Split the sequence at its real stage boundary. R_SP107 room 3 layer 8
        selects `Demo07_01`; normal `demo07_01` completion enters R_SP301 room 0
        layer 8. Only that downstream room selects `Demo07_02`, whose normal
        completion enters Castle Town. Do not collapse these into one event.
  - [x] Extract the exact R_SP107 `demo07_01` wrapper and package/outer behavior.
        Its normal exit is R_SP301, its skip exit is R_SP107 room 3 spawn 1, its
        finish flags are `[19, -1, -1]`, and the generic resolver emits its five
        phase/dispatch transitions without `demo07_02`-specific IDs or labels.
  - [x] Extract the exact outer retail event topology: R_SP301 layer 8 selects
        `Demo07_02`; `demo07_02` runs `demo07_02.stb` plus map-tool ID 4; normal
        completion selects SCLS 1 to Castle Town and event skip selects SCLS 2
        back to Zelda's tower. The STB internals and writer ordering remain open.
  - [x] Canonically join and validate the topology, including exact source
        digests and explicit unresolved coverage for the STB program, failed
        resource flow, and return-place writers.
  - [x] Add planner-owned exact DOL function evidence and prove that GZ2E01
        `dComIfGp_ret_wp_set__FSc` is exactly one `blr`. The room-loader call
        therefore preserves arbitrary incoming return-place values; other
        possible JStudio/glitch-path writers remain open.
  - [x] Structurally decode and seal `demo07_02.stb`: 30 outer blocks, 200 FVB
        functions, 29 object streams, 387 commands, and 817 paragraph headers.
        Keep executable-specific meanings out of the structural schema.
  - [x] Add exact-content JStudio adaptor profiles and a separate semantic
        resolver. The GZ2E01 profile resolves all 695 object-specific paragraphs
        through 29 audited selector rules while retaining 122 reserved controls;
        it types actor shape/animation ID writes and the three demo-message IDs
        without implying actor execution, message completion, or scene change.
- [ ] Capture normal completion and actor-corruption/archive-load-failure paths;
      identify the last confirmed operation and every flag or writer that becomes
      skipped versus unknown.
  - [x] Correct the witnessed failure site from `Demo07_02` to `Demo07_01`.
        The primary video visibly reports allocation/resource failure for
        `Demo07_01.arc`, then demonstrates a later save/reload into Castle Town.
        The recording does not by itself prove an exact disc build, every
        intervening flag, or the retained return-place bytes.
  - [x] Resolve the exact GZ2E01 all-STB-lookups-missing branch: archive request
        rejection clears the demo name, negative sync continues room init, STB
        lookup falls through demo/room/stage archives, parse returns before the
        demo-mode write, the exact PLAY cut has no EventFlag write, and mode zero
        completes PACKAGE. Keep the corruption producer, actual runtime branch
        selection, and other return-place writers unresolved.
  - [x] Resolve the exact outer event-manager branch table without selecting a
        branch for the corruption path: prove PACKAGE PLAY -> zero-timer WAIT ->
        event finish flag 5, then emit exact-context candidate transitions where
        clear suppression/skip selects Castle Town, active skip selects Zelda's
        tower for this REVT type, and suppression prevents either scene change.
- [x] Model actor corruption as the producer of the failed-load/exceptional-flow
      predicate, not as a direct Castle Town warp. The exact-context hypothesis
      transition has unknown evidence and explicit failure-site, all-STB-miss,
      and completed-prefix requirements; its only effect writes the named
      failure predicate, never location or return place. The compiler is
      event-generic and binds this hypothesis to `demo07_01`; it must not attach
      the witnessed failure to downstream `demo07_02`.
- [ ] Verify whether any writer other than the proven room-loader no-op can run
      on the actor-corruption path, and that ordinary savewarp subsequently
      reads the retained value from Zelda's tower.
  - [x] Identify the exact unlayered R_SP107 room-3 `Savmem` placement and decode
        its ordinary writer: parameters `0x0000ff01`, event-label index 45 must
        be set, event-label index 47 must be unset, switches are unguarded, and
        `NO_TELOP` must be clear. When it executes, it writes R_SP107 room 3,
        spawn 1. Whether this actor executes after the witnessed allocation
        failure remains an explicit runtime question.
  - [x] Source-audit the ordinary savewarp reader: `dComIfGs_gameStart` reads the
        persistent player return-place stage, room, and player-status/spawn into
        `setNextStage`; it does not synthesize Castle Town from the current map.
  - [x] Compile those facts into a standalone exact-context mechanics catalog:
        one atomic tower `Savmem` writer, raw M_012/M_014 backing guards, a raw
        `NO_TELOP` gate, three reader records, and one dynamic savewarp
        transition. Keep actor execution as an explicit live component field,
        so corruption cannot silently count as either execution or suppression.
  - [x] Decode raw JStudio data sent to exact `d_actN` generic actors instead of
        discarding every type-`0x80` paragraph as semantically reserved. The
        standalone `demo-actor-program/v1` artifact implements the retail
        status-51 packed-word decoder and distinguishes persistent event-bit,
        temporary event-bit, and other operations. Exact `demo07_01.stb` has
        three generic actor streams, 14 raw writes, 25 packed commands, and no
        persistent or temporary event-bit write; it therefore does not set
        M_012 through `daDemo00_c`.
  - [x] Source-audit static room-actor reconstruction after demo-archive failure.
        The room loader continues into `dStage_dt_c_roomReLoader`, but each
        placement separately allocates an actor append record and process-create
        request; either allocation can fail. Those failures are not checked by
        the room-data loop, so a missing `Savmem` execution is mechanically
        possible without preventing the room from finishing initialization.
        Archive allocation failure alone does not prove this happened because
        archive and actor-request allocations use distinct backing heaps.
  - [ ] Capture the `Savmem` placement's append allocation, process request,
        create/execute result, and M_012/M_014/NO_TELOP values on the witnessed
        corruption setup. Until then, do not select actor-allocation failure as
        the explanation merely because source proves it is possible.
  - [ ] Join decoded generic-actor event-bit effects into ordered cutscene state
        operations, while retaining actor creation/execution as a separate
        runtime precondition rather than treating authored commands as executed.
- [ ] Vary the incoming return place across a witnessed actor-corruption trace;
      the GZ2E01 room-loader call is already proven to preserve it generically,
      but the complete failure suffix is not yet bounded.
- [x] Keep the route unknown in established mode until the relevant partial
      effects and scene-change branch have source or trace evidence. The
      producer remains `unknown` (not merely hypothetical), is rejected by both
      standard evidence policies, and requires an explicit refinement/replacement
      before any what-if search can traverse it.

---

## 7. Regression and acceptance matrix

Every fixture specifies exact content identity, selected language/configuration,
start environment, active packs, query, expected classification, and key proof
facts.

| Fixture | Required assertion |
| --- | --- |
| Normal fresh file | Ordinary milestones and inventory progress correctly. |
| BiT file 0 | Runtime is memory-backed with persistent-domain contents; no physical slot 0 exists. |
| File-0 save | State projects only to slot 1–3 and applies save policy. |
| Unescaped file 0 | Lack of slot backing is not equated with dead; hypothetical continuation remains representable. |
| GCN rod after BiT | All enabled rod producers fail causally; no authored loss marker exists. |
| Rod via Auru recent-item state | Shared grant semantics exist independently of whether the active build has a feasible Auru setup. |
| Rod route collapse | Alternative proofs collapse only when residual differences are safe. |
| Lakebed graph regions | One-trip, leave/return, and partial-circuit routes are independently constructed subgraphs over the same state-transition graph. |
| Region reuse | Copying or referencing a saved region after another state reevaluates every enclosed transition and grants no inherited authority. |
| Invalid join | The planner rejects a non-composing transition pair and identifies a missing producer, obstruction resolver, unknown obligation, or context mismatch; no force-connect edge exists. |
| Live state inspection | Every accepted step and closest rejected witness exposes its exact state, provenance, and before/after diff, including inside a collapsed region. |
| EMS upper bound | Logical authorization appears before geometry is supplied. |
| EMS obstruction | Physical blocker removes route only from feasible projection. |
| Epona OOB | Route is rejected while twilight/mount predicate fails. |
| Local bank normal flow | Stored stage entries are runtime-file scoped; outgoing bytes commit and destination bytes load under the proper explicit binding. |
| Hypothetical rebind | Payload remains stable, aliases and consequences change. |
| BiTE splice | Only declared components cross; provenance remains mixed. |
| Fanadi gate off | Passing SavMem updates return place. |
| Fanadi gate on | Passing SavMem preserves the prior value. |
| Fanadi ordering | Intervening pre-lock writer determines held value. |
| Hypothetical Fanadi access | New access edge exposes prior values without editing gate rules. |
| Lanayru spirit appearance | Appearance, interaction, and Vessel grant have separately verified predicates. |
| Encoded keyed door | Destination alone is not executable while its hard guard is unsatisfied. |
| Fungible small keys | Different pickups feed the same bound count and can satisfy the same door. |
| Key consumption/unlock | Count, persisted unlock, and live actor state update independently. |
| Key-store rebind hypothesis | Target door derives access from transferred backing state and reports the overlay. |
| Door-avoidance OOB | Alternate route neither consumes a key nor mutates the door's persisted state. |
| Auru session state | Item prepared on file A remains available to the modeled Auru consumer on file B. |
| Auru normal path | Memo creation overwrites recent-item state and grants the memo normally. |
| Auru HD activation | External HD targeting evidence resolves talk-inside/outside-trigger geometry. |
| Auru SD base | Candidate is surfaced as obstructed or unknown, never silently absent or reachable. |
| Auru SD hypothetical | Adding only a spatial resolver exposes all compatible recent-item producers. |
| Dialogue interrupt | Microtrace preserves declared message/trigger state and executes no skipped cleanup. |
| Text-bit producer search | Goron consumer traces backward to every enabled Coro/Auru/Yeta/Ooccoo producer. |
| Goron entrance residuals | Unlock state does not silently remove still-live NPC/collision blockers. |
| Glitchless story replay | Every step reaches its next state with balanced keys and required quest/dungeon facts. |
| Forest Temple monkeys | Each rescue is causal; the collapsed group preserves count/set and downstream effects. |
| Glitchless 100% replay | Every requirement is individually produced once and the final contract is derived. |
| Any% comparison | Solver divergence from a reference route has an explicit cost, rule, or knowledge explanation. |
| Suspicious early state | Implausible reachability produces a reproducible proof and triage classification. |
| Overlay isolation | Disabling an overlay restores identical base results. |
| Unknown geometry | Query returns unknown, not impossible or reachable. |
| Exact context scope | Unsupported techniques and language-specific facts never appear through group leakage. |
| Verified fact-pack identity | User labels cannot override input digests; unknown or stale packs do not silently load. |
| PAL language split | French-only message-flow behavior does not leak into English or proven-portable queries. |
| Zelda cutscene archive failure | Partial cutscene writes are preserved/unknown at the witnessed boundary; retained return place drives the later ordinary savewarp. |
| Derived-pack replay | Exact queries reproduce without `orig/` once the matching generated pack exists. |
| Cross-context semantic binding | Friendly facts compare across contexts while retaining exact raw bindings and provenance. |
| Planner/workbench schema boundary | Cycles, AND requirements, and hypothetical edges survive projection without masquerading as TAS timeline segments. |
| Thin browser contract | Invalid authoring commands are rejected by Rust validation and browser-local state cannot alter solver truth. |

Also require:

- Golden state diffs for room, stage, void, save, load, title, and splice
  boundaries.
- Schema round trips and stable IDs.
- Solver determinism for identical packs and cost policies.
- Property tests that no undeclared component survives a boundary.
- Property tests that collapsed plan regions preserve all valid continuations.
- Conflict tests for competing aliases, refinements, and obstruction claims.
- Determinism tests for extraction and resolved common-base-plus-delta packs.

---

## 8. Open research questions

These should remain explicit unknowns until evidence closes them:

- Exact file-0 component initialization and every build difference.
- Exact BiTE preservation/clearing matrix and whether multiple setups share it.
- Other known respawn-with-wrong-flags or wrong-bank glitches.
- Whether any existing technique can preserve/rebind live stage memory across a
  context that normally replaces it.
- Complete semantic mapping for stage, zone, dungeon, and temporary flag banks.
- Executable rider/caravan/external-switch gate modeling after their audited
  event-bit, switch-domain, paired/transient, and destructive state is added;
  cross-build keyed-family equivalence also remains unproved.
- Exact Fanadi/Ooccoo glitch sequence, `NO_TELOP` lifetime, clear conditions, and
  build support.
- All return-place writers encountered on paths to Fanadi.
- Whether actor unload/order can extend or alter the write gate.
- Exact Auru broken-flow path, source/destination restrictions, `mGtItm` reset
  behavior, and build-specific interaction geometry despite the shared mechanism
  visible in SD source.
- Exact mappings from message-flow assets to the shared temporary progress bits,
  and every abnormal exit that preserves or clears them.
- Complete executable/resource digest catalogue and semantic equivalence matrix
  across supported revisions and selectable languages.
- Exact PAL French cannon-flow divergence, payment guard/debit behavior, language
  switching constraints, and any residual physical or temporal setup.
- Exact one-frame interruption windows and player-control/input behavior across
  GCN, Wii, and externally documented HD routes.
- Exact Gor Coron displaced predicate, entrance event/switch effects, live NPC
  residual state, and reload requirements.
- Which Faron-twilight target maps have encoded but physically inaccessible
  incoming transitions.
- How much geometry feasibility can be derived automatically versus authored.
- Which actor-instance fields can be observed generically and which require
  actor-specific adapters or trace annotations.
- Unresolved build differences or hidden actor/event conditions in Lanayru's
  appearance and Vessel-of-Light sequence after the 11G audit.
- Evidence thresholds for claiming `unreachable_under_model` in partially audited
  regions.

---

## 9. Definition of done

The first useful release is complete when:

- The blueprint-style browser can create, open, drag/connect, validate, save,
  save-as, and reload a route through the authoritative Rust service.
- A route is assembled exclusively from typed transitions applied to exact states;
  reachability is never an authored loss list or an unvalidated visual connection.
- The provider catalogue includes representative physical travel, encoded
  loading-zone/door, event/cutscene, savewarp, game-over/void, and technique
  transitions. Unsupported providers remain explicit gaps rather than blocking
  the entire release.
- Selecting any accepted node or closest rejected-join witness shows its exact
  location, inventory, relevant flags/components, bindings, provenance, and diff
  from the prior state.
- An invalid join names the missing state producer, active obstruction and known
  resolvers, unknown obligation, or exact-context mismatch. The only escape hatch
  is an explicit evidenced or hypothetical refinement, never force-connect.
- Any selected graph region can be named, nested, collapsed, and saved without
  becoming a transition or changing solver truth. Fork/copy/reference workflows
  may follow after the first usable slice.
- Reusing a region after another state reevaluates every enclosed transition.
  One-trip, leave-and-return, and partial-dungeon subgraphs coexist as ordinary
  graph fragments rather than implementations of a parent goal.
- Authorization, physical obstruction, known bypass, witnessed execution, and
  unknown coverage remain distinct and visible.
- Exact content identity and active fact/refinement packs are sealed into the
  query and proof; unsupported contexts never inherit neighboring facts silently.
- The headless solver and authoring UI round-trip the same graph, state, rejection,
  and proof identities deterministically.
- Several pre-shipped demonstrations load as ordinary editable route books and
  visibly recompute downstream state when their transitions change.
- At least one ordinary multi-room route and one obstruction/resolver or
  leave-and-return fixture demonstrate the complete compose, inspect, collapse,
  reuse, and re-expand loop.

Complete glitchless story, 100%, Any%, every dungeon, and every named glitch are
validation-catalogue growth targets. They are not release gates for the core
transition composer.
