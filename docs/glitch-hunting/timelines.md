# Route segment trees and Git

Git is the route database. Checked-in `.timeline` files, segment artifacts, and
proof metadata are authoritative; commits provide history, copy, move,
deletion, review, and recovery. The route layer adds game-specific tree and
playback semantics, not another version-control system.

## One structural object: the segment

Every playable input artifact is a segment. A segment owns:

- one candidate, TAS source, or compact tape;
- either the Boot origin or exactly one parent segment;
- its exact input and output boundary fingerprints; and
- the execution profile needed to interpret the artifact.

Alternative attempts are ordinary sibling segments with the same parent:

```text
Boot
└─ golf439
   ├─ human420
   └─ human_alt420
```

This makes ancestry structural and mechanically enforceable. Adding a faster,
slower, or RNG-different attempt means adding another sibling. It does not
rewrite the existing path or imply that local speed determines which branch is
globally useful.

Search populations, failed attempts, traces, and transient champions remain in
the ignored `build/` tree. Promotion copies only a useful immutable artifact to
`routes/<route>/segments/`, declares its segment in the timeline, and adds any
evidence worth retaining. Pruning is ordinary Git branch hygiene.

## Goals and proofs are metadata

A goal is declared *on* an existing segment after topology exists. It names a
read-only predicate that defines one parity axis. It never owns segments or
creates an edge.

A proof says that a segment satisfied a goal. The proving segment may be the
goal's reference segment or one of its siblings. This supports questions such
as “does this alternative reach the same crawl state?” without inventing a
second structural type. Missing or stale proof blocks only the parity claim,
score, or predicate-backed handoff; it does not make an exact segment chain
unplayable.

Milestone language 1.4 provides named value-parity projections for exact RNG,
actor-population, and flag-subset comparisons. Equal, different, and
incomparable are decided from the authenticated projection identity,
availability, and value fingerprint; topology and hard-coded folklore are not
parity evidence.

## Pinned paths

A continuation pins a root-to-leaf segment path and the fingerprint at every
edge. A branch inherits a named prefix and adds a different child path. These
pins make replay composition explicit and prevent a segment from silently
moving beneath a different RNG or loader state.

Because every segment has one structural parent, the tree itself remains easy
to inspect. Pinned paths answer a separate question: which sibling was selected
for a named route such as `main`?

## Route Workbench

Choose **Glitch Hunt: Route Workbench** under VS Code's Run and Debug panel.
The pre-launch task builds Dusklight and the Rust server, then opens the local
view in Brave when available.

The left sidebar is a compact Projects tree for standalone boot-rooted QA,
canary, glitch, and sample tapes. The large canvas remains the structural route
graph: Boot followed by nested segments, with alternatives as siblings. Goals,
proofs, fingerprints, scores, and artifact information appear only in the
selected entry's details.

**Playback** follows the selected segment's unique parent chain to Boot,
composes those exact artifacts, launches a fresh isolated process at the global
Playback speed, and releases controller ownership when the selected tape ends.
**Resume (accelerated)** replays that same full tape windowless and uncapped,
then hands over at the endpoint. Named continuations are bookmarks and preferred
paths, not playback authorization; an unpinned sibling remains playable when
its structural chain and fingerprints are valid.

Segment **Rename** writes a human-facing `label` beside the segment declaration
in the Git-owned timeline. The immutable segment ID remains the key used by
parents, goals, proofs, artifacts, and playback, so renaming never rewrites the
tree or invalidates evidence.

Segment **Delete subtree** previews and removes the selected segment and every
structural descendant from the Git-owned timeline. Goals and proofs attached to
that subtree are removed, named lineages are truncated or removed when emptied,
and attached draft descendants move to recoverable trash. Input tape/TAS files
and predicate definitions are deliberately retained because they may be shared;
Git exposes and can restore the complete topology edit.

**Keep this; delete siblings** treats the selected checked-in segment as the
survivor and previews every sibling currently displayed beneath the same
parent. Checked-in sibling subtrees are removed from the authored timeline,
direct sibling drafts and their descendants move to recoverable trash, and the
exact visible generated candidates are hidden by content ID without deleting
their search artifacts. Compatible goals, proofs, and named-continuation steps
are re-anchored to the survivor. New future search candidates remain visible.
The complete displayed set is bound into the confirmation token; root segments,
empty selections, stale confirmations, and active recording subtrees are
rejected.

**Record child** performs an exact windowless, muted, uncapped prefix replay,
then begins recording live port-0 input on the first PAD read after handoff.
The prefix is still fully simulated rather than replaced with a partial save.
The global Recording control selects host pacing after handoff; it never
changes the deterministic 30 Hz logical tick or one-input-frame-per-tick
contract.

Closing Dusklight finalizes the recording beneath the ignored
`build/automation-state/route-workbench/drafts/` tree. Drafts are temporary
child segments: they carry one parent, continuation input, fingerprints, and
verification state. Promotion creates a normal checked-in segment. Restarting
the workbench rediscovers ready drafts from disk.

