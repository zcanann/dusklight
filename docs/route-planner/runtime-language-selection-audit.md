# Runtime language and message-resource selection audit

Status: the selection and ownership contract is source-audited, and the only
registered exact build (GZ2E01) has an exact binary witness for its fixed
`Msgus` stage-message path. Exact PAL, Wii, and HD executable/resource identities
remain unregistered, so their language tables are source-family evidence rather
than exact-context planner facts.

## Three distinct pieces of state

The planner must not collapse these into one field:

1. **Console language** is external runtime configuration. GameCube code reads
   `OSGetLanguage()` and Wii-family code reads `SCGetLanguage()`. It is not owned
   by a TP save slot.
2. **Saved config byte** is `dSv_player_config_c::mLanguage`, at component-local
   offset `0x4` inside the serialized player configuration. Initialization copies
   a console-language value on some builds, but the retail language accessor does
   not read this byte and no other audited source reference reads or writes it.
3. **Mounted message resource** is the concrete locale archive chosen when the
   base or numbered BMG group is mounted. Already extracted/compiled flow data
   remains tied to that archive's digest; changing a configuration value cannot
   retroactively relabel it.

Accordingly, friendly `runtime.language` is a build-specific interpretation of
external configuration, while a compiled message program is bound to a concrete
resource. The saved byte remains observable raw state even where it is
semantically inert for message selection.

## Exact GZ2E01 behavior

GZ2E01 is GameCube USA. Its compiled `readMessageGroupLocal` function is at
virtual address `0x80236bf8`, size `0x98`. The planner-owned
`binary-function-evidence/v1` artifact has SHA-256
`c9398ec0b4ad4908eeab818d8e98c2153b9bef5a16e590df1a451fe297c5f152`;
the selected code bytes have SHA-256
`91e23a804d713c5bb60c0d7a2b2afcda47f51f836c6fb907380a99be467f99de`.
The executable and symbol-table identities are the already registered
`e7f197436815e66c4a11df3d7bd557d66083b641ff8a8e76439f3caba7ae60e8`
and
`8b8c98b86b6270543709adbbd489ca4a5cd4fa5c30fd4a410420702fd37a085a`.

That function references the NUL-terminated bytes
`/res/Msgus/bmgres%d.arc` at virtual address `0x803996cf` (DOL file offset
`0x3966cf`). Its canonical `binary-range-evidence/v1` artifact has SHA-256
`6f728f7bc8dd1a509f76dd90ac4a043ffab02bf672df9fd3cebbeacf7f8ebae2`;
the 24 selected bytes have SHA-256
`6676d909599d7580a719729cc10f28e34b5b5e4002512b5e5ecc2a951c6888d1`.
The base archive is likewise compiled as the 22 NUL-terminated bytes
`/res/Msgus/bmgres.arc` at virtual address `0x8039a12c` (DOL file offset
`0x39712c`). Its range artifact has SHA-256
`6c0031d1d6ce9a38ef2ad3eccc5d45e769f54b4d39374c3cc4b017d4fade44cb`;
the selected bytes have SHA-256
`b08e43df99c3e892d20330d1953ebacf9ba459060bb80fd80087c5d216af687b`.
`dSv_player_config_c::getPalLanguage()` falls through to `0` on this build.

Therefore the only established GZ2E01 runtime language is English, selected from
`Msgus`. A syntactically valid GZ2E01 runtime configuration tagged `fr` is not
evidence that French assets or French flow semantics exist for that content
identity. Consumers must continue to require an exact language/resource profile
instead of falling back to English or a nearby PAL build.

## GameCube PAL source-family behavior

For the GameCube PAL compile branch, `getPalLanguage()` reads
`OSGetLanguage()` each time and maps:

| OS value | Friendly language | Base/group directory |
| --- | --- | --- |
| `0` | English | `Msguk` |
| `1` | German | `Msgde` |
| `2` | French | `Msgfr` |
| `3` | Spanish | `Msgsp` |
| `4` | Italian | `Msgit` |
| other, including Dutch | fallback English | `Msguk` |

