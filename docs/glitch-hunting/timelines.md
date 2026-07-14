# Route timelines and immutable lineage

The route layer separates human-authored intent from stored evidence.
routes/intro.timeline is a readable view of milestone nodes, segment edges,
competing variants, continuations, and branches. The content-addressed route
store is the authority for imported programs, tapes, boundary states,
evaluations, lineages, and named route or experiment heads.

## Immutable variant model

A variant is one immutable attempt at one segment. It declares:

- a candidate or tape artifact;
- its exact starting boundary fingerprint;
- its produced boundary fingerprint; and
- an optional first-hit tick.

Boundary fingerprints represent the full state contract needed by the next
segment, including RNG-sensitive state. A faster sibling does not stale an
established continuation. It is only speed-comparable when both its starting
and produced fingerprints equal the incumbent's. Otherwise it is a separate
frontier point.

A continuation pins each segment variant to the exact preceding variant and
checkpoint fingerprint. A branch inherits a named continuation through a named
milestone and then supplies a different tail. Both remain valid when another
variant becomes incumbent.

The stale label is restricted to an explicit workspace preview:

    huntctl timeline status --timeline routes/intro.timeline --continuation main --select boot_to_link.golf

Selecting an upstream replacement marks the preview's descendants stale until
an explicit repair. Compatible repair produces authored text for a new
continuation and preserves the original:

    huntctl timeline rebase-compatible --timeline routes/intro.timeline --continuation main --select boot_to_link.golf --name main_golf

No command silently prunes or rewrites a lineage.

## DSL

The line-oriented format uses these declarations:

    timeline intro
    milestone process_boot
    milestone link_control
    segment boot_to_link from process_boot to link_control profile boot_to_fsp103
    variant boot_to_link.safe incumbent uses baseline boot_to_fsp103 starts process-v1 produces control-rng1
    continuation main starts root@process-v1
    continue main with boot_to_link.safe after root@process-v1
    branch experiment from main at link_control

Candidate and tape paths may be quoted. Comments start with a hash. The parser
reports source line and column and rejects duplicate names, missing references,
boundary mismatches, discontinuous continuations, and milestone or branch
cycles.

## Content-addressed route store

Initialize a store and atomically import a timeline snapshot:

    huntctl timeline store init build/route-store
    huntctl timeline store import --store build/route-store --timeline routes/intro.timeline --ref routes/intro

Every object ID is the SHA-256 of its canonical typed object. Objects are
immutable. Named refs are append-only head events, so promote cannot partially
overwrite a prior head. An import writes all objects before its single snapshot
ref event; an interrupted import leaves only unreachable objects.

Native evaluator output can be imported with its tape and observed boundary as
one immutable evaluation object:

    huntctl timeline store import-evaluation --store build/route-store --evaluation build/result.json --milestone f_sp104 --fingerprint fsp104-rng1 --ref evaluations/candidate

Fork a lineage into an experiment head, then append or repair through authored
timeline intent:

    huntctl timeline store fork --store build/route-store --from routes/intro --lineage main --to experiments/golf
    huntctl timeline store append --store build/route-store --ref experiments/golf --timeline routes/intro.timeline --continuation main
    huntctl timeline store replay-repair --store build/route-store --from experiments/golf --to experiments/golf-repaired --timeline routes/intro.timeline --continuation main_golf

verify rehashes typed objects and traverses all current refs, rejecting missing
references, invalid schemas, and cycles. Garbage collection is conservative
and dry-run by default:

    huntctl timeline store verify --store build/route-store
    huntctl timeline store gc --store build/route-store
    huntctl timeline store gc --store build/route-store --apply

Only objects unreachable from current named heads are eligible. Promotion and
garbage collection are always explicit.
