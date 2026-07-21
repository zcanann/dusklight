# GZ2E01 post-Zelda cutscene source audit

Status: the exact retail sequence is now split into its two real cutscenes, and
the room-loader's nominal return-place writer is proven to be a retail no-op.
The primary wrong-warp witness visibly fails `Demo07_01.arc` in Zelda's tower,
not downstream `Demo07_02.arc`. The complete exceptional suffix and exact-build
correspondence remain unknown and must not be promoted to an established route.

## Corrected two-stage sequence

The primary witness is the video `8SjZ--barD0`, mirrored by the Skybook
`castle-town-wrong-warp` benchmark. The captured video bytes used for this audit
have SHA-256
`574443357d65fa3369c436596b0a8779b3e4c4baad24171582a10803efa69d16`.
Its allocation diagnostics visibly name `Demo07_01.arc`; the later save/reload
arrives in Castle Town. The video does not expose the precise return-place bytes,
all event-control flags, or a trustworthy exact disc identity.

The exact GZ2E01 resources establish this ordinary chain:

1. R_SP107 room 3 layer 8 selects `Demo07_01` and event `demo07_01`.
2. Its normal SCLS 2 enters R_SP301 room 0, spawn 20, layer 8. Its skip SCLS 3
   remains in R_SP107 room 3, spawn 1.
3. R_SP301 layer 8 selects `Demo07_02` and event `demo07_02`.
4. Its normal SCLS 1 enters Castle Town (`F_SP116`); its skip SCLS 2 returns to
   R_SP107 room 3, spawn 1.

Consequently, `demo07_02` is useful downstream evidence, but it is not the
archive failure seen in the wrong-warp witness.

The R_SP107 room archive SHA-256 is
`88cd34f72b7d193d82e3c564aa77a9a340e0dcb20aae681eb768af0a17bdf205`;
its `room.dzr` and `event_list.dat` digests are respectively
`ff9ac474c6c282be807b78c289a31a9358f752033b586ecc6888f497c0647370`
and
`409017e871c57cf61565ca7d1e949b49c178d330e52c0ad2ee4f77e4c187d528`.
The canonical `demo07_01` wrapper digest is
`6333063dc4f072cec00236061ec18046b728767631c1b83ea153e83573d407c4`.
Its exact package and outer artifacts have SHA-256
`9a120fb2d57250b9e239312431239903b13c05d2ff76ffbb6b228ec713cac50d`
and
`40a70665788d0030e6153cdef408d146ae45967dc3c0c57e3751f34f69c541f1`.
The event finishes on `[19, -1, -1]`.

## Downstream `demo07_02` resource chain

The GZ2E01 room archive
`files/res/Stage/R_SP301/R00_00.arc` has SHA-256
`1f8c692843344b7c739d53f940a65eb8f280a857b83879f07db38c94c390fa1d`.
Its `room.dzr` resource has SHA-256
`185cc38ccc5456405380aebccb99f12edf55063de4f048f84b41e9a35f71221e`.

The planner-owned DZR extractor now resolves these records:

- LBNK layer 8 is raw `0702ff`, selecting object archive `Demo07_02`.
- REVT record 0 is STB event `demo07_02`, map-tool ID 4, normal exit 1,
  skip exit 2, raw
  `0202030304ff64010302ffff0064656d6f30375f303200000000ffff`.
- SCLS exit 1 is `F_SP116`, spawn 20, room 0, layer 8 (Castle Town), raw
  `465f5350313136001400f01816`.
