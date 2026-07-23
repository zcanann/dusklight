# Semantic context comparison

`compare-semantic-contexts` compares two planner states and their independently
composed catalogs without selecting a nearby build, language, or configuration.
The report schema is
`dusklight.route-planner.semantic-context-comparison/v1`.

```sh
tools/route-planner/target/debug/route-planner compare-semantic-contexts \
  --left-state left-state.json \
  --left-catalog left-catalog.json \
  --right-state right-state.json \
  --right-catalog right-catalog.json \
  --output comparison.json
```

Each side retains its complete runtime configuration, exact content and runtime
configuration digests, execution-state digest, composed/base catalog digests,
and refinement-stack digest. The top-level relation distinguishes the same exact
context, different runtime configurations over the same immutable content, and
different content.

Facts are joined only by stable semantic ID. Each row retains both independent
evaluations, scope applicability, evidence admission, authored confidence, and
raw binding. It is classified as equivalent, outcome-changed, binding-changed,
contract-changed, or one-sided. Mechanics records are likewise joined by record
family and stable ID and compared by deterministic content digest. Summary lists
make every one-sided or out-of-scope fact visible.

`fallback_used` is always `false`. A fact scoped to English GZ2E01 remains
inapplicable when the right state is changed to French; it is not evaluated with
English data merely because that is the closest installed profile. Evidenced
equivalence is opt-in and side-specific through `--left-equivalence-set` and
`--right-equivalence-set`. `--research` changes only evidence admission, never
context selection.
