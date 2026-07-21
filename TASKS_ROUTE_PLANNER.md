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

The primary result of a query is one of:

1. `reachable`: the active model contains a complete feasible route.
2. `unreachable_under_model`: the search space is sufficiently closed to prove
   that no route exists under the selected rules and knowledge packs.
3. `unknown`: a route depends on unresolved geometry, incomplete behavior, an
   unaudited state transformation, or an intentionally open research question.

“No known route” must not silently become “impossible.”

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
- Snapshot v2 keeps observed slot descriptors separate from verified serialized
  slot contents, permits unknown runtime origin/backing and player-control state,
  and diffs slot observation changes independently from slot-content changes.
- Native snapshot sequences accept an explicit incoming boundary kind, emit
  semantic/component/raw-byte diffs, and seal each snapshot into a contiguous
  digest-linked chain. This makes room/stage/save/load/void/title/BiT/BiTE test
  captures comparable without inferring the boundary label from coincidental
  state changes; representative captures for each boundary remain outstanding.
- Extracted world-facts schema v1 now compiles an exact content identity,
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
- Refinement-pack schema v3 and composed-catalog schema v3 now live entirely in
  the planner workspace. `route-planner compose` validates canonical packs,
  dependency digests, conflicts, deterministic precedence, explicit
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
- Planner graph schema v2 is an independent, canonical projection of fact and
  mechanics catalogs. It exposes typed fact, goal, transition, obligation,
  obstruction, resolver, technique, writer/gate/reader, reconstruction, and
  microtrace nodes with causal edge kinds. Every nested predicate is projected
  as an ordered `all`/`any`/`not`/fact/comparison tree inside its own collapsible
  region, so the editor can summarize requirements without flattening or losing
  their interchangeability. The planner-owned `route-planner project-graph`
  command emits this artifact from either base or composed catalogs.
- Planner service schema v3 provides a typed JSON-lines transport owned by the
  standalone planner runtime. `route-planner serve-stdio` accepts refinement and
  route-book validation/editing, catalog composition, graph projection, state
  inspection, exact-context solve, and portable multi-context solve requests;
  every response retains its request ID and
  returns either a typed payload or a structured field/detail error. It imports
  no Huntctl CLI, TAS timeline, WorkbenchGraph, playback, or browser-state types.
- State-inspection schema v1 preserves the full execution-state document—live
  components, serialized owner stores, bindings, lifetimes, provenance, gates,
  cleanup, runtime-file identity, physical slots, location, and player state—
  while evaluating every friendly alias and derived fact under the selected
  exact context and evidence policy. `route-planner inspect-state` and the
  service protocol expose the same projection, so raw inventory/flag bytes and
  their semantic names remain inspectable together.
- Route-book schema v1 is a validated, exact-context-scoped preference layer over
  mechanics. It can name goals and path constraints, reference ordered actions,
  define alternative methods and nested plan regions, request pin/ban/prefer
  behavior, and attach non-semantic annotations. It deliberately has no effects
  or loss fields: every referenced action and predicate must validate against
  the fact/mechanics catalog. `route-planner validate-route-book` and the typed
  service validate books without composing them into mechanics.
- Planner graph schema v2 can optionally project a route book as distinct plan
  region, method, and reference-step nodes connected to the underlying causal
  actions. Region outcomes and step pre/postconditions remain nested predicate
  graphs. A book's collapse policy is surfaced, but the catalog projection does
  not mark a region collapsed before a solver proof establishes continuation
  equivalence or supplies residual-state differences.
- Route-book edit-batch schema v1 provides revision-checked authoritative
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
  digest into solve-report schema v4. Unsupported writer/microtrace execution
  fails closed rather than being silently ignored.
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
- Solve-report schema v3 retains active/unknown obstructions, selected
  resolvers/techniques, discharged/outstanding/introduced obligations, and
  semantic state identities on successful steps. Failed searches keep the
  deterministic closest witnessed blocker state for each transition, including
  guard truth and unknown requirements; portable solve-report schema v3 embeds
  those per-context proofs.
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
- Mechanics-catalog schema v4 adds explicit cutscene scene-change and resource-
  load-failure transition classes plus masked raw-knownness invalidation. This
  supports partial execution records that preserve confirmed prefix bytes while
  marking only unaudited suffix effects unknown; extracting concrete cutscene
  phase programs remains open.
