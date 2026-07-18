# Skybook benchmark import

Skybook is an offline requirements corpus, not a runtime service and not a
directory Dusklight may mutate. The checked manifest was imported from
[`qwertyquerty/skybook`](https://github.com/qwertyquerty/skybook) at exact Git
revision `e9104852ff6b87862b67100f58aaa729096b42dc`.

## Checked artifact

`benchmarks/skybook/manifest.json` uses schema
`dusklight-skybook-manifest/v1`. It contains 483 pages, of which 452 have the
`Glitches` category. Its canonical content digest is
`b31e49f23ec79eee0d49783532f8bda8799b0eb2871a9da5e2e67ebed94fc270`.

Each page retains:

- its repository-relative source path and revision-bound GitHub URL;
- SHA-256 identities for the original source bytes and normalized body;
- all front-matter keys plus title, description, author, category, and tag
  projections;
- platform and map names projected from `platform-*` and `map-*` tags;
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
SHA-256 `bb2cddfa41c8f7383fbb77bb9d4fcbd3b9b23fcb86f282b92eb773102850b15f`.

## Current boundary

The importer records original names faithfully. It does not yet normalize
aliases, infer scenarios/actions/observations/oracles, assign readiness states,
or create per-glitch benchmark specifications. Those are separate roadmap
steps so extraction cannot silently masquerade as completed triage.
