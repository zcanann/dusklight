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

The importer remains a source-extraction boundary. A separate deterministic
derivation writes `benchmarks/skybook/requirements.json` using schema
`dusklight-skybook-requirements/v1`:

```sh
cargo run --manifest-path tools/huntctl/Cargo.toml -- \
  benchmark derive-skybook-requirements \
  --manifest benchmarks/skybook/manifest.json \
  --output build/skybook-requirements.json
```

Every one of the 483 source pages maps to nonempty scenario, action,
observation, semantic-oracle, and fidelity requirements. Source tag provenance
is retained on inferred requirements. Structured list items under requirement,
setup, method, instruction, procedure, step, and execution headings become
known setup steps; missing fixture, timing, predicate, version, or setup facts
remain explicit unknowns rather than guesses.

The derived artifact is bound to the exact source revision and manifest content
digest. Its canonical content digest is
`e671f0c96feeefc4675bbd23b01333dd27ca2737495a92d82e6bbccba9621a9b`,
and independently generated files were byte-identical with file SHA-256
`27e28e76335242b4c0fe339971298de4b728d904ca91ae416571fb6cf86f1f44`.
Readiness states and executable per-glitch benchmark specifications remain
separate roadmap steps; a requirement projection is not a reproduction claim.