- Soft action and method preferences use deterministic lexicographic search:
  minimize action depth first, then maximize total preference weight among
  equal-depth routes. Each directive contributes at most once, preference and
  method progress participate in search identity, and the result reports both
  the score and satisfied directive IDs; loops therefore cannot farm weight.
- `CostAtMost` constraints accumulate every executed technique's authored
  `RouteCost` axes, retain the totals in search identity, prune paths exceeding
  the strictest active per-axis maximum, and report the reached route's totals.
  Transitions and resolvers currently have no cost field in mechanics schema v4,
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

### 5.16 Route books and fractal plan regions

A route book is a curated preference over the causal graph, not a second mechanics
database. Ship or support books such as:

- Normal completion.
- Glitchless story reference routes.
- Versioned glitchless 100% reference routes.
- Standard Any% variants.
- Back in Time research.
- HD cross-file/item-transfer routes.
- Hypothetical/theorycraft showcases.

Plan regions are nested, single-entry/single-exit proof regions when possible:

```text
Obtain Fishing Rod
  selected proof: chicken bypass + hawk + ordinary Uli reward
  alternatives: ordinary cradle quest, glitched cradle carry, Auru recent-item
  grant through a build-feasible or hypothetical activation route
```

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

### Phase 0 — Evidence inventory and terminology

- [ ] Catalogue exact supported builds, revisions, disc/executable/resource
      digests, available language bundles, and stable content IDs.
- [ ] Audit how language/configuration is selected, persisted, changed, and used
      to select message or other resource variants.
- [x] Define the supported-input policy: pre-generated known fact packs for normal
      use, optional `orig/` extraction for verification/new inputs, and explicit
      unsupported behavior for unknown identities.
- [ ] Inventory extracted world-data schemas and their missing fields.
- [ ] Catalogue all save/runtime components and reset boundaries.
- [ ] Audit title, no-file, save-slot, load, void, death, and savewarp flows.
- [ ] Audit SCLS and actor-driven transition consumers.
- [ ] Audit message-flow assets, generic node handlers, shared temporary progress
      bits, normal/abnormal cleanup, and item/event handoffs.
- [ ] Audit interaction/attention volumes, forced cutscene triggers, player-control
      gates, and temporal windows for representative NPC setups.
- [ ] Audit keyed door/gate actor families, their key/boss-key guards, consumption,
      persistent unlock writes, and alternate activation paths.
- [ ] Inventory static placement, persistent control, and transient instance state
      for representative actor families.
- [ ] Audit SavMem placements, guards, and all return/restart-place writers.
- [ ] Record known BiT, BiTE, Auru duplication, wrong-flags respawn, Fanadi lock,
      Text Displacement, and Ordon/twilight route evidence without prematurely
      encoding conclusions.
- [ ] Establish a glossary: build, runtime file, backing, slot, component, payload,
      binding, fact, technique, obstruction, obligation, refinement, route book,
      proof.

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

- [ ] Build the deterministic `orig/` extraction pipeline and cache/reuse its
      content-addressed derived fact packs without requiring original assets at
      planner runtime.
  - [x] Compile canonical world inventories into a strict exact-context payload
        and sealed manifest, and cache both in the immutable content store.
  - [ ] Add full input discovery/version verification and one-command extraction
        from a supplied `orig/` tree.
- [ ] Auto-detect and verify supported inputs; reject label/digest disagreement
      and represent unknown inputs as unsupported rather than guessing.
- [x] Import world-context stages, room/layer bindings, player spawns, static
      placements, raw SCLS records, and collision/SCLS activation joins.
  - [x] Own the compatible world/native observation input contracts inside the
        planner and remove all planner-to-Huntctl crate dependencies.
