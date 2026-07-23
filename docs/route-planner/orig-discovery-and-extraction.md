# Planner-owned `orig/` discovery and extraction

Status: deterministic discovery, exact registry classification, one-command
stage/message extraction, and immutable content-addressed fact-pack reuse are
implemented for extracted GameCube and Wii disc trees. Populating the registry
with audited retail fingerprints remains evidence work.

## Commands

Inspect an input tree without assigning it a friendly build label:

```text
route-planner scan-orig \
  --orig /path/to/orig \
  --product-id GZ2E01 \
  --output scan.json
```

Produce a verified derived bundle and manifest:

```text
route-planner extract-orig \
  --orig /path/to/orig \
  --output extracted-orig.json \
  --manifest extracted-orig.manifest.json
```

Convert that canonical bundle into planner-owned stage inventories without a
Huntctl artifact or crate dependency:

```text
route-planner construct-world-inventories \
  --bundle extracted-orig.json \
  --output extracted-orig-world-inventories.json
```

The default path identifies the input against the bundled audited registry.
`--content-id ID` can select one bundled entry when the parent contains multiple
games, and `--registry REGISTRY.json` can replace the bundled registry. An
explicit `--content-identity CONTENT.json` remains available for inspecting a
new exact build, but cannot be combined with registry-selection options.

Classify a tree against an exact supported-build registry:

```text
route-planner identify-orig \
  --orig /path/to/orig \
  --output identification.json
```

`--content-id ID` may select one friendly registry entry when the parent holds
multiple games. The friendly ID is only a selection hint: all fingerprint fields
must still match or the command fails. Without a hint, an exact fingerprint is
selected automatically. A well-formed but unlisted fingerprint produces an
explicit `unsupported` result containing the detected fingerprint; it never
inherits a nearby build's facts.

`--orig` may name the extracted game root containing `sys/` and `files/`, or a
parent containing one or more product directories. Registry-backed commands
use the selected exact identity's product ID to select among multiple games.
`scan-orig` requires either an unambiguous root or `--product-id`.

## Identity and failure behavior

Discovery reads the six-byte disc product ID and revision byte from
`sys/boot.bin`. Platform and region derive from that header, not from the folder
name. It then hashes `sys/main.dol`, every regular file beneath the extracted
`sys/` and `files/` trees, and a resource-only manifest. Optional container ISO
images or unrelated files beside those trees cannot change content identity or
force a redundant multi-gigabyte hash. Those three values form the detected
`ContentFingerprint`:

- executable SHA-256: `sys/main.dol`;
- game-data SHA-256: canonical normalized-path/size/SHA-256 manifest of every
  file; and
- resource-manifest SHA-256: the same canonical manifest restricted to
  `files/res/**/*.arc`.

`extract-orig` requires a canonical `ContentIdentity` and compares its complete
fingerprint to the detected fingerprint before decoding anything. A correct
product label with different executable or resource bytes fails. An incorrect
friendly label cannot override detected content. Unsupported disc prefixes,
unsupported region codes, missing boot/executable/resource files, ambiguous
roots, non-UTF-8 paths, and symbolic links all fail closed.

The strict registry schema maps a friendly content ID to one complete
`ContentIdentity`, is canonical and sorted, and rejects duplicate IDs or two
labels for one fingerprint. The bundled registry currently contains one locally
reproduced retail identity:

- `gcn-us-1.0-gz2e01`: GameCube USA, disc revision byte `0` (reported as
  revision `1.0`), product `GZ2E01`;
- executable SHA-256
  `e7f197436815e66c4a11df3d7bd557d66083b641ff8a8e76439f3caba7ae60e8`;
- normalized game-data manifest SHA-256
  `0bc3bb229279d4b8a8c7cbe962b0bffdfecd35ff21e2d6761ad42e90a070f772`;
- resource manifest SHA-256
  `2ab36f6c1d9d551c1397e1cf59e13288d2684c973cb7bd0ad6878f5a3b3a2ab1`.

The canonical registry source is
`tools/route-planner/crates/engine/data/supported-builds.json` and is embedded in
the planner binary. The repository does not claim fingerprints for retail
builds it has not reproduced. An unknown fingerprint is inspectable through
`scan-orig` and classifiable as unsupported, but is not silently treated as the
nearest known revision.

## Derived artifact

The canonical bundle contains only:

- the verified content identity;
- normalized relative paths, sizes, and source digests;
- decoded DZS/DZR chunk, actor/scaled/door placement, treasure, player-spawn,
  STAG, and SCLS records;
- decoded BMG flow graphs with temporary, persistent, and switch accesses; and
- explicit records for candidate archives containing no numbered message-flow
  resource, including their sorted resource-name inventory.

