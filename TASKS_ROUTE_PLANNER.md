# Twilight Princess Route Planner — Remaining Product Work

This document is the active product plan. It is not an implementation diary,
schema catalogue, changelog, or archive of completed work.

Rules for maintaining it:

- Keep only unfinished work and current release gates.
- Delete completed tasks instead of accumulating checked boxes and victory text.
- Do not call a schema, service command, unit test, or demo fixture a finished
  user workflow.
- Judge progress from a new user's ability to author, understand, save, compose,
  and solve a real route without editing serialized data.
- Treat usability failures as product failures, not deferred visual polish.
- Preserve detailed research results in source, tests, focused documentation,
  and Git history rather than copying them into this plan.

## 1. Current product truth

The planner engine contains useful typed state, transition, route-book,
validation, and solving machinery. That machinery is not yet a usable route
planner.

The current browser is a transition-composer prototype over preassembled demo
documents:

- New Project starts with an empty mechanics catalogue, no start state, no goal,
  and therefore no useful node palette.
- A user cannot select a game context, mount a mechanics library, choose an
  initial state, or author a meaningful route from a blank project.
- The browser can append and rearrange transitions that were already embedded in
  a demo, but it cannot create the assets needed to express a route.
- The current project document embeds a composed catalogue, start state, route
  book, overlays, equivalence sets, and presentation data in one JSON document.
- Built-in demonstrations are constructed in Rust rather than loaded as ordinary
  independently serialized assets.
- Presentation regions are not reusable semantic subgraphs.
- The default screen prioritizes raw model payloads and controls over the route
  authoring task.
- The browser acceptance test exercises one preassembled keyed-door demo and can
  skip without running on unsupported hosts. It does not prove that a blank user
  project can express a route.
- No fresh-file glitchless route, versioned 100% route, or standard Any% route
  currently replays end to end through the planner.

Until the release gates below pass, describe the application as a planner engine
and authoring prototype, not as a completed blueprint-style route planner.

## 2. Product contract

The product must let a route author:

1. Create or open a workspace.
2. Select an exact game build and runtime configuration.
3. Create a scenario anchored to a concrete or explicitly contingent entry
   state.
4. Discover mechanics, state predicates, goals, and reusable graphs through a
   searchable node catalogue.
5. Build a route by placing and connecting typed nodes on a canvas.
6. See whether every connection is executable, blocked, unknown, or outside the
   selected context.
7. Inspect only the relevant before-state, delta, after-state, obligations, and
   evidence for the selected node or connection.
8. Extract repeated route sections into reusable subgraphs with typed entry and
   outcome contracts.
9. Create explicitly hypothetical custom nodes without allowing them to
   masquerade as established mechanics.
10. Save, rename, move, duplicate, reference, fork, export, import, and delete
    user-authored assets as ordinary files.
11. Ask the solver for a route or missing producer and apply a proposed result
    through the same graph-authoring surface.
12. Close and reopen the workspace with identical semantic identities,
    references, layout, and solver results.

Raw JSON remains an interchange and debugging surface. It is never the primary
authoring workflow.

### Delivery order

Work proceeds in this order:

1. File-backed workspace and asset boundaries
2. Workspace/Library content browser and CRUD
3. Graph-first application shell and direct node authoring
4. Scenario grounding, semantic subgraphs, and custom nodes
5. The first meaningful fresh-boot route vertical slice
6. Broader route and build coverage

Do not skip a product gate because a lower-level API is easier to implement.
Until the usable-authoring alpha passes, new model/schema work is in scope only
when it directly unblocks the selected vertical-slice route or fixes a
correctness defect in existing authority. A new report, serializer, extractor,
or test fixture does not count as progress by itself.

## 3. Workspace and asset architecture

### 3.1 Replace the monolithic project document

- [x] Introduce a workspace manifest that contains identity, version, mounted
      libraries, exact-context defaults, and asset roots without embedding
      catalogues, route graphs, snapshots, or layouts.
- [x] Store each mutable asset in its own canonical JSON file using the existing
      typed serialization and validation infrastructure. Do not introduce YAML.
- [x] Separate semantic graph data from presentation layout so moving a node
      cannot change a route or invalidate semantic identity.
- [x] Give every asset a stable identity independent of its file path. Rename and
      move operations must preserve references.
- [x] Seal library dependencies by exact identity and digest. Opening a workspace
      with missing or changed dependencies must produce an actionable dependency
      error rather than silently rebinding.