- SCLS exit 2 is `R_SP107`, spawn 1, room 3 (Zelda's tower), raw
  `525f5350313037000103f03f00`.

The same room archive's `event_list.dat` has SHA-256
`9b266caac37cb1c582161bb3e04dc2194d944cd1bc2d7040f8f63141ed64b5fe`.
The `extracted-event-list/v1` decoder proves that event `demo07_02` contains:

- PACKAGE `PLAY` with `FileName = demo07_02.stb`, followed by `WAIT`;
- CAMERA `STBWAIT` with `BGCheck = 3`;
- DIRECTOR `MAPTOOL` with `ID = 4`; and
- the parallel ALL/DUMMY staff completion.

The generic `cutscene-wrapper-topology/v1` join resolves all of those references
and the two SCLS records into one canonical artifact with SHA-256
`b4c74b3201720a9de93a0dd7fc4a71978579a81be339c147c474bc56514e20ec`.
Its coverage fields explicitly classify the outer wrapper as extracted and the
JStudio program, resource-failure control flow, and return-place writers as
unresolved.

`files/res/Object/Demo07_02.arc` has SHA-256
`7a5de4cf1bfb197430a7631b913311e245ed249b16d97616575cb58e001ac11a`.
Its selected `demo07_02.stb` resource has SHA-256
`6417533ffd470dfadcb96ef8a70f2acc7ee9037a4c71f5b864db50b84c176017`.
The planner-owned structural decoder emits canonical artifact SHA-256
`b9334b80cfd8417c0c9eaf10123b1e3ba8187ac742fe9be3dc3987b416c72ff4`.
It proves a version-3 STB targeting JStudio version 6, with 30 outer blocks:
one FVB bank containing 200 indexed functions and 29 object streams. Those
streams contain 387 commands and 817 paragraph headers. The command stream has
29 explicit ends, 189 waits, 3 suspends, 166 paragraph bundles, and no explicit
relative jumps.

The structural artifact remains version-neutral. A separate exact-GZ2E01
adaptor profile resolves all 695 object-specific paragraph occurrences through
29 selector rules while retaining 122 reserved controls. Its canonical semantic
artifact SHA-256 is
`a560e4f30d55403a68ab65e533e08bcd0c84d8164c1dc3de557c21c230890da5`.
This types actor shape/animation ID writes, camera/particle/sound
behavior, and demo-message IDs without proving which operations execute on the
corruption path or promoting any of them to gameplay writes.

## Exact room-loader writer evidence

The GZ2E01 executable `orig/GZ2E01/sys/main.dol` has SHA-256
`e7f197436815e66c4a11df3d7bd557d66083b641ff8a8e76439f3caba7ae60e8`.
The corresponding symbol table has SHA-256
`8b8c98b86b6270543709adbbd489ca4a5cd4fa5c30fd4a410420702fd37a085a`.
It declares `dComIfGp_ret_wp_set__FSc` at virtual address `0x8002f500` with
size `0x4`. The planner-owned `binary-function-evidence/v1` extractor maps it
to DOL text section 1, file offset `0x2c440`, and bytes `4e800020`: one PowerPC
`blr`, hence an immediate return with no state write.

The canonical evidence artifact has SHA-256
`d49e5c65992f2f7ba2c643399de880e4b857909c336eaae000712aa4d550338e`;
the selected four bytes have SHA-256
`f332ea5b5437103cbb6f1508679da89eec9288ad775c96c439a17fccabe3de8e`.
This proves that the call made after `roomInit` in `d_s_room.cpp` preserves
*every* incoming return-place value on this build. It is not a Castle-Town
special case. It does not, by itself, exclude a different writer on a partially
executed JStudio path or in the glitch's earlier setup.

## Source-backed control flow

`d_s_room.cpp` derives `Demo%02d_%02d` from the current layer's LBNK record,
requests that object archive, waits for its load phase, initializes the event
manager, reloads room actors, and then calls the no-op function proven above.
If the initial archive request fails it clears the demo-archive name. If a later
sync reports a negative phase, only the Wii USA revision-0 conditional invokes
`dStage_escapeRestart`; GZ2E01 falls through to room/event initialization.
`d_event_data.cpp` resolves PACKAGE `FileName`, loads the STB through the current
demo archive, starts JStudio, and resolves DIRECTOR `MAPTOOL` IDs. On event
completion, `d_event_manager.cpp` sends an STB event to REVT's normal exit, or
to its skip exit when the skip flag is active; `d_event.cpp` performs the
ordinary indexed SCLS scene change.

The audited source file SHA-256 values are:

- `d_s_room.cpp`: `15569f9038bb7fb9b956b82b205321c7bebd98ac947c4299739c720b94fe75b4`
- `d_event_manager.cpp`: `bbe434e385c99add82cc4bd0e57923244ce55835535fbd3392e3c086d1ec2c0d`
- `d_event_data.cpp`: `d9cf06093454fc60610bb1b550900feab4e8a0f13d485efcfe2961a8145cd6cc`
- `d_event.cpp`: `663b9f58268a1407827b7de13fc6add512c4eb6b71e1c3dd694df5110dc45eb3`
- `d_stage.h`: `f83222f27a4a93066bb0df7c4cee6861404c99f33a071d1071e8a7711ee799d5`

## Conservative boundary

These facts establish two authored completion destinations and prove that the
room-loader call cannot overwrite return place on GZ2E01. Source inspection also
shows that a wholly missing STB makes `dDemo_c::start` fail before setting demo
mode; PACKAGE then completes its PLAY cut when mode remains zero. CAMERA
`STBWAIT`, DIRECTOR `MAPTOOL`, and ALL/DUMMY have compatible early-completion
paths, but that is not yet evidence that the observed actor-corruption setup is
a whole-archive/STB-missing failure. The exact corrupted actor/resource, last
executed JStudio operation, skip flags, and final event-manager dispatch remain
unwitnessed.

The planner now seals that source-backed subset as
`resolved-cutscene-package/v1`. The exact artifact SHA-256 is
`2c99cd9c90795dd71c94529bc99b7478a32701446245cfc83610c81ed1162905`.
It proves the ordered demo/room/stage fallback, zero executed STB paragraphs,
absence of an exact PLAY-cut EventFlag parameter, and the mode-zero PACKAGE
completion guard.

A separate downstream `resolved-cutscene-outer-event/v1` artifact with SHA-256
`407d4d71d578506556f05e6b3bcea738fbf93dbf601722c2ccb749a9d818356e`
verifies the raw stage/event-list resources and proves that PACKAGE PLAY
advances to zero-timer WAIT, whose flag 5 satisfies the event finish condition.
It encodes PLAY and WAIT completion as distinct transitions, then preserves the
outer branch table as three more exact-context transitions: flag2 bit 1
suppresses scene change, while bit 2 selects the skip exit only when suppression
is clear (and, as in this REVT record, skip-cut type is 1); otherwise the normal
exit is selected. Its coverage keeps the
actor-corruption producer, actual outer branch taken, and other return-place
writers unresolved.

The producer boundary is represented separately by
`cutscene-corruption-hypothesis/v1`. The corrected GZ2E01/English
`demo07_01` hypothesis has SHA-256
`3a3b2ad8d4469e2b6ce888e2c73e274855f420ab7f70217532a4fda570588c16`.
The transition has unknown evidence and writes only the
all-STB-lookups-missing field. Its required unknowns retain the actual failure
site, proof that all STB lookups miss, and the last completed operation/prefix.
It has no direct scene or return-place effect.

## Ordinary return-place writer and reader

R_SP107 room 3 contains an unlayered `Savmem` actor with raw placement
`5361766d656d00000000ff0146a21c00459b19434525a000002d002fffffffff`.
Its parameters are `0x0000ff01`, and its angles are `[45, 47, -1]`.
`d_a_kytag14.cpp` decodes those values into spawn 1, room 3 (the placement's
home room overrides parameter room `0xff`), event-label index 45 required set,
event-label index 47 required unset, and no switch guards. `NO_TELOP` must also
be clear. When all guards hold, the actor writes R_SP107 room 3 spawn 1 into
the persistent player return place. The source digest is
`57744385e319f4f6df99298ce4ebeeb48b67558e557dd8dc0d56af35b22d9283`.

The ordinary savewarp reader is `dComIfGs_gameStart` in `d_com_inf_game.cpp`.
It passes the stored return-place stage, player-status/spawn, and room directly
to `setNextStage`; its source digest is
`b9b37aed0b76eef2d27b35a2ece6ee077086a970f98d18936a83649303f15761`.
Thus a Castle Town savewarp from the tower requires a Castle Town return place
to survive. Savewarp does not infer Castle Town from the cutscene or current
map. The remaining runtime question is why the tower `Savmem` writer does not
replace that incoming value on the witnessed failure path.

The failure must therefore still be modeled as a resource/actor failure
predicate with an unknown exceptional suffix, never as a direct warp to
`R_SP107` or `F_SP116`. The no-op proof narrows the unknown suffix: this specific
room-loader site is a confirmed preservation, not an ambiguous writer.

The next importer must preserve the incoming return place across the proven
no-op, invalidate it only if another potential writer's execution is ambiguous,
and leave the route unknown in established mode until a source or trace witness
identifies the actual corruption site and last completed operation. A later
savewarp remains a separate ordinary transition that reads whichever
return-place value survived.

## Planner commands

The extraction is independent of Huntctl:

```text
route-planner extract-stage-data \
  --archive files/res/Stage/R_SP301/R00_00.arc \
  --resource room.dzr \
  --output r-sp301-room.json

route-planner extract-event-list \
  --archive files/res/Stage/R_SP301/R00_00.arc \
  --output r-sp301-events.json

route-planner extract-cutscene-wrapper \
  --archive files/res/Stage/R_SP301/R00_00.arc \
  --event-name demo07_02 \
  --layer 8 \
  --output r-sp301-demo07_02-wrapper.json

route-planner extract-cutscene-wrapper \
  --archive files/res/Stage/R_SP107/R03_00.arc \
  --event-name demo07_01 \
  --layer 8 \
  --output r-sp107-demo07_01-wrapper.json

route-planner extract-function-evidence \
  --dol orig/GZ2E01/sys/main.dol \
  --symbols config/GZ2E01/symbols.txt \
  --symbol dComIfGp_ret_wp_set__FSc \
  --output gz2e01-ret-wp-set.json

route-planner extract-jstudio-stb \
  --archive files/res/Object/Demo07_02.arc \
  --resource demo07_02.stb \
  --output gz2e01-demo07_02-program.json

route-planner resolve-jstudio-stb \
  --archive files/res/Object/Demo07_02.arc \
  --resource demo07_02.stb \
  --content-identity gz2e01-content.json \
  --output gz2e01-demo07_02-semantics.json

route-planner resolve-cutscene-package \
  --content-identity gz2e01-content.json \
  --topology r-sp301-demo07_02-wrapper.json \
  --semantics gz2e01-demo07_02-semantics.json \
  --output gz2e01-demo07_02-package.json

route-planner resolve-cutscene-outer \
  --content-identity gz2e01-content.json \
  --runtime-configuration gz2e01-runtime-en.json \
  --topology r-sp301-demo07_02-wrapper.json \
  --package gz2e01-demo07_02-package.json \
  --stage-resource-file room.dzr \
  --event-list-resource-file event_list.dat \
  --output gz2e01-demo07_02-outer.json

route-planner resolve-cutscene-package \
  --content-identity gz2e01-content.json \
  --topology r-sp107-demo07_01-wrapper.json \
  --semantics gz2e01-demo07_01-semantics.json \
  --profile data/cutscene-runtime-profiles/gz2e01-demo07_01.json \
  --output gz2e01-demo07_01-package.json

route-planner resolve-cutscene-outer \
  --content-identity gz2e01-content.json \
  --runtime-configuration gz2e01-runtime-en.json \
  --topology r-sp107-demo07_01-wrapper.json \
  --package gz2e01-demo07_01-package.json \
  --stage-resource-file room.dzr \
  --event-list-resource-file event_list.dat \
  --profile data/cutscene-outer-runtime-profiles/gz2e01-demo07_01.json \
  --output gz2e01-demo07_01-outer.json

route-planner compile-cutscene-corruption-hypothesis \
  --content-identity gz2e01-content.json \
  --runtime-configuration gz2e01-runtime-en.json \
  --outer-event gz2e01-demo07_01-outer.json \
  --outer-profile data/cutscene-outer-runtime-profiles/gz2e01-demo07_01.json \
  --output gz2e01-demo07_01-corruption-hypothesis.json
```

The extraction commands reject malformed offsets, overlapping tables,
out-of-range graph references, unsupported data dispatch types, and ambiguous
RARC resources.
