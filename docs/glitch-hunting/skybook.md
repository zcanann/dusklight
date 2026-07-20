# Skybook benchmark import

Skybook is an offline requirements corpus, not a runtime service and not a
directory Dusklight may mutate. The checked manifest was imported from
[`qwertyquerty/skybook`](https://github.com/qwertyquerty/skybook) at exact Git
revision `e9104852ff6b87862b67100f58aaa729096b42dc`.

## Checked artifact

`benchmarks/skybook/manifest.json` uses schema
`dusklight-skybook-manifest/v2`. It contains 483 pages, of which 452 have the
`Glitches` category. Its canonical content digest is
`bec0be7ef25f970c0ee59ff2105ec734b17a689ed41d3dc924c2037a311aedf6`.

Each page retains:

- its repository-relative source path and revision-bound GitHub URL;
- SHA-256 identities for the original source bytes and normalized body;
- all front-matter keys plus title, description, author, category, and tag
  projections;
- platform and map names projected from `platform-*` and `map-*` tags;
- original tags alongside canonical tags, per-page alias resolutions, and
  canonical platform, map, and region projections;
- the Markdown body, internal link labels/targets/resolved slugs, external
  source links, image alt/source pairs, and typed video evidence.

Retaining the body makes the artifact self-contained for later capability
triage. The game, workers, and normal huntctl workflows do not read the
external Skybook checkout.

## Learner requirements index

`benchmarks/skybook/requirements-index.json` is a complete, overlapping
mechanism index over all 483 manifest pages. It is bound to the same source
revision and manifest digest. The checked index has canonical content digest
`c610657bbf327c5f4b0a9c071598b445b1a3cb047456fa283ded0da7a8c9f07e`.

The classifier can read only canonical tags, titles, and descriptions. It
cannot read page bodies, so published controller sequences and setup steps
cannot become tactics, routes, checkpoints, or rewards. Location, platform,
and region tags are also excluded as mechanism evidence. The index exists to
route later audits toward observation fields, action authority, temporal
memory, and read-only success oracles; it does not claim that Dusklight knows
how to perform any indexed glitch.

Generate and validate it with:

```sh
cargo run --manifest-path tools/huntctl/Cargo.toml -- \
  benchmark index-skybook-requirements \
  --manifest benchmarks/skybook/manifest.json \
  --output build/skybook-requirements.json

cargo run --manifest-path tools/huntctl/Cargo.toml -- \
  benchmark validate-skybook-requirements \
  --manifest benchmarks/skybook/manifest.json \
  --index benchmarks/skybook/requirements-index.json
```

Validation reclassifies every page from the bound manifest and rejects stale
source identity, missing or reordered pages, edited classifications, summary
count drift, and content-digest drift. Two independent generations of the
checked index were byte-identical with file SHA-256
`a40adcebd7bdfd51223c704c50ebc98bb162683482bd1dfe5be8c8f14f33b710`.

## Read-only import

Generate a new manifest explicitly:

```sh
cargo run --manifest-path tools/huntctl/Cargo.toml -- \
  benchmark import-skybook \
  --source ../skybook \
  --revision e9104852ff6b87862b67100f58aaa729096b42dc \
  --output build/skybook-manifest.json
```

The command resolves the checkout's full Git revision and refuses to import if
`_posts` has tracked, staged, or untracked changes. An explicit `--revision`
must match `HEAD`. The importer reads `_posts/*.md` only, accepts the corpus's
CRLF, UTF-8 BOM, and empty-scalar variants, and never writes into Skybook.

Pages are ordered by portable source path. Sets of extracted evidence are
ordered and deduplicated. The manifest digest covers the schema, source
identity, counts, and every page; validation recomputes body and manifest
digests and rejects reordered, duplicated, detached, or modified content. Two
imports from the pinned checkout produced byte-identical JSON with file
SHA-256 `e7cb73980bb6ef5c176ed6e2669ee04ceaceb0133d9f68f81ac7f5d9100d64b0`.

## Alias catalog

Normalization is a checked, versioned catalog rather than a fuzzy text rule.
The current seven source-to-canonical mappings are:

| Source tag | Canonical tag |
| --- | --- |
| `castle-town-sewers` | `map-castle-town-sewers` |
| `map-snowpeak-mountain` | `map-snowpeak-mountains` |
| `map-zora-river` | `map-zoras-river` |
| `platform-gcn` | `platform-gamecube` |
| `platform-hd` | `platform-wii-u-hd` |
| `platform-pal` | `region-pal` |
| `reference` | `type-reference` |

These resolve 31 occurrences across 29 pages. Each page still stores its raw
tags and source path, so normalization is reversible and does not rewrite
Skybook terminology. Manifest validation recomputes canonical projections from
the checked catalog and rejects altered rules or resolutions.

## Current boundary

The importer preserves source evidence, and the derived index performs only
coarse metadata classification. Neither assigns learner readiness, encodes a
reproduction, nor creates optimization targets. Representative human review is
still required to identify capability gaps, and any later benchmark selection
remains a separately reviewed decision. See
[project scope and benchmark selection](scope.md).
