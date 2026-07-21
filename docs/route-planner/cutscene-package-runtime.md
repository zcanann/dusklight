# Cutscene PACKAGE resource-failure control flow

Status: `cutscene-package-runtime-profile/v1` binds source-audited PACKAGE and
archive-load behavior to one exact executable. `resolved-cutscene-package/v1`
joins that profile to an exact outer wrapper and the nominal JStudio semantic
program. It is a control-flow artifact, not an actor-corruption technique or a
warp.

The join deliberately keeps three claims separate:

1. The nominal STB contains actor shape/animation ID writes.
2. A failed archive/STB lookup has known runtime consequences.
3. A particular corruption setup actually produces that failure at a witnessed
   point.

The first two are now source/data-backed for GZ2E01 `demo07_02`; the third is
still unresolved.

## Audited failure chain

For the exact GZ2E01 executable:

- rejection of the initial demo-archive request clears the selected archive
  name and room initialization continues;
- a negative asynchronous archive sync also continues room initialization on
  this build (the source's escape-restart branch is Wii USA revision 0 only);
- PACKAGE searches for its STB in the selected demo archive, then the current
  room archive, then the stage archive;
- when all lookups miss, the shared binary parser rejects the null pointer and
  `dDemo_c::start` returns false before its demo-mode write;
- the optional PACKAGE `EventFlag` check occurs after the start attempt; the
  exact `demo07_02` PLAY cut has no such parameter, so no event-flag write occurs;
  and
- PACKAGE completes PLAY when demo mode is zero.

Once the authored event finish condition is satisfied, the outer event manager
has three separate outcomes for this REVT record. The exact event's finish list
is `[5, -1, -1]`; it does not require every parallel staff path to finish:

- with scene-change suppression clear and skip inactive, it selects SCLS 1
  (`F_SP116`, Castle Town);
- with suppression clear and skip active, this record's skip-cut type 1 selects
  SCLS 2 (`R_SP107`, Zelda's tower); and
- with the suppression flag set, it selects no scene change regardless of the
  skip-active flag.

These are conditional outcomes, not a conclusion about the corruption path.
That path's two runtime flag values are still unknown.

Because parsing never begins on this path, zero of the nominal STB's semantic
paragraphs execute. This is stronger than saying a downstream actor resource
lookup failed after some prefix: it is specifically the all-STB-lookups-missing
path.

## Exact GZ2E01 result

The runtime profile pins:

- content identity SHA-256
  `6fc8c6f4c4dcd1671c037646b2660aa4a0e5602d4bf66aa6e109aba5f20a4aaa`;
- executable SHA-256
  `e7f197436815e66c4a11df3d7bd557d66083b641ff8a8e76439f3caba7ae60e8`;
- wrapper topology SHA-256
  `b4c74b3201720a9de93a0dd7fc4a71978579a81be339c147c474bc56514e20ec`;
  and
- nominal semantic-program SHA-256
  `a560e4f30d55403a68ab65e533e08bcd0c84d8164c1dc3de557c21c230890da5`.

The canonical profile SHA-256 is
`3e6b657f467de789bddae35c6c3073b5227a83dcdd0b95a82fee39dc87898825`.
The resolved `demo07_02` package artifact SHA-256 is
`2c99cd9c90795dd71c94529bc99b7478a32701446245cfc83610c81ed1162905`.
Its nominal program contains 73 actor-animation ID writes over 53 distinct raw
IDs and 50 actor-shape ID writes over 9 distinct raw IDs. Those counts describe
the nominal program; the missing-STB branch executes none of them.

The separate `cutscene-outer-runtime-profile/v1` has SHA-256
`350cd598eeea13768c8f901e7227fe60b21b125468230b911af497b9ccde9930`.
It binds the exact package artifact, state-field names, and audited event-data,
event-manager, and REVT source semantics. The resulting
`resolved-cutscene-outer-event/v1` artifact has SHA-256
`a867ffa2abf2a7c4a07810d8b8109b96deb755b068973e1141fd8315cf7938c6`.
It verifies the raw stage and event-list resources, proves PACKAGE PLAY advances
to a zero-timer WAIT whose flag 5 satisfies the event finish condition, and
emits two ordered completion transitions followed by the three conditioned
outer outcomes.

Together the artifacts mark archive failure behavior, lookup/parse behavior,
PACKAGE mode-zero completion, and the outer flag-conditioned dispatch table as
resolved. They explicitly leave these unresolved:

- whether the known actor-corruption setup produces this exact all-lookups-miss
  predicate;
- which outer event case the corruption path ultimately satisfies; and
- whether any other return-place writer runs on the observed corruption path.

Consequently the artifact cannot directly choose Castle Town or Zelda's tower,
cannot invent a return-place write, and cannot make the later savewarp implicit.

The separate `cutscene-corruption-hypothesis/v1` artifact models the remaining
research link without promoting it. Its canonical GZ2E01/English artifact
SHA-256 is
`4009349305be05f0f005095a341d417a500cb956c41415b475a22d349ec46323`.
Its unknown-evidence producer writes only
`package.stb_lookup_result = all_stb_lookups_missing`. It carries three explicit
unknown requirements for the actual failure site, whether all STB lookups
really miss, and the last completed operation/prefix. Validation rejects a
direct location or return-place effect, so theorycrafting the producer cannot
silently become a Castle Town warp.

```text
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

route-planner compile-cutscene-corruption-hypothesis \
  --content-identity gz2e01-content.json \
  --runtime-configuration gz2e01-runtime-en.json \
  --outer-event gz2e01-demo07_02-outer.json \
  --output gz2e01-demo07_02-corruption-hypothesis.json
```

An explicit `--profile` supports another audited build or a theorycraft profile,
but exact content, executable, wrapper, and semantic-program digests must all
match. There is no closest-build fallback.