The bundle contains no original archive bytes and no absolute host paths. The
separate fact-pack manifest seals the bundle digest, extractor executable and
schema digests, source archive digests, and per-domain coverage. Physical
feasibility remains unavailable rather than being inferred from an encoded
destination.

Stage-data schema v4 keeps `actor_placements`, `treasure_placements`, and
`player_spawns` distinct. Its exact R_SP116 room-6 parity witness matches the
compatible inventory at 95 actors and five spawns for resource SHA-256
`10487ef6754fec1f454c93aa33f605ee9781b4db4b91eed8e864721d76304d40`;
the conditional retail regression reruns that check when `orig/` is present.

`construct-world-inventories` groups the decoded archives by their exact
`files/res/Stage/STAGE/{STG_00,RNN_00}.arc` coordinates, orders one stage source
before its room sources, and emits
`extracted-orig-world-inventories/v1`. The artifact binds the content,
game-data, and source-bundle digests and retains all decoded chunk, placement,
spawn, and SCLS source identities. Validation independently reconstructs every
represented field from the retained lowercase raw hex and proves complete
coverage of every recognized placement and SCLS chunk. Collision coverage is
explicitly `unavailable`: the command does not manufacture KCL/PLC paths,
spatial digests, prisms, or load-trigger joins.

Stage discovery recognizes `files/res/Stage/**/STG_00.arc` as `stage.dzs` and
room archives beginning with `R` as `room.dzr`. Message discovery treats
`files/res/Msg*/bmgres*.arc` as candidates, enumerates the bounded RARC, and
derives the group from the actual unique `zel_NN.bmg` resource rather than the
archive filename. This imports group 0 from `bmgres.arc` and records GZ2E01's
empty `bmgres99.arc` as intentionally ignored. Multiple numbered flow resources
remain an explicit ambiguity error. Any selected archive or resource that fails
bounded Yaz0/RARC/BMG/DZS decoding aborts the operation instead of producing a
partial success that looks complete.

`list-archive-resources` exposes that same deterministic basename inventory for
auditing filename/resource discrepancies without extracting original bytes.

## Build and language comparison

Compare all decoded records in two canonical bundles:

```text
route-planner diff-orig \
  --left extracted-left.json \
  --right extracted-right.json \
  --output orig-diff.json
```

For a cross-language comparison, pair the same message-group identities rather
than their locale-qualified paths:

```text
route-planner diff-orig \
  --left extracted-pal.json --left-locale fr \
  --right extracted-pal.json --right-locale de \
  --output fr-vs-de.json
```

Each stage, message-flow, and ignored-candidate record is classified as byte
identical, raw-changed but decoded-semantic-equal, semantic-changed, or uncovered
on one side. The artifact seals both exact input-bundle digests independently
from their content-identity digests. Locale comparisons normalize ignored
candidate names across the selected locale directories and also carry each
side's decoded group count, so
an absent or currently undecoded language bundle cannot produce a misleading
empty-equivalence result. The artifact does not claim behavior equivalence for
data the current extractors do not decode; adding those domains remains an
explicit extraction task.

## Content-addressed reuse

Install any derived payload and its canonical fact-pack manifest into the
planner-owned immutable cache:

```text
route-planner cache-fact-pack \
  --cache /path/to/planner-cache \
  --payload extracted-orig.json \
  --manifest extracted-orig.manifest.json \
  --receipt cache-receipt.json
```

The entry key is the manifest SHA-256. The cache verifies the payload digest,
uses create-new installation semantics, accepts byte-identical reuse, and
rejects collisions, tampering, or symlinked entries. Original game assets are
never installed.

Materialize the verified derived pack later, including on a machine with no
`orig/` tree:

```text
route-planner materialize-fact-pack \
  --cache /path/to/planner-cache \
  --manifest-sha256 SHA256 \
  --payload extracted-orig.json \
  --manifest extracted-orig.manifest.json
```

Loading rechecks the canonical manifest, requested manifest key, and payload
digest before returning bytes.

## Acceptance coverage

`orig_discovery::tests` verifies:

- direct-game-root and parent-`orig/` discovery produce identical scans;
- a misleading directory name cannot change the detected product;
- product mismatch, ambiguous roots, and symlinks fail closed;
- exact identity verification catches digest disagreement;
- the bundled registry is canonical and contains only the audited GZ2E01
  identity;
- registry lookup accepts only an exact fingerprint, reports unknown bytes as
  unsupported, and rejects a friendly-label override;
- one call decodes synthetic stage and message archives into a canonical bundle;
- group 0 is selected from its actual resource name and empty message candidates
  remain explicit ignored-archive records;
- raw/decoded comparisons distinguish semantic changes from container-only
  changes, retain ignored candidates, and report one-sided locale coverage;
- serialized output contains no host path; and
- mutating an archive after creating the identity causes extraction to fail;
- identical packs reuse one immutable cache entry; and
- payload mismatch or post-install tampering fails closed.