- [ ] Import actor-driven transitions and any remaining map/room metadata not
      represented by the current world inventories.
- [ ] Model ordinary item/NPC/event producers.
- [ ] Implement normal bank commit/load and binding changes.
  - [x] Execute typed serialize/restore/bind/rebind operations against independent
        owner stores atomically; concrete normal-boundary policies remain to be
        extracted and applied.
- [ ] Derive bound small-key counts and dungeon items from per-stage memory.
- [ ] Import hard door/actor guards and their state operations where decidable.
- [ ] Import message-flow graph nodes, temporary-bit reads/writes, branch
      predicates, normal cleanup, and item/event handoffs from the selected
      language resources where decidable.
- [ ] Import cutscene phase graphs, embedded scene changes, return/restart-place
      writers, actor/resource archive requests, load-failure/fallback branches,
      and ordered cleanup where decidable.
- [ ] Represent partial cutscene execution conservatively: preserve confirmed
      prefix writes, retain values whose writers are confirmed skipped, and mark
      effects beyond an unknown failure/interruption boundary unknown.
  - [x] Add cutscene scene-change/resource-load-failure transition kinds and a
        masked raw-knownness invalidation operation, so a modeled exceptional
        branch can retain confirmed bits while invalidating only unaudited ones.
  - [ ] Import concrete ordered phase branches and affected-bit masks from
        evidence instead of authoring guessed suffix effects.
- [ ] Produce semantic/raw build and language diffs, with explicit unknown or
      uncovered fields rather than assumed equivalence.
- [ ] Reconstruct live actor behavior from placement, layer, persisted state, and
      instance lifecycle.
- [ ] Implement save/load/title/runtime-file operations.
  - [x] Add transactional primitives for serialization, restoration, explicit
        component projection between runtime-file bindings, and location changes.
  - [ ] Model active-runtime lifecycle/backing attachment and concrete normal
        save/load/title sequences as evidenced transition programs.
- [ ] Implement writer/gate/reader evaluation and last-writer provenance.
  - [x] Evaluate scoped/evidenced writer activation separately from active and
        unknown blocking gates, resolve reader source values separately from
        friendly interpretations, and append the responsible transition to every
        component mutation.
  - [ ] Execute ordered writer/gate/read programs from imported mechanics and
        retain a queryable last-writer/gate event history across search states.
- [ ] Generate the upper-bound authorization graph.
  - [x] Add exact-context, evidence-aware tri-state predicate evaluation and
        per-transition upper-bound assessment; unknown raw bits, absent values,
        unsupported equivalence scopes, and disallowed evidence remain unknown.
  - [ ] Materialize and traverse the authorization graph from evaluated
        snapshots.
- [ ] Keep extracted destinations non-executable until their activation contracts
      are discharged.
  - [x] Classify hard-guard failure, unresolved requirements, and outstanding
        physical obligations separately; modeled feasibility cannot treat an
        outstanding obligation as executable.
- [ ] Mark candidates whose activation physics remain unknown.

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
- [ ] Encode the shared Auru recent-item store/writer/consumer mechanism separately
      from build-specific activation feasibility and external HD evidence.
- [ ] Add a hypothetical local-bank rebind refinement for testing.
- [ ] Add diagnostics for aliases that change under a binding.

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
- [ ] Expose upper-bound versus modeled-feasible graph diffs.

Deliverable: flag-permitted nonsense is visible but no longer reported as a
verified route.

### Phase 6 — Technique and refinement packs

- [x] Build validated pack contracts for techniques and obstruction resolvers.
- [x] Encode exact setup, component operations, discharged obligations, and cost.
- [x] Allow a pack to supply a witnessed microtrace with exact pre/post state and
      timing rather than a global named Boolean.
- [ ] Add built-in packs for ordinary movement and selected sequence breaks.
- [ ] Add route-local and ephemeral what-if overlays.
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

