# Route timelines, variants, and Git

Git is the route database. Checked-in `.timeline` files, segment programs, and
proof metadata are authoritative; normal Git commits provide history, copy,
move, deletion, review, and recovery. The route layer adds game-specific graph
semantics, not a second version-control system.

`routes/intro.timeline` describes milestone nodes, segment edges, competing
variants, and pinned lineages. A lineage contains references to segment
variants. It does not copy their input, so even a large route tree remains a
small collection of independent segment payloads and tiny manifests.

## Curated variants versus mining output

Search populations, failed attempts, traces, and transient champions remain in
the ignored `build/` tree. A useful result is promoted by adding one immutable
segment artifact under `routes/<route>/variants/<segment>/`, adding its proof
metadata, and referencing it from the timeline. Everything else can be thrown
away.

This makes pruning ordinary branch hygiene:

- delete variants or lineages that led nowhere;
- keep unusual RNG frontiers even when they are locally slower;
- commit only results worth sharing or preserving; and
- use Git history to recover a discarded experiment when needed.

Promotion is not “the fastest score wins.” Tick count is one property. Boundary
fingerprints, RNG state, stability, and downstream usefulness determine whether
two variants are substitutes or separate frontier points.

## Immutable segment model

A variant is one attempt at one segment. It declares:

- a candidate, TAS source, or compact tape artifact;
- its exact starting boundary fingerprint;
- its produced boundary fingerprint; and
- optional score information such as the first-hit simulation tick.

The input artifact contains only that segment. Stage-launch setup, search
harness frames, and other evaluation scaffolding are not valid continuation
payloads. The workbench permits standalone preview of those artifacts but
refuses to concatenate them until a canonical payload window and exact boundary
proof exist.

A continuation pins every segment variant to the exact preceding variant and
checkpoint fingerprint. A branch inherits a named prefix and supplies a
different tail. Adding a sibling variant never rewrites an existing lineage.

## Route Workbench

In VS Code, choose the single **Glitch Hunt: Route Workbench** entry under
**Run and Debug**. The pre-launch task builds Dusklight and the Rust workbench,
then opens a local browser view of the checked-in route graph.

On Windows, the launcher prefers Brave when it is installed and otherwise uses
the system default browser.

The workbench renders milestones as a vertical tree and each checked-in segment
variant as a card. **Play** runs the selected chain and releases controller
ownership when its tape ends. **Record** does the same deterministic replay,
then records live port-0 input beginning with the first PAD read after handoff.
Each launch gets a fresh isolated writable state directory.

Record actions belong to concrete lineage occurrences, not just milestone or
variant names. This matters when the same variant is reachable through multiple
RNG/state prefixes. A checked-in endpoint is recordable only when its complete
lineage is canonical and the native milestone fingerprint can be verified at
the exact handoff frame.

Closing Dusklight normally finalizes the recording. The workbench adds it as an
ignored draft child under
`build/automation-state/route-workbench/drafts/` and polls until its status is
known. Restarting the workbench scans the same directory, so ready drafts remain
visible. A draft stores only its continuation tape plus small parent, launch,
and result manifests; playback reconstructs and verifies the selected chain.

Draft endpoints begin as `manual_stop` / `unverified`. An optional human label
describes intent but is not proof. Ready unverified drafts may be replayed and
extended. Future promotion into the checked-in timeline must attach a native
boundary predicate/fingerprint. Zero-frame, capacity-exhausted, corrupt,
detached, or failed recordings remain visible for diagnosis but cannot become
parents.

The native result binds the launch with a random session token and authenticates
the continuation by frame count, encoded length, and SHA-256. Parent-chain
digests, exact lineage pins, path containment, and cycle checks prevent a draft
from silently moving to another route state. Mouse, gyro, and Dusklight-specific
action bindings are suppressed during recording because the current DUSKTAPE
schema cannot replay those side channels.

The same workbench is available directly:

```powershell
cargo run --manifest-path tools/huntctl/Cargo.toml -- timeline workbench `
  --timeline routes/intro.timeline `
  --game build/windows-clang-debug/dusklight.exe `
  --dvd orig/GZ2E01/GZ2E01.iso
```

`--dvd` may be omitted to use the image last selected in Dusklight's normal
configuration. The VS Code launch uses this behavior, so it does not encode a
machine-specific image path.

The server binds only to loopback. It rereads the timeline on every request, so
working-tree edits appear after refreshing the graph. Game, disc, and state
paths are server-owned and cannot be supplied by the browser.

Random-access playback can be added after a restorable checkpoint format exists.
The initial UI deliberately exposes only complete-segment playback.

## DSL

The line-oriented format uses declarations such as:

```text
timeline intro
milestone process_boot
milestone link_control
segment boot_to_link from process_boot to link_control profile boot_to_fsp103
variant boot_to_link.golf439 incumbent uses tas intro/variants/boot_to_link/golf-439.tas starts process-clean-v1 produces 5f3f489f2cf561844564368fbc427d85 ticks 439
continuation main starts root@process-clean-v1
continue main with boot_to_link.golf439 after root@process-clean-v1
branch experiment from main at link_control
```

Artifact forms are `uses candidate`, `uses tas`, and `uses tape`. Baselines are
generated profile seeds intended for evaluation and standalone preview. Paths
are relative to the directory containing the timeline. Comments start with a
hash. Validation rejects duplicate names, missing references, boundary
mismatches, discontinuous continuations, and cycles.

Preview an upstream substitution without changing files:

```powershell
cargo run --manifest-path tools/huntctl/Cargo.toml -- timeline status `
  --timeline routes/intro.timeline `
  --continuation main `
  --select boot_to_link.golf439
```

`timeline rebase-compatible` can emit the text for a boundary-compatible
lineage variant. It never mutates or prunes the original.

## Legacy route store

The `timeline store` commands predate the Git-owned model. They remain readable
for existing experiments, but their object refs, promotion history, and garbage
collection are not route authority and should not be used for new work. Useful
validation and content-hash ideas may later become an ignored generated index
over the checked-in route tree.
