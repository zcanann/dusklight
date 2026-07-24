# Supported route-planner contexts

The planner recognizes three exact retail extracted-tree identities today.
“Registered” means that the complete executable/game-data/resource fingerprint
appears in the fail-closed build registry and a canonical input scan reproduces
it. Language rows separately declare whether they have semantic message-import
authority or only exact resource identity. A product ID, region label,
directory name, source-family branch, or neighboring revision is not
sufficient.

The canonical machine-readable catalog is
`tools/route-planner/crates/engine/data/supported-contexts.json`. Engine startup
tests decode it as canonical LF JSON, require a one-to-one match with
`supported-builds.json`, and bind the GZ2E01 English row to the exact bundled
message import profile. GZ2P01 and RZDE01 languages are
`resource_identity_only`; they cannot authorize message facts. Any changed
build fingerprint, zero scan/resource digest, cross-locale archive path,
duplicate row, unsorted row, or unregistered context fails validation.

## Exact registered builds

| Field | GCN PAL 1.0 | GCN USA 1.0 | Wii USA 1.2 |
| --- | --- | --- | --- |
| Stable content ID | `gcn-pal-1.0-gz2p01` | `gcn-us-1.0-gz2e01` | `wii-us-1.2-rzde01` |
| Product ID | `GZ2P01` | `GZ2E01` | `RZDE01` |
| Canonical input scan | `22c1cc7ee9c667392b20c3718385e0b1a9e00f2803f88a6316be31384d376540` | `a4bde024c74d8bd14912932aa3c5cfe733fe5b28dee54e20f1b36fe2eaece7ca` | `aeceea7b356febccd88f10d42e60821b825b326524da5f7e1b9db9142a560ba1` |
| Executable SHA-256 | `c52754b7be32734754ac9aa2b16d3e73c3dbfc4151780ceaae6a71a2c591ed08` | `e7f197436815e66c4a11df3d7bd557d66083b641ff8a8e76439f3caba7ae60e8` | `5b31fd11b3a7ab96e7dce1e94635046569e6b5887a0bd29de9ff966e3fa4be07` |
| Normalized game-data manifest | `327476f1f9c759e5d999e8fb1e6ce801b3335abea668ad2c4eeab5cb96fb49a6` | `0bc3bb229279d4b8a8c7cbe962b0bffdfecd35ff21e2d6761ad42e90a070f772` | `6e22de21d639a69fd2e571260f9d367874d95f0421571c6db2fb2badfa5da6b8` |
| Resource-archive manifest | `84095d3c39d39a62d76934185b292a569b4173e117261d7bbe2d99af7c1ae83e` | `2ab36f6c1d9d551c1397e1cf59e13288d2684c973cb7bd0ad6878f5a3b3a2ab1` | `c67d316b2ed920238b9e49801e1d54fff08d630d3ebad9fd15d89debd266fdda` |
| Exact retail container | unavailable | ISO `490ef919f413e00daedb4711777c3be05ef9afc12c7acca7675c721c9393c814` | unavailable |

An empty container list means the canonical extracted tree was reproduced but
the original container bytes were unavailable. It never means that a
reconstructed image is equivalent to the retail container. The normalized
game-data identity remains the extraction authority and seals every normalized
file record without depending on a host path.

## Exact supported runtime language

Only GZ2E01 English has semantic message-import support. Runtime language `en` selects
locale bundle `us` through profile `gcn-us-1.0-gz2e01-en`, whose canonical
whole-artifact SHA-256 is
`f969f0706818448f3ef93b9b447821f30dd9f4e576167c0eb8e7f3d5c3cc4889`
(the profile's internal semantic content seal is
`6fc8c6f4c4dcd1671c037646b2660aa4a0e5602d4bf66aa6e109aba5f20a4aaa`).
The catalog independently seals the nine selected `Msgus` archives:
`bmgres.arc` and `bmgres1.arc` through `bmgres8.arc`. This is the actual base
plus numbered set consumed by the profile; `bmgres99.arc` exists in the retail
tree but is not silently treated as a selected message-flow group.

GZ2P01 binds `de/en/es/fr/it` to `Msgde/Msguk/Msgsp/Msgfr/Msgit`; RZDE01 binds
`en/es/fr` to `Msgus/Msgsp/Msgfr`. Every row seals its actual base archive plus
groups 1 through 8. All eight rows are `resource_identity_only`, so callers may
inspect and diff exact resources but cannot construct message-flow facts until
the executable-specific handler/backing profile is audited. HD releases remain
absent because no exact executable or normalized resource tree is present.

## Stable-ID rule

The friendly content ID names exactly one complete `ContentFingerprint`; it is
not an alias for “US GameCube.” Runtime language is a separate mutable selection
under that content identity. Fact packs and import profiles bind both dimensions,
so a French runtime request cannot borrow the English row and a nearby build
cannot borrow the GZ2E01 facts.