- [ ] Implement backward relevance expansion from goals.
- [ ] Combine it with forward stateful feasibility from the start.
  - [x] Implement bounded forward state search over exact snapshots, typed
        transition effects, action-local resolver/technique selections, and
        modeled versus upper-bound feasibility.
  - [ ] Add backward relevance pruning and the remaining route/path constraints
        before treating this as the production solver.
- [ ] Support OR producers, AND requirements, and ordered writer/gate/read setups.
- [ ] Implement state hashing, dominance, cycles, and continuation-safe merging.
  - [x] Add semantic search-state hashing that includes backing stores, bindings,
        gates, preservation, and pending cleanup while excluding snapshot labels
        and proof-only provenance; use it for cycle/duplicate suppression.
  - [ ] Add resource dominance and continuation-safe merge proofs.
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
- [x] Return reachable, unreachable-under-model, or unknown.
  - [x] Expose canonical fact/mechanics/execution-state artifacts through a
        standalone planner runtime boundary, isolated from huntctl's TAS CLI and
        workbench implementation.
- [ ] Report minimal missing obligations/assumptions where practical.
  - [x] Retain a deterministic closest blocker witness per transition with guard
        truth, active/unknown obstructions, selected setup, discharged and
        outstanding obligations, unknown requirements, and source-state digest.
  - [ ] Compute minimal failed-producer cuts across multiple upstream actions.
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
- [ ] Show component transformation and provenance histories.
- [ ] Show last-writer and gate history for latched values.
- [ ] Label all hypothetical and low-confidence dependencies.
- [ ] Generate concise collapsed summaries and fully expanded research views.

Deliverable: every route and failure is inspectable rather than magical.

### Phase 9 — Independent planner UX and authoring

- [ ] Define an independent versioned planner graph-projection schema and Rust
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
  - [ ] Add an HTTP/WebSocket adapter if the browser client requires one.
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
  - [ ] Add the browser interactions.
- [ ] Add inventory/flag/component state inspector with before/after diff.
  - [x] Add a planner-owned headless state-inspection projection for live and
        serialized stores, raw/structured payloads, bindings, provenance, and
        exact-context friendly/derived fact evaluations.
  - [ ] Add the visual inspector and before/after route-step diff interaction.
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

### Phase 11 — Vertical slices

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
- [ ] Show physical slots 1–3 separately.
- [ ] Model void/title-state handling and save projection to a chosen slot.
- [ ] Model BiTE as a selected component splice into an existing file.
- [ ] Allow an unsaved file-0 goal and hypothetical escape overlay.
- [ ] Explain exactly which components die when a file-0 lifetime ends.

#### 11C. EMS to Hyrule Castle/Ganon

- [ ] Produce upper-bound logic path.
- [ ] Introduce geometry/twilight/form/mount obstructions.
- [ ] Encode standard EMS setup.
- [ ] Encode Epona OOB non-twilight constraint.
- [ ] Encode rupee clip as a scoped replacement for the charge-attack approach.
- [ ] Show how route results refine as obstruction knowledge is enabled.

#### 11D. Local-bank rebind

- [ ] Snapshot a Forest Temple-bound payload.
- [ ] Add hypothetical preservation and Temple of Time rebind.
- [ ] Verify raw bytes remain identical while aliases change.
- [ ] Derive downstream effects only from the new interpretation.
- [ ] Display mixed provenance and hypothesis dependency.
- [ ] Remove overlay and verify base reachability returns unchanged.

#### 11E. Fanadi save-location lock

- [ ] Model SavMem writer, event/switch guards, and placements.
- [ ] Model `NO_TELOP` as a write gate with observed lifetime.
- [ ] Model Fanadi setter/clearer and Ooccoo/setup prerequisites.
- [ ] Retain the exact last successful `PlayerReturnPlace` write.
- [ ] Model savewarp as a reader of the held value.
- [ ] Search for setup orderings and explain intervening writes.
- [ ] Add a hypothetical Fanadi-access bypass and verify earlier return locations
      become usable without modifying the core lock mechanism.

