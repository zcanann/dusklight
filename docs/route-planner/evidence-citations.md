# Evidence citations

The versioned `dusklight.route-planner.evidence-citation-index/v1` artifact
attaches research references to stable `EvidenceRecord` IDs without weakening or
rewriting the underlying fact or mechanics contract.

Each index is bound to the exact composed fact- and mechanics-catalog digests.
A citation whose evidence record is absent from those catalogs is rejected, as
is an index copied onto a later catalog revision. Citation IDs are unique and
canonically sorted.

Five citation kinds are explicit:

- `source` for source-code or binary-analysis references;
- `extraction` for content-addressed derived artifacts;
- `trace` for replay or observation evidence;
- `video` for audiovisual witnesses;
- `community` for community research.

Locators can name a content-addressed artifact, an exact repository revision and
normalized path with an optional inclusive line range and source digest, or an
HTTP(S) URL with an optional archived-content digest. URLs and repository paths
are bounded and normalized; all supplied digests must be nonzero.

Drafts become canonical, validated artifacts through:

```sh
tools/route-planner/target/debug/route-planner export-evidence-citations \
  --catalog composed-catalog.json \
  --input citations.draft.json \
  --output citations.json
```

The command prints the index digest and exact target catalog digests. It refuses
dangling evidence IDs, catalog drift, malformed locators, duplicate/unsorted
IDs, and noncanonical targets.
