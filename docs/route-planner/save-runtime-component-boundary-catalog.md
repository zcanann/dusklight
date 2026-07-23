# Save/runtime component and boundary catalog

Status: implemented as a sealed, state-local inventory. Exact retail struct
members that no imported snapshot represents remain coverage gaps rather than
invented components.

The planner now derives
`dusklight.route-planner.component-boundary-catalog/v1` from a validated
execution state and one or more validated boundary policies. The catalog does
not maintain a parallel friendly component list. It walks every component in:

- the live execution environment;
- each transient serialized-component store;
- each persistent file image's runtime store; and
- each persistent file image's stage banks.

Every row retains its concrete storage location, component ID and kind,
binding, semantic lifetime, serialization owner, and payload coverage. Raw
payloads report their total and fully known bytes, structured payloads list all
field names, and unknown payloads retain their expected size when known. The
catalog also copies the active/inactive runtime-file identities and physical
slot images, and binds the complete result to the exact execution-state digest.

## Effective boundary matrix

For every supplied policy, the derivation emits exactly one effective row for
every live component. A row identifies whether its disposition came from:

- an explicit component rule;
- the policy's default disposition; or
- the execution state's one-boundary preserve override.

This distinction is important: an omitted selector is never displayed as an
implicit preserve. A default `unknown` remains `unknown`, and applying such a
policy still fails closed in the executor. If ID, kind, and binding selectors
overlap on the same component, catalog derivation rejects the policy exactly as
execution would. Validation independently recomputes every effective row from
the embedded policies, component metadata, and preserve set, so even a resealed
row cannot drift from its source rule.

## CLI

```text
route-planner catalog-state-boundaries \
  --state STATE.json \
  --policy TITLE_RETURN.json \
  --policy LOAD_SLOT.json \
  --output COMPONENT_BOUNDARIES.json
```

The output is canonical JSON with a content seal. Supplying no policy is an
error: a component inventory without any reset/transition boundary would not
satisfy this catalog's purpose.

## Coverage boundary

“Complete” is deliberately relative to the authenticated state and supplied
policy set. The catalog proves that none of those represented live or backed
components and none of those policy/component combinations were omitted. It
does not claim that currently unobserved `dSv_info_c` bytes, an unsupported
build, or an unaudited void/death policy has been decoded. Those remain explicit
work for the exact-build save/runtime audit.
