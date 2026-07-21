# JStudio STB programs and adaptor profiles

Status: `jstudio-stb-program/v1` is the version-neutral structural boundary.
`jstudio-adaptor-profile/v1` binds object selectors to the audited adaptor
dispatch of one exact executable, and `jstudio-semantic-program/v1` applies that
profile to user-supplied STB bytes. None of these artifacts alone claims that a
cutscene completed or that a gameplay write occurred.

## Structural extraction

The structural extractor validates the STB signature, byte order, format and
target versions, declared size, block count, aligned block coverage, nested FVB
coverage, object headers, command payloads, paragraph headers, and relative-jump
targets. It records:

- exact archive and STB resource digests;
- outer block coordinates, types, and block digests;
- indexed FVB function IDs and hashed function payloads;
- object type/flag/ID plus every physical sequence command;
- end, flag-operation, wait, relative-jump, suspend, paragraph, and unknown
  command classes; and
- reserved paragraph controls and unresolved object-specific paragraph
  type/size/digest records.

The structural schema intentionally does not know that a selector means actor
shape, camera position, or message ID. Those meanings are executable behavior,
not properties of the STB container format.

```text
route-planner extract-jstudio-stb \
  --archive files/res/Object/Demo07_02.arc \
  --resource demo07_02.stb \
  --output demo07_02-program.json
```

For GZ2E01 `demo07_02.stb`, the canonical structural artifact has SHA-256
`b9334b80cfd8417c0c9eaf10123b1e3ba8187ac742fe9be3dc3987b416c72ff4`.
It proves a version-3 STB targeting JStudio version 6 with 30 outer blocks: one
embedded FVB containing 200 indexed functions and 29 object streams. The object
streams contain 387 commands and 817 paragraph headers. Their command classes
are 29 ends, 189 waits, 3 suspends, and 166 paragraph bundles; there are no
explicit command-level relative jumps.

## Exact adaptor resolution

The bundled GZ2E01 profile pins both content identity SHA-256
`6fc8c6f4c4dcd1671c037646b2660aa4a0e5602d4bf66aa6e109aba5f20a4aaa`
and executable SHA-256
`e7f197436815e66c4a11df3d7bd557d66083b641ff8a8e76439f3caba7ae60e8`.
It is ordinary runtime data and may be replaced explicitly for theorycrafting;
a neighboring build is never selected implicitly. The canonical profile SHA-256
is `7c4438f7d5a9c74406734770b152953548944670f65bf78cdbff0bec565575aa`.

Each rule is keyed by object four-character code and selector. Its target is one
of:

- one or more JStudio variables;
- an adaptor method with a typed payload contract; or
- a variable-or-adaptor dispatch whose operation code selects the path.

Variable operations distinguish clear, immediate IEEE-754 values, time-scaled
values, and function-value references. Floating-point payloads are stored as
exact bit words rather than JSON numbers. Adaptor calls distinguish direct
resource/name IDs, immediate integers/booleans/floats, and zero-payload invokes.
Operation `0x11` on a variable-or-adaptor selector is retained separately as an
output-binding-only behavior because the audited generic setter does not assign
a new value for that operation.

```text
route-planner resolve-jstudio-stb \
  --archive files/res/Object/Demo07_02.arc \
  --resource demo07_02.stb \
  --content-identity gz2e01-content.json \
  --output demo07_02-semantics.json
```

An explicit `--profile PROFILE.json` overrides the bundled profile, but it still
must match the selected exact content and executable digests. Unknown selectors,
unsupported operation/payload combinations, and non-ASCII object types remain
typed unresolved records instead of acquiring guessed semantics.

The exact GZ2E01 semantic artifact has SHA-256
`a560e4f30d55403a68ab65e533e08bcd0c84d8164c1dc3de557c21c230890da5`.
Of 817 paragraph records, 122 use reserved JStudio controls and 695 are
object-specific. All 695 object-specific records resolve through 29 audited
selector rules:

| Object | Resolved paragraphs | Semantic families |
| --- | ---: | --- |
| `JACT` | 555 | translation, rotation, scale, shape, animation, modes and frames |
| `JCMR` | 104 | position, target, roll, field of view, near/far |
| `JMSG` | 3 | demo message ID |
| `JPTC` | 22 | transform, resource, begin/fade/end, repeat |
| `JSND` | 11 | resource, begin/fade-out, on-exit behavior |

Every decoded payload is reconstructed during artifact validation and checked
against its structural size and SHA-256. A friendly integer, float word, name,
behavior, or handler cannot be changed independently of the source bytes.

The three `JMSG` records call `dMsgObject_setDemoMessage` with IDs 1632, 1633,
and 1634. Actor shape and animation records are now typed raw ID writes. Turning
those IDs into live actor behavior, resource lookups/results, story writes, or scene
changes requires the actor/event lifecycle layer and evidence for the executed
cutscene path.