- [x] Support schema migration explicitly. Never make users repair serialized
      files by hand after an application update.
- [x] Define crash-safe transactions for multi-file operations such as moving a
      graph with dependent layouts or deleting an asset with references.

The initial mutable asset types are:

- Scenario
- Route graph
- Reusable subgraph
- Custom node definition
- State seed
- Query/goal
- Route book
- Layout

### 3.2 Separate Workspace and Library content

- [x] Add a **Workspace** browser tab containing only mutable user assets.
- [ ] Add a **Library** browser tab containing immutable mechanics, exact
      contexts, verified fixtures, templates, and source-backed examples.
- [x] Keep read-only and writable items out of the same undifferentiated tree.
      Cross-source search results must carry an unmistakable source badge.
- [ ] Make library operations contextual: Open, Inspect, Add Reference, Create
      Scenario From Template, and Fork to Workspace. Do not show disabled
      Rename/Delete commands.
- [ ] Make dragging a Library asset create a reference. Copying or forking must
      remain an explicit separate action with recorded provenance.
- [ ] Surface code-authored node kinds through the node catalogue, not as fake
      content files.

### 3.3 Implement real CRUD

- [ ] Add create, rename, move, duplicate, delete-to-trash, restore, and permanent
      delete operations for Workspace assets and folders.
- [ ] Use fixed virtual roots for typed assets while allowing user folders below
      those roots.
- [ ] Validate names, collisions, references, and permissions before mutation.
- [x] Show inbound references before delete and preserve resolvable broken-link
      records when deletion is confirmed.
- [x] Add revision-checked save and conflict resolution for independently open
      editor tabs.
- [ ] Add import/export for individual assets and complete workspaces.
- [x] Add filesystem change detection so external edits or Git operations do not
      leave stale in-memory documents.
- [ ] Move hard-coded Rust demonstrations into ordinary read-only serialized
      Library assets loaded through the same validation path as user content.

## 4. Application shell and information hierarchy

- [x] Replace the current wall-of-panels layout with a graph-first workspace:
      Content Browser, central canvas, selection-driven Details panel, and a
      collapsed diagnostics drawer.
- [x] Keep the canvas as the dominant surface at ordinary desktop sizes.
- [x] Open diagnostics automatically only for errors, explicit trace requests, or
      completed solve results.
- [x] Show no raw payload or full state dump on initial load.
- [x] Put rare commands in contextual menus instead of a permanent toolbar of
      unrelated buttons.
- [x] Limit the persistent toolbar to workspace navigation, Save, Undo/Redo,
      Validate, Solve/Play, and view controls.
- [ ] Support multiple asset editor tabs with breadcrumbs and unsaved-state
      indicators.
- [ ] Add command-palette access and consistent keyboard shortcuts for every
      primary operation.
- [ ] Preserve selection and viewport when switching between a graph and its
      referenced subgraph.
- [ ] Provide empty states that lead directly to meaningful actions: choose a
      context, choose an anchor, add a node, or open a template.
- [ ] Remove internal schema names, digests, enum spellings, and serialized field
      names from default labels. Keep them available in an Advanced inspector.

## 5. Blueprint-style graph authoring

### 5.1 Node catalogue and placement

- [ ] Add a canvas context menu with searchable, categorized **Add Node**
      results.
- [ ] Let dragging from a pin open the same catalogue filtered to compatible
      nodes.
- [ ] Merge three sources into the catalogue without conflating them:
      code-authored node kinds, immutable mechanics/library references, and
      Workspace subgraphs/custom nodes.
- [ ] Rank results by context compatibility, current state, category, recent use,
      and text relevance.
- [ ] Support keyboard placement, copy/paste, duplicate, multi-select, delete,
      alignment, comments, and reroute nodes.
- [ ] Implement undo/redo as semantic commands, including asset creation and
      graph rewiring.

### 5.2 Connections and pins

- [ ] Define typed execution, state-contract, predicate, and data pins. Avoid a
      pin for every field in the complete game state.
- [ ] Make invalid connections impossible to commit and explain why the proposed
      connection is incompatible.
- [ ] Render executable, blocked, unknown, contingent, and context-incompatible
      connections with distinct accessible treatments.
- [ ] Recompute downstream state after every semantic edit and retain the closest
      valid state when a connection fails.
- [ ] Support insertion between existing nodes, branch creation, alternative
      methods, and reconnection without requiring ordered-list button workflows.
- [ ] Keep visual connections non-authoritative until the Rust service validates
      and commits their semantic edit.

