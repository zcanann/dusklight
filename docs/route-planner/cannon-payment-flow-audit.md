# Cannon-payment message-flow audit

Status: the exact supported inputs do not evidence a French-only
cannon-payment graph divergence. No planner mechanic or named version-skip
flag is authorized by this audit.

## Reported behavior and build scope

The repository's imported Skybook source record for “Lake Hylia - Free
Cannon” describes declining a cannon repair on specific **French Wii**
versions and then receiving the repair automatically. Its source record is
sealed in `benchmarks/skybook/manifest.json` with source SHA-256
`b42eb8e57179230e6d27e5960f43b84b114c0bd7e7b31c9e18c4ba21e2f2200e`
and points to the primary-source video `3qVasPAI5O8`.

That scope matters:

- GZ2P01 is GameCube PAL, with runtime-selectable `Msguk` and `Msgfr`
  resources.
- RZDE01 revision 1.2 is Wii USA. Its extracted tree contains `Msgus`,
  `Msgsp`, and `Msgfr`, but the audited USA compile branch mounts `Msgus`;
  the other directories are resource identities, not established runtime
  selections.
- No exact affected Wii PAL content identity is currently present in `orig/`
  or the supported-build registry.

The earlier task wording treated the report as a GameCube PAL graph
divergence. Exact extraction does not support that inference.

## Exact resource audit

The exact GZ2P01 bundle
`0fc0285954f43d0ab56b1d6fd9ca9119a2914dd8dd43f055ab53e3edbfb5182f`
was compared between `Msguk` and `Msgfr`. All nine decoded `FLW1` node arrays,
`FLW1` branch-target arrays, and `FLI1` label arrays are identical. In message
group 8, flow 1001 begins at node 945 in both locales.

The exact RZDE01 revision-1.2 extraction produced:

- bundle SHA-256
  `e22a5693d35148b9e741ee9b2fae936dff2e7f7bd6ce5b8d4c692e594a41f1e6`;
- manifest SHA-256
  `3a4cd74c096af7e1322fa026f3e75fbb8cf85a78519f9984a261dbcd0ab9d04a`;
- 3,704 sealed files, 384 stage archives, 27 selected message archives, and
  three explicitly ignored group-99 archives.

Its `Msgus`/`Msgfr` comparison also finds identical group-8 node arrays,
branch-target arrays, and labels. The exact group-8 resources differ because
their message text and other BMG sections differ:

| Resource | `Msgus` | `Msgfr` |
| --- | --- | --- |
| Archive | `c93718aec0b4979faf0bb903c98a6ddcbe3730c0c11dd533e8cc8176ff867fa5` | `446073d415c36e1436d513f40464bb19314afdb961ac97b84393745fceda236d` |
| `zel_08.bmg` | `11576980ba2de394577d83b0afcdebe503f486e5fe2fd3981688bd26a6383336` | `80498eaaec8fc8fdc7dedfb73f535324c66a0708d255372db63e5bc08ddda8fa` |

The exact F_SP115 `STAG` record selects message group 8. Its decoded Toby
placements use flow 1 for the ordinary Lake Hylia actor, flow 5 for one
layer-3 actor, and flow 1001 for the sky-cannon actors on layers 13 and 14.
The `Msgus` and `Msgfr` flow-1001 reachable subgraphs each contain 44 nodes and
no rupee-query or rupee-debit handler.

The group-8 payment subgraph is flow 4, starting at node 160. It is identical
in both exact locales:

- node 182 is raw query-table index 6, resolved handler 4, parameter 300;
- node 190 is event handler 3 with raw parameter 300.

The audited source family identifies resolved query handler 4 as the rupee
threshold check and event handler 3 as the rupee decrease. This establishes
the intended shape of the 300-rupee subgraph, but it does not establish how an
affected Wii PAL executable enters or bypasses it.

## Planner consequence

There is no evidenced locale-specific graph guard, write, or effect to import
for either supported exact content identity. In particular:

- GZ2P01 French cannot receive a free-cannon edge merely because it is a
  French runtime context;
- RZDE01's unused `Msgfr` resource cannot be promoted into a selectable French
  runtime or a French-only mechanic; and
- portable queries cannot inherit a shortcut from a friendly language label,
  an adjacent build, or the external report.

The report may depend on Wii PAL executable behavior, actor/event handoff, or
localized message/selection metadata outside `FLW1` and `FLI1`. Resolving it
requires an exact affected Wii PAL extraction, exact runtime-selection proof,
and an audit of the repair choice through the actor/event boundary. Until then
the route obligation remains unknown rather than reachable or impossible.

## Reproduction

```text
route-planner extract-orig \
  --orig orig/RZDE01_02/DATA \
  --content-id wii-us-1.2-rzde01 \
  --output build/route-planner/rzde01.bundle.json \
  --manifest build/route-planner/rzde01.bundle.manifest.json

route-planner diff-orig \
  --left build/route-planner/rzde01.bundle.json \
  --right build/route-planner/rzde01.bundle.json \
  --left-locale us \
  --right-locale fr \
  --output build/route-planner/rzde01-us-fr.diff.json
```

`diff-orig` intentionally reports each translated BMG resource as
semantically changed because its semantic seal includes the complete decoded
resource shape. The narrower cannon audit additionally content-compares the
group-8 `nodes`, `branch_targets`, and `labels` arrays and then traces the
flow-4 and flow-1001 reachable subgraphs.