#### 11F. Faron-twilight return research

- [ ] Define goals for Goats map, Ordon Village, outside Link's house, Link's
      house, and Ordon Spring while Faron remains in twilight.
- [ ] Enumerate SCLS, spawn, savewarp, void, death, title, cutscene, actor, and
      technique-provided incoming transitions to each target.
- [ ] Apply twilight, form, collision, and activation obstructions per approach.
- [ ] Include BiT/BiTE, held return place, wrong-state respawns, OOB, and proposed
      component-transfer hypotheses where scoped plausibly.
- [ ] Report reachable, blocked, and unknown candidates with exact missing
      obligations instead of flattening them into “no.”

#### 11G. Lanayru spirit and Vessel of Light

- [ ] Locate the spirit actor/event flow and every build-specific placement.
- [ ] Identify the raw event bits, temporary bits, room/layer, form, twilight,
      approach, and cutscene prerequisites for the spirit to appear.
- [ ] Separate appearance, interaction, cutscene start, Vessel grant, and post-grant
      state into distinct transitions rather than one milestone.
- [ ] Identify all writers and consumers of the Vessel and tear-count state.
- [ ] Test whether alternate entrances, wrong layers, wrong-state respawns, or
      component transfers can satisfy or bypass individual prerequisites.
- [ ] Produce both a friendly explanation and the exact raw predicate for each
      supported build, with unknown conditions called out explicitly.

#### 11H. Keyed dungeon transition

- [ ] Extract one representative small-key door's encoded destination without
      making it immediately executable.
- [ ] Derive the bound dungeon key count from the live per-stage backing store.
- [ ] Model key pickup provenance independently from the fungible count.
- [ ] Audit and model the door actor's guard, key consumption, persistent unlock
      write, live animation/collision, and reload reconstruction.
- [ ] Verify any key from the same bound dungeon store can satisfy the door.
- [ ] Add a hypothetical key-store preserve/rebind overlay and verify it opens
      routes only through backing-store semantics, with hypothesis provenance.
- [ ] Verify an OOB route that avoids the door does not falsely mark the door
      unlocked or consume a key.

#### 11I. Auru recent-item grant

- [ ] Model `mGtItm` as a session/process storage site separate from save-file
      inventory and `mPreItemNo`.
- [ ] Enumerate presentation/chest/show-item writers and prove which boundaries
      preserve or reset the value.
- [ ] Model file A writing an item ID, file load preserving session state, and file
      B consuming it through generic get-item semantics.
- [ ] Decompose Auru's normal memo path, pending item actor, `DEFAULT_GETITEM`
      handoff, and broken path that avoids the memo overwrite.
- [ ] Author the talk-volume/outside-trigger/player-control obligation.
- [ ] Mark the known HD targeting resolver as external build evidence; keep the SD
      candidate surfaced as obstructed or unknown rather than absent.
- [ ] Add a hypothetical SD geometry/interaction resolver and verify arbitrary
      recent-item producers become usable without editing Auru's grant rule.
- [ ] Model the optional memo-preservation sidehop/backflip interruption as a
      separate frame-exact microtransition.

#### 11J. Text Displacement to Goron Mines

- [ ] Extract raw shared message-progress bits and their generic flow-node writers,
      readers, and cleanup paths.
- [ ] Model at least Coro, Auru, Yeta, and Ooccoo producer routes as distinct
      interruption/advancement proofs where evidence exists.
- [ ] Identify Gor Coron's exact displaced-branch predicate and downstream
      persistent event/switch writes.
- [ ] Model invisible wall, elevator authorization, live NPC blockers, and room
      reload reconstruction independently.
- [ ] Start with the Goron Mines encoded transition visible but non-executable;
      discharge each authorization and physical obligation causally.
- [ ] Verify the solver can work backward from the entrance to all enabled
      producers of the required text-bit pattern.