### 5.3 Compact node presentation

- [ ] Default each node to a compact title, category icon, relevant pins, and
      small evidence/context status badges.
- [ ] Put guards, effects, state deltas, evidence, and diagnostics in the Details
      panel rather than expanding prose on the canvas.
- [ ] Let users pin selected state fields or predicates as small watches without
      exposing the entire execution state.
- [ ] Show one-line summaries for before-state, effect, and outcome on hover or
      selection.
- [ ] Provide graph-level filters for evidence status, context, route membership,
      obstruction state, and unknown coverage.

## 6. Scenario roots and state grounding

A reusable graph is not inherently rooted at a blank memory card. A concrete
scenario is.

- [ ] Add a Scenario Root node with exactly one explicit anchor:
  - Fresh boot plus memory-card configuration
  - Exact card fixture and selected slot
  - Exact recorded snapshot
  - Output contract of an upstream graph
  - Explicit contingent/hypothetical entry contract
- [ ] Require exact content identity and runtime configuration at every concrete
      root.
- [ ] Represent a reusable graph's required state as a typed Entry Contract, not
      as an embedded full save or a fabricated default snapshot.
- [ ] Preserve unknown fields as unknown. An omitted field must never silently
      become false, zero, empty, or absent.
- [ ] Classify graph grounding visibly:
  - **Grounded** — backed by an exact boot, card, or snapshot artifact
  - **Bound** — supplied by a validated upstream graph
  - **Contingent** — depends on explicit assumptions
  - **Incomplete** — required entry values remain unresolved
- [ ] Allow local validation of a contingent graph while preventing it from
      claiming boot-to-goal reachability until all entry contracts are grounded.
- [ ] Show Before → Delta → After only for the selected node or edge.
- [ ] Provide state comparison and watches for inventory, location, runtime/file
      identity, flags, keys, actor state, and user-selected component fields.
- [ ] Make scenario anchors and state seeds first-class files that can be
      referenced by multiple scenarios without copying their complete payload.

## 7. Reusable subgraphs and custom nodes

### 7.1 Semantic subgraphs

- [ ] Make a subgraph an independently serialized asset with typed entry
      predicates, outcome predicates, parameters, state projections, costs, and
      unresolved obligations.
- [ ] Add **Extract Selection to Subgraph** and **Collapse to Subgraph Call**
      operations.
- [ ] Validate every subgraph call against the caller's current state and exact
      context.
- [ ] Reevaluate subgraph internals after every distinct caller state; never cache
      one successful execution as universally valid.
- [ ] Support multiple implementations of the same outcome as explicit methods
      sharing one contract.
- [ ] Allow navigation into a subgraph and back through editor breadcrumbs.
- [ ] Show the residual state differences between alternate methods rather than
      collapsing them merely because they reach the same headline goal.

### 7.2 Blueprint-like custom nodes

- [x] Let users define a custom transition node with typed inputs, guards,
      effects, costs, obligations, outputs, scope, and evidence status.
- [ ] Compile custom transition nodes through the existing refinement/validation
      machinery instead of interpreting browser-owned behavior.
- [x] Default new custom mechanics to hypothetical/research status.
- [ ] Require explicit evidence and review before a custom node can become an
      established Library mechanic.
- [ ] Support custom macro nodes backed by subgraphs without duplicating their
      internal transitions.
- [ ] Reject arbitrary untyped force-state or force-connect nodes.
- [ ] Provide templates for common custom-node shapes without making users author
      serialized contracts directly.

## 8. Actual route-planning workflow

- [ ] Add a new-workspace flow that selects a writable folder and mounts one or
      more exact immutable libraries.
- [ ] Add a new-scenario flow that selects build, runtime configuration, anchor,
      and goal before opening the graph.
- [ ] Populate the node catalogue from mounted mechanics and current context,
      rather than embedding a private catalogue in every scenario.
- [ ] Let an author build, insert, remove, replace, branch, and reconnect route
      steps directly on the canvas.
- [ ] Keep the route book as an independently serialized semantic asset referenced
      by scenarios and graphs.
- [ ] Let the solver propose complete or partial producer chains as a preview
      graph that the user can accept, edit, or reject.
- [ ] Explain solver failure in route-author language: missing producer, blocked
      transition, unresolved obligation, unsupported context, or incomplete
      model.
- [ ] Allow pin, ban, prefer, cost, and method choices from the graph and Details
      surfaces without exposing raw route-book edits.
