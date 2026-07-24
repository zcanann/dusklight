# Exact GZ2P01 structural message contexts

The PAL GameCube input is now an exact registered content identity, and its
English and French message resources can be extracted, serialized, compiled,
and compared without granting unaudited message-handler effects.

## Exact inputs and derived packs

The canonical GZ2P01 scan is
`22c1cc7ee9c667392b20c3718385e0b1a9e00f2803f88a6316be31384d376540`.
Extraction produced:

- bundle SHA-256
  `0fc0285954f43d0ab56b1d6fd9ca9119a2914dd8dd43f055ab53e3edbfb5182f`;
- fact-pack manifest SHA-256
  `73d4488b1217a2fe1f236a17b208719b5f88bd957b72f8f7a1eaa3268f982944`;
- 3,701 sealed files, 384 stage archives, 45 selected message archives, and
  five explicitly ignored group-99 archives.

For the second exact identity, GZ2E01 extraction produced bundle
`b0316841e8b05eed56f9c632f01a173135600a503710cfc0abda07b868eea9d1`
and manifest
`bae6e03843dd6ff8b78e9fda27949ddcbca04bd5120a0c7bd833123277f77812`.
Both payload/manifest pairs were installed in the immutable cache, materialized
again by manifest digest, and compared byte-for-byte with their source
artifacts.

## Structural PAL profile

`gcn-pal-1.0-gz2p01-structural` maps the five PAL runtime languages to the
exact locale directories but intentionally supplies no temporary, persistent,
item-ownership, or switch-store bindings. Extracted nodes and encoded control
flow remain usable; any handler requiring one of those backings compiles with
an unknown requirement rather than an invented state effect.

The profile SHA-256 is
`11d853e76abd74f44e95466e8a7b992a264295ea2ea4884d8fae943be9875476`.
The exact runtime configurations produce:

| Runtime | Bundle | Program set | Compiled set | Manifest | Resources | Transitions | Readers / aliases |
| --- | --- | --- | --- | --- | ---: | ---: | ---: |
| PAL English | `Msguk` | `b14750517e5b77e68a1e699685508bec6f1e795d17911401e2425400593e64f1` | `0796d6480154bd2c2b06515c52869099940591411786d6e78c6515da9af1ccc3` | `24121120e864a6051669942942c41ed480c4e0f7d87d8c907ccf65b9ac165f8e` | 9 | 16,993 | 0 / 0 |
| PAL French | `Msgfr` | `78a6e6c00994f1e0b764dec7de851d57530f8496ffb0fa595574223965e71366` | `156f259f9f2c25396e502fd26e4c8e6594628ffa96865d9a416c351b6691c23c` | `803667e37e495048fc4d5b0bd636282098c5d8675beef85b78da83c3192ec93d` | 9 | 16,993 | 0 / 0 |

The two contexts share one content identity but have different runtime
configuration and resource/program digests. No equivalence set is inferred.
The cannon-payment audit found that the exact `Msguk` and `Msgfr` control-flow
arrays are identical, so this GameCube PAL identity does not evidence the
reported French-only graph shortcut. The repository report names specific
French Wii versions; exact affected-build and actor/event evidence remain
separate. See `cannon-payment-flow-audit.md`.

## Reproduction

```text
route-planner extract-orig \
  --orig orig/GZ2P01 \
  --content-id gcn-pal-1.0-gz2p01 \
  --output build/route-planner/gz2p01.bundle.json \
  --manifest build/route-planner/gz2p01.bundle.manifest.json

route-planner compile-message-flows \
  --bundle build/route-planner/gz2p01.bundle.json \
  --runtime-configuration tools/route-planner/crates/engine/data/runtime-configurations/gz2p01-en.json \
  --profile tools/route-planner/crates/engine/data/message-import-profiles/gz2p01-structural.json \
  --output build/route-planner/gz2p01-en.compiled-message-flows.json \
  --manifest build/route-planner/gz2p01-en.compiled-message-flows.manifest.json
```

Use `gz2p01-fr.json` for the French context.