Terminal thumbnails are an illustrative cache, not proof data. A proved
segment image is owned by its terminal boundary fingerprint; an unproved draft
image is owned by its finalized tape digest. Rebuilding Dusklight or renaming a
segment therefore does not invalidate the image. Browsing never mutates this
cache. Explicit pruning computes reachability from every current segment,
ready draft, and projected search result, previews by default, and moves
orphans to a recoverable transaction under the workbench state root:

```sh
huntctl timeline prune-thumbnails --timeline routes/intro.timeline \
  --repository-root . \
  --state-root build/automation-state/route-workbench
# Review the JSON report, then repeat with --apply.
```

Draft **Rename** changes only its human label. **Delete** previews and moves the
selected draft subtree to recoverable trash. Active, corrupt, detached, or
path-escaping drafts are rejected.

Predicate editing is likewise separate from topology. The server exposes only
the timeline-configured predicate program, validates optimistic revision
hashes, recompiles the complete source, and requires all referenced predicates
to remain defined. Editing a predicate makes matching proof hashes stale but
does not change the segment tree or input artifacts.

Boot itself is an authored origin predicate at pre-input boundary zero. A Boot
recording starts before the first emulated controller read and creates a root
draft. It is not represented by a placeholder tape or hidden segment.

The server binds only to loopback and owns all game, disc, repository, and
state paths. The browser cannot provide filesystem paths. Timeline edits are
reloaded from disk, while native gameplay observations remain read-only.

## DSL

The line-oriented format describes the same model directly:

```text
timeline intro
predicate_program intro/milestones.milestones
origin boot predicate process_boot

segment golf439 root profile boot_to_fsp103 uses tas intro/segments/golf439.tas starts process-clean-v1 produces LINK_STATE
label golf439 "Boot to Link control"
segment human420 after golf439 profile link_control_to_tunnel_crawl_start uses tape intro/segments/human420.tape starts LINK_STATE produces CRAWL_STATE_A
label human420 "Link control to tunnel crawl"
segment human_alt420 after golf439 profile link_control_to_tunnel_crawl_start uses tape intro/segments/human_alt420.tape starts LINK_STATE produces CRAWL_STATE_B

goal link_control on golf439 predicate link_control
goal tunnel_crawl_start on human420 predicate tunnel_crawl_start

proof golf439 satisfies link_control program PROGRAM_SHA256 predicate DEFINITION_SHA256 ticks 439
proof human420 satisfies tunnel_crawl_start program PROGRAM_SHA256 predicate DEFINITION_SHA256 ticks 420
proof human_alt420 satisfies tunnel_crawl_start program PROGRAM_SHA256 predicate DEFINITION_SHA256 ticks 420

continuation main starts root@process-clean-v1
continue main with golf439 after root@process-clean-v1
continue main with human420 after golf439@LINK_STATE
branch experiment from main after golf439
continue experiment with human_alt420 after golf439@LINK_STATE
```

Artifact forms are `uses candidate`, `uses tas`, `uses tape`, and generated
profile baselines where supported. Paths are relative to the timeline file.
Validation rejects duplicate IDs, missing parents, parent cycles, discontinuous
fingerprints, invalid artifacts, unknown goals, and stale proof identities.

Run the workbench directly with:

```powershell
cargo run --manifest-path tools/huntctl/Cargo.toml -- timeline workbench `
  --timeline routes/intro.timeline `
  --game build/windows-clang-debug/dusklight.exe `
  --dvd orig/GZ2E01/GZ2E01.iso
```

`--dvd` may be omitted to use Dusklight's configured image. Portable random
access remains deferred until a full checkpoint can reproduce live actors,
collision, loaders, heaps, and host-side state; until then exact prefix replay
is authoritative.

Every playable route node and standalone Project tape exposes **Playback** and
**Resume (accelerated)**. Both compose the complete absolute tape from its
declared process or stage boot. Accelerated resume suppresses its window,
presentation, and audio, runs the complete tape uncapped, submits the retained
terminal image without a simulation tick, then reveals it and releases live
input at normal pacing. It deliberately ignores Playback speed and works on
root nodes. The real renderer remains active so render-side game state can
resume; it is not Aurora's irreversible null backend. None of these controls
change logical game time.

Standalone entries are declared in `projects/workbench.projects`. Group IDs use
slash-separated parents; each project points at checked TAS source or a compact
tape and may attach a checked scenario fixture. The compiled tape remains the
authority for boot type. Process boot starts from a clean process; stage boot
records its map, room, spawn point, layer, optional save slot, inventory,
equipment, flags, settings, and RNG fixture directly in the tape launched by the
workbench.

## Route store

The content-addressed route store is an optional derived index, not route
authority. Its objects mirror segment, goal, proof, and pinned-path identities.
Git-owned timeline files and artifacts remain the source of truth.

`huntctl timeline store gc --store DIR` verifies the complete reachable object
graph and every unreachable object, then emits a dry-run report. Repeating with
`--apply` moves unreachable objects into a unique `trash/objects/gc-*`
transaction inside the store; it does not delete them. The report names that
transaction so an accidental collection remains directly recoverable.