- [ ] Support multiple goals and milestone queries within one scenario.
- [ ] Export a human-readable route summary and a machine-verifiable route proof
      from the same saved graph.

## 9. First meaningful vertical slice

The first usable release is not satisfied by a blank canvas or a single
preassembled door demo.

- [ ] Ship an exact GZ2E01 Library sufficient to author a route from fresh boot
      through character creation, Ordon progression, early-world traversal, and
      a meaningful Forest Temple milestone.
- [ ] Require the user to create that scenario from a blank workspace through the
      ordinary UI, without importing a preassembled project document.
- [ ] Include ordinary travel, loading zones, NPC interaction, item acquisition,
      event/cutscene progress, dungeon keys, a locked door, and at least one
      obstruction or alternate method.
- [ ] Extract a repeated or logically bounded portion into a reusable subgraph,
      invoke it from a second compatible state, and show any residual-state
      difference.
- [ ] Create one hypothetical custom node, demonstrate its contingent result in
      research mode, then remove it and recover the established result.
- [ ] Save, close, reopen, rename, move, duplicate, fork, and delete/restore the
      relevant assets without changing semantic identities or losing references.
- [ ] Produce a deterministic proof and route summary whose identities match the
      headless solver.

## 10. Model and evidence work required by real routes

Model work is prioritized by routes the product is trying to author. Isolated
schema expansion does not count as product progress until it unblocks a route or
closes a documented unknown.

### 10.1 Context and resource coverage

- [ ] Finish exact language/configuration selection, persistence, switching, and
      resource-resolution behavior for supported builds.
- [ ] Reproduce and bind the exact PAL, Wii, and HD executable/resource identities
      required by supported route scenarios.
- [ ] Add the exact affected Wii PAL French build and verify cannon-payment
      behavior without leaking it into unaffected contexts.
- [ ] Keep portable queries contingent on a valid witness for every selected
      context.

### 10.2 World, actor, and interaction coverage

- [ ] Import actor-driven transitions and remaining selected map/room metadata
      needed by the vertical-slice and glitchless routes.
- [ ] Model interaction eligibility, actor lifecycle, control ownership,
      collision/geometry, resource loads, RNG/timers, and relevant failure paths
      only where required by selected route actions.
- [ ] Make unaudited physical feasibility an explicit obligation rather than an
      executable edge.
- [ ] Add coverage reports that name the exact route node blocked by each missing
      actor, writer, guard, or interaction rule.

### 10.3 Message and cutscene coverage

- [ ] Finish message-flow branches, temporary/persistent effects, item/resource
      effects, event handoffs, speaker selection, and cleanup needed by selected
      routes.
- [ ] Import cutscene phases, embedded scene changes, return/restart-place
      writers, actor/archive requests, load-failure branches, and ordered cleanup
      required by selected routes.
- [ ] Represent partial cutscene execution as confirmed prefix effects plus
      explicit unknown suffixes.
- [ ] Complete the witnessed Zelda-tower actor-corruption trace, including the
      Savmem actor allocation/create/execute result, relevant flags, incoming
      return place, and later savewarp read.

### 10.4 Route catalogue milestones

- [ ] Author and replay one reasonable glitchless route from fresh file through
      final-boss completion.
- [ ] Require every selected room transition, interaction, cutscene, acquisition,
      key expenditure, boss-key door, dungeon exit, and completion flag to pass
      through the causal model.
- [ ] Model Forest Temple monkey rescues and every selected dungeon key/door
      sequence individually before exposing collapsible route subgraphs.
- [ ] Compare propagated state against expected location, inventory, flags,
      dungeon resources, and relevant actor state throughout the route.
- [ ] Define a versioned 100% completion contract and author a verified route
      without aggregate goals hiding missing or double-counted collectibles.
- [ ] Author a versioned standard Any% route only after the glitchless route
      replays coherently.
- [ ] Represent known sequence breaks as alternate methods over shared mechanics,
      not bespoke route-only booleans.
- [ ] Compare solver results with known routes and classify every divergence as a
      real alternative, evidence gap, model gap, or bug.

## 11. Usability, accessibility, and performance gates

- [ ] Run task-based usability sessions with people who did not implement the
      planner. Observe them creating a workspace, grounding a scenario, adding
      nodes, fixing a broken join, extracting a subgraph, and saving/reopening.
- [ ] Require a first-time user to create and validate a short route without
      reading source code, opening raw JSON, or receiving developer coaching.
- [ ] Record failure points and change the product; do not explain around a bad
      interaction in documentation.