The logo scene mounts the matching base `bmgres.arc` and
`dMsgObject_c::readMessageGroupLocal` mounts the matching
`bmgres<group>.arc`, where the group comes from the current stage's STAG
record. Group `99` is normalized to logical group `9` only after the archive
request is formed.

This is why PAL English and PAL French are two runtime contexts over one exact
disc identity, not two builds and not universally equivalent message graphs.
The cannon-payment divergence must ultimately be imported from the exact
`Msguk` and `Msgfr` resources selected by those contexts.

## Persistence and change boundaries

- New player configuration initializes `mLanguage` from the platform API only
  on the applicable compile branches; GZ2E01 writes zero.
- The whole player configuration is contained in saved player data, so the byte
  can survive save/load. That persistence does not make it the selector:
  `getPalLanguage()` queries the platform API instead.
- No audited gameplay setter changes the console language. A language change is
  an external configuration/boot boundary unless future platform-specific
  evidence proves an in-process mutation route.
- Base and numbered message archives are selected when mounted. A theoretical
  in-process console-language mutation would affect later selection calls, not
  already mounted or already compiled resources. Such a route therefore needs
  explicit mount/unmount and message-session state rather than a global relabel.
- Language is also consulted directly by some presentation and formatting code.
  Resource equivalence alone cannot prove all language-sensitive behavior
  equivalent.

## Planner contract

The current separation between immutable `ContentIdentity` and mutable
`RuntimeConfiguration` is correct, with these refinements:

- exact content profiles enumerate supported language tags and their platform
  values/resource directories; arbitrary tags do not gain facts;
- selected BMG resources retain their archive/resource digests and runtime
  configuration digest, as the message importer already requires;
- the saved config byte is exposed as raw serialized state but is not aliased to
  authoritative language without exact-build reader evidence;
- mounted-resource identity is modeled separately if routes ever cross a
  language/configuration or resource-lifetime boundary; and
- comparisons report an unsupported or uncovered language explicitly. They do
  not substitute the closest directory, English, or another revision.

## Evidence boundary

The source snapshot used for the family-level audit has these SHA-256 values:

- `d_save.cpp`:
  `fa6f4f39f92e143ca0a010d191de65675137c5fb1621971b3f27cf15082179ec`;
- `d_msg_object.cpp`:
  `8a01e4005b6d49956d0a2e2297eb0b1795f52e7d130071e3025804733ccc7c15`;
- `d_s_logo.cpp`:
  `97d6cad43429c4123266da27c823a6872e748a958cd6acd2e0c51223f39c86a0`;
- `d_s_name.cpp`:
  `be8a9879977e676cbee758d7a2cbd37a5a3529e99d11a38df74ef15a1a4e52b5`;
- `d_save.h`:
  `2fff23b2be435502c1c9ec33ba4837f2f418bb7c08cf9f09c1c630f187bdfe59`;
  and
- `global.h`:
  `ab00abf2e6155271c85c2a6433ea0bf2004fe40cda6a7e5c6bbc2351b7f58da9`.

These sources establish the ownership and compile-branch design, but they do
not replace exact retail identities. Before enabling PAL or Wii language facts,
reproduce the relevant executable, enumerate its locale archives, bind each
runtime tag to exact resource digests, and compare the decoded semantics.

The exact GZ2E01 binary witness is reproducible with:

```text
route-planner extract-function-evidence \
  --dol orig/GZ2E01/sys/main.dol \
  --symbols config/GZ2E01/symbols.txt \
  --symbol readMessageGroupLocal__12dMsgObject_cFPP25mDoDvdThd_mountXArchive_c \
  --output gz2e01-read-message-group.json

route-planner extract-binary-range-evidence \
  --dol orig/GZ2E01/sys/main.dol \
  --virtual-address 0x803996cf \
  --size 24 \
  --output gz2e01-msgus-group-path.json

route-planner extract-binary-range-evidence \
  --dol orig/GZ2E01/sys/main.dol \
  --virtual-address 0x8039a12c \
  --size 22 \
  --output gz2e01-msgus-base-path.json
```
