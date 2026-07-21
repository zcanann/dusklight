# JStudio STB structural programs

Status: `jstudio-stb-program/v1` provides a bounded, canonical structural decode
of retail JStudio sequence files. It is not yet an executable cutscene-effects
program.

The extractor validates the STB signature, byte order, format and target
versions, declared size, block count, aligned block coverage, nested FVB
coverage, object headers, command payloads, paragraph headers, and relative-jump
targets. It records:

- exact archive and STB resource digests;
- outer block coordinates, types, and block digests;
- indexed FVB function IDs and hashed function payloads;
- object type/flag/ID plus every physical sequence command;
- end, flag-operation, wait, relative-jump, suspend, paragraph, and unknown
  command classes;
- reserved paragraph controls and object-specific paragraph type/size/digest;
  and
- explicit coverage stating that paragraph semantics remain unresolved.

Animation, camera, actor, sound, particle, and message payload bytes are not
copied into the derived artifact. Their sizes and SHA-256 identities make later
decoders reproducible from user-supplied originals without turning the fact pack
into a copy of the source asset.

```text
route-planner extract-jstudio-stb \
  --archive files/res/Object/Demo07_02.arc \
  --resource demo07_02.stb \
  --output demo07_02-program.json
```

For GZ2E01 `demo07_02.stb`, the canonical artifact has SHA-256
`b9334b80cfd8417c0c9eaf10123b1e3ba8187ac742fe9be3dc3987b416c72ff4`.
It proves a version-3 STB targeting JStudio version 6 with 30 outer blocks:
one embedded FVB containing 200 indexed functions and 29 object streams. The
object streams contain 387 commands and 817 paragraph headers. Their command
classes are 29 ends, 189 waits, 3 suspends, and 166 paragraph bundles; there are
no explicit command-level relative jumps.

That structural result narrows the next audit but does not identify gameplay
writes. The 26 observed object-specific paragraph type codes still need to be
bound through the relevant JStudio adaptors and TP demo actors before they can
compile into state operations, actor/resource requests, or cleanup effects.