- [ ] Ensure every primary action is available by keyboard and every semantic
      status has a non-color cue.
- [ ] Add screen-reader labels, focus order, scalable text, reduced motion, and
      high-contrast validation.
- [ ] Keep interaction responsive on the largest supported route graph. Define and
      enforce budgets for initial projection, search, pan/zoom, semantic edit,
      downstream replay, and solve preview.
- [ ] Virtualize large content lists and graph surfaces instead of flooding the
      DOM.
- [ ] Preserve selected node, viewport, open tabs, and pending diagnostics across
      non-semantic refreshes.

## 12. Acceptance and regression strategy

- [ ] Replace optional, silently skipped browser coverage with explicit platform
      jobs for Windows and macOS. A missing configured browser must fail the
      browser job, not count as a pass.
- [ ] Add an end-to-end test that starts from New Workspace and authors the first
      meaningful vertical-slice route through visible UI operations.
- [ ] Add CRUD tests for every mutable asset type, including rename/move reference
      preservation, trash/restore, collision rejection, conflict handling, and
      crash recovery.
- [ ] Add tests proving Library assets are immutable and Fork to Workspace
      preserves provenance.
- [ ] Add graph tests for right-click creation, compatible-pin filtering,
      insertion, rewiring, branches, subgraph extraction, and undo/redo.
- [ ] Add scenario tests for every anchor kind and every grounding status.
- [ ] Add tests proving contingent graphs cannot claim rooted reachability.
- [ ] Add tests proving layout changes never alter semantic digests or solver
      output.
- [ ] Add round-trip tests across workspace files rather than only one embedded
      project document.
- [ ] Add golden screenshots only for stable visual hierarchy and accessibility;
      never use them as substitutes for semantic assertions.
- [ ] Run the same saved route through the browser, service API, CLI, and engine
      and require identical graph, state, proof, and rejection identities.

## 13. Migration and deletion

- [ ] Define a one-time importer from
      `dusklight.route-planner.web-project/v3` into workspace assets.
- [ ] Extract embedded catalogues, start states, route books, and layouts into
      separate files while preserving exact semantic identities where possible.
- [ ] Convert built-in Rust demos into serialized Library fixtures and delete
      their hand-assembled project constructors.
- [ ] Delete the legacy New Project flow that creates an empty, unusable
      catalogue.
- [ ] Delete or redesign toolbar and panel controls that expose implementation
      operations instead of user tasks.
- [ ] Remove claims that the first usable release is complete until all release
      gates in this document pass.
- [ ] Archive obsolete architecture prose in Git history rather than retaining it
      inside the active task plan.

## 14. Release gates

### 14.1 Usable authoring alpha

All of the following must be true:

- [ ] A new user can create a workspace and a grounded scenario without editing
      serialized data.
- [ ] Workspace and Library content are visibly separate.
- [ ] Workspace assets support complete CRUD with stable references.
- [ ] Right-click Add Node and pin-drag placement work from a searchable typed
      catalogue.
- [ ] The user can author and validate a meaningful multi-system route from a
      blank workspace.
- [ ] The user can extract, reuse, and inspect a semantic subgraph.
- [ ] The user can create and remove a clearly hypothetical custom node.
- [ ] State and diagnostics are available on demand without dominating the
      default screen.
- [ ] Save/reopen and headless replay preserve exact semantics.
- [ ] Windows and macOS end-to-end browser jobs execute rather than skip.

### 14.2 Route-planning beta

- [ ] The fresh-boot-to-Forest-Temple vertical slice passes through ordinary
      authoring, solver, persistence, and proof workflows.
- [ ] At least one obstruction/resolver route and one leave-and-return route are
      authored from Library mechanics rather than preassembled project documents.
- [ ] Alternate methods and residual state differences are inspectable and
      reusable.
- [ ] Model coverage reports are route-oriented and lead directly to the blocked
      node and missing evidence.
- [ ] Usability sessions no longer reveal workflow-blocking failures in project
      creation, node placement, connection, state grounding, or persistence.

### 14.3 Complete planner milestone

- [ ] A reasonable glitchless full-game route replays from fresh boot to final
      completion.
- [ ] The versioned 100% contract and route replay without hidden aggregate
      shortcuts.
- [ ] A standard Any% route and its major alternate methods are expressible over
      shared mechanics.
- [ ] Unsupported builds, unknown feasibility, and hypothetical mechanics remain
      explicit and cannot leak into established route proofs.

Nothing before these gates should be summarized as “the route planner is done.”