- [ ] Verify removing one producer or adding a hypothetical new interrupt changes
      reachability without changing the Goron consumer or entrance rules.

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
- [ ] Capture normal completion and actor-corruption/archive-load-failure paths;
      identify the last confirmed operation and every flag or writer that becomes
      skipped versus unknown.
- [ ] Model actor corruption as the producer of the failed-load/exceptional-flow
      predicate, not as a direct Castle Town warp.
- [ ] Verify a preexisting Castle Town return place survives only when its
      overwriting writer is skipped, and that ordinary savewarp subsequently
      reads it from Zelda's tower.
- [ ] Vary the incoming return place and prove the cutscene failure preserves
      that value generically rather than hard-coding Castle Town.
- [ ] Keep the route unknown in established mode until the relevant partial
      effects and scene-change branch have source or trace evidence.

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
| EMS upper bound | Logical authorization appears before geometry is supplied. |
| EMS obstruction | Physical blocker removes route only from feasible projection. |
| Epona OOB | Route is rejected while twilight/mount predicate fails. |
| Local bank normal flow | Stored stage entries load/commit to the proper binding. |
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
- Complete small-key consumption and persistent-unlock behavior across all
  door/gate actor families and exceptional stages.
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

The first serious release is complete when:

- Reachability is derived from transitions and state, never authored loss lists.
- Slotless file 0, its persistent-domain contents, physical slots, save
  projection/reattachment, and cross-file provenance are represented accurately.
- Raw storage, bindings, friendly flag interpretations, and component lifetimes
  are independently inspectable.
- Wrong-state respawns and hypothetical local-flag transfers use generic typed
  component operations.
- Fanadi save locking emerges from writers, a `NO_TELOP` gate, the held value, and
  a savewarp reader.
- Authorization, physical obstruction, known bypass, and witnessed execution are
  distinct layers.
- Encoded destinations remain inert until hard guards and physical activation
  obligations are satisfied; dungeon keys derive from their bound backing store.
- Static placements, persisted actor-control state, and transient actor instances
  are modeled separately across load/unload boundaries.
- Session recent-item state, message-flow nodes, shared text-progress bits,
  scheduled cleanup, and frame-exact interruption edges are independently modeled.
- Auru's causal grant mechanism is separated from HD/SD spatial feasibility, and
  hypothetical SD resolvers compose without changing the grant rule.
- Goron Mines Text Displacement derives from raw bit producers and the actual NPC
  flow consumer before resolving the remaining world obstructions.
- Users can author versioned refinement packs and ephemeral what-if scenarios
  without mutating base data.
- Exact content is digest-verified; language/configuration is an independent
  runtime dimension; deterministic derived fact packs support normal use without
  requiring `orig/` and never silently substitute an unverified build.
- Shared semantic names retain exact per-context raw bindings, and portable routes
  contain only facts proven equivalent across their selected contexts.
- The solver supports alternative producers, ordered setups, exact
  content/language contexts, path constraints, costs, and honest unknowns.
- The UI scales from a collapsed milestone route to raw flags, component history,
  and full proof graphs.
- The visual editor is recognizably a sibling of the TAS Route Workbench, while
  its independent Rust schemas and graph projection preserve planner-specific
  causal semantics.
- Fishing Rod, BiT/file 0, EMS/obstructions, local-bank rebind, Fanadi locking, and
  Faron-twilight return research all pass their vertical-slice fixtures.
- Lanayru spirit appearance, interaction, and Vessel grant are modeled as
  separately testable transitions with raw and friendly requirements.
- A complete glitchless story route and versioned glitchless 100% contract replay
  coherently through the fact system, including Forest Temple monkeys and balanced
  dungeon-key/door state.
- A standard Any% graph composes known sequence breaks without duplicating
  mechanics, and suspiciously early states produce auditable proofs for bug versus
  discovery triage.
- Every displayed route can explain why it works, every rejected route can explain
  what blocks it, and every hypothetical route names its assumptions.
