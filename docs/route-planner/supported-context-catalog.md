# Supported route-planner contexts

The planner supports one exact retail content identity today. “Supported” means
that the complete executable/game-data/resource fingerprint appears in the
fail-closed build registry and that every runtime language the planner accepts
for that build appears in the canonical supported-context catalog. A product ID,
region label, directory name, source-family branch, or neighboring revision is
not sufficient.

The canonical machine-readable catalog is
`tools/route-planner/crates/engine/data/supported-contexts.json`. Engine startup
tests decode it as canonical LF JSON, require a one-to-one match with
`supported-builds.json`, and bind the GZ2E01 English row to the exact bundled
message import profile. Any changed build fingerprint, zero resource digest,
cross-locale archive path, duplicate row, unsorted row, or unregistered context
fails validation.

## Exact supported build

| Field | GCN USA 1.0 |
| --- | --- |
| Stable content ID | `gcn-us-1.0-gz2e01` |
| Product ID | `GZ2E01` |
| Platform / region / revision | GameCube / USA / 1.0 (disc revision byte 0) |
| Canonical 1,459,978,240-byte ISO SHA-256 | `490ef919f413e00daedb4711777c3be05ef9afc12c7acca7675c721c9393c814` |
| `sys/main.dol` SHA-256 | `e7f197436815e66c4a11df3d7bd557d66083b641ff8a8e76439f3caba7ae60e8` |
| Normalized game-data manifest SHA-256 | `0bc3bb229279d4b8a8c7cbe962b0bffdfecd35ff21e2d6761ad42e90a070f772` |
| Resource-archive manifest SHA-256 | `2ab36f6c1d9d551c1397e1cf59e13288d2684c973cb7bd0ad6878f5a3b3a2ab1` |

The ISO digest records the verified raw input image without embedding or
redistributing its bytes. The normalized game-data identity remains the
extraction authority: it seals every normalized file record and deliberately
does not depend on the host path or container filename.

## Exact supported runtime language

Only English is supported for this USA build. Runtime language `en` selects
locale bundle `us` through profile `gcn-us-1.0-gz2e01-en`, whose canonical
whole-artifact SHA-256 is
`fbd7d0e9e1c061fc338d187b0f37cc1c335cbf72a4f11b85453d572daeea096f`
(the profile's internal semantic content seal is
`6fc8c6f4c4dcd1671c037646b2660aa4a0e5602d4bf66aa6e109aba5f20a4aaa`).
The catalog independently seals the nine selected `Msgus` archives:
`bmgres.arc` and `bmgres1.arc` through `bmgres8.arc`. This is the actual base
plus numbered set consumed by the profile; `bmgres99.arc` exists in the retail
tree but is not silently treated as a selected message-flow group.

PAL languages, Wii revisions, and HD releases are deliberately absent. The
source-audited PAL language table documents likely resource-family behavior but
does not create supported rows. Those contexts remain unsupported until their
disc/executable/normalized-resource identities and every selected base/numbered
message archive are reproduced and added as exact catalog entries.

## Stable-ID rule

The friendly content ID names exactly one complete `ContentFingerprint`; it is
not an alias for “US GameCube.” Runtime language is a separate mutable selection
under that content identity. Fact packs and import profiles bind both dimensions,
so a French runtime request cannot borrow the English row and a nearby build
cannot borrow the GZ2E01 facts.
