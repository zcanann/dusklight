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

The current goal engine proves Boolean predicates. Dynamic projections such as
“same RNG value as this reference segment” still need an explicit value-parity
form; they must not be approximated by topology or hard-coded folklore.

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

The left pane is a literal tree: Boot followed by nested segments. Alternative
attempts appear as siblings. Goals, proofs, fingerprints, scores, and artifact
information appear only in the selected segment's details. There are no
decorative type icons beside expanders because every checked-in entry has the
same structural meaning.

**Play** follows the selected segment's unique parent chain to Boot, composes
those exact artifacts, launches a fresh isolated process, and releases
controller ownership when the selected tape ends. Named continuations are
bookmarks and preferred paths, not playback authorization; an unpinned sibling
remains playable when its structural chain and fingerprints are valid.

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

**Record child** performs the same deterministic prefix replay, then begins
recording live port-0 input on the first PAD read after handoff. The prefix can
run hidden, muted, and unpaced, but it is still simulated rather than replaced
with a partial save. The window becomes visible and normal pacing returns at
handoff.

Closing Dusklight finalizes the recording beneath the ignored
`build/automation-state/route-workbench/drafts/` tree. Drafts are temporary
child segments: they carry one parent, continuation input, fingerprints, and
verification state. Promotion creates a normal checked-in segment. Restarting
the workbench rediscovers ready drafts from disk.

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

## Route store

The content-addressed route store is an optional derived index, not route
authority. Its objects mirror segment, goal, proof, and pinned-path identities.
Git-owned timeline files and artifacts remain the source of truth.
