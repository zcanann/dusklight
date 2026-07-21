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
  --content-identity content.json \
  --output extracted-orig.json \
  --manifest extracted-orig.manifest.json
```

Classify a tree against an exact supported-build registry:

```text
route-planner identify-orig \
  --orig /path/to/orig \
  --registry supported-builds.json \
  --output identification.json
```

`--content-id ID` may select one friendly registry entry when the parent holds
multiple games. The friendly ID is only a selection hint: all fingerprint fields
must still match or the command fails. Without a hint, an exact fingerprint is
selected automatically. A well-formed but unlisted fingerprint produces an
explicit `unsupported` result containing the detected fingerprint; it never
inherits a nearby build's facts.

`--orig` may name the extracted game root containing `sys/` and `files/`, or a
parent containing one or more product directories. `extract-orig` uses the
product ID in the exact content identity to select among multiple games.
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
labels for one fingerprint. The repository does not yet claim audited complete
fingerprints for retail builds it cannot reproduce locally. An unknown
fingerprint is inspectable through `scan-orig` and classifiable as unsupported,
but is not silently treated as the nearest known revision.

## Derived artifact

The canonical bundle contains only:

- the verified content identity;
- normalized relative paths, sizes, and source digests;
- decoded DZS/DZR chunk, placement, STAG, and SCLS records; and
- decoded BMG flow graphs with temporary, persistent, and switch accesses.

The bundle contains no original archive bytes and no absolute host paths. The
separate fact-pack manifest seals the bundle digest, extractor executable and
schema digests, source archive digests, and per-domain coverage. Physical
feasibility remains unavailable rather than being inferred from an encoded
destination.

Stage discovery recognizes `files/res/Stage/**/STG_00.arc` as `stage.dzs` and
room archives beginning with `R` as `room.dzr`. Message discovery recognizes
`files/res/Msg*/bmgresN.arc` and extracts `zel_NN.bmg`, retaining the locale
bundle and group number separately. Any recognized archive that fails bounded
Yaz0/RARC/BMG/DZS decoding aborts the operation instead of producing a partial
success that looks complete.

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
- registry lookup accepts only an exact fingerprint, reports unknown bytes as
  unsupported, and rejects a friendly-label override;
- one call decodes synthetic stage and message archives into a canonical bundle;
- serialized output contains no host path; and
- mutating an archive after creating the identity causes extraction to fail;
- identical packs reuse one immutable cache entry; and
- payload mismatch or post-install tampering fails closed.
