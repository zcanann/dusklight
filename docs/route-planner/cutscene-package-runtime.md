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

The artifact marks archive failure behavior, lookup/parse behavior, and PACKAGE
mode-zero completion as resolved. It explicitly leaves these unresolved:

- whether the known actor-corruption setup produces this exact all-lookups-miss
  predicate;
- which outer event exit ultimately runs; and
- whether any other return-place writer runs on the observed corruption path.

Consequently the artifact cannot directly choose Castle Town or Zelda's tower,
cannot invent a return-place write, and cannot make the later savewarp implicit.

```text
route-planner resolve-cutscene-package \
  --content-identity gz2e01-content.json \
  --topology r-sp301-demo07_02-wrapper.json \
  --semantics gz2e01-demo07_02-semantics.json \
  --output gz2e01-demo07_02-package.json
```

An explicit `--profile` supports another audited build or a theorycraft profile,
but exact content, executable, wrapper, and semantic-program digests must all
match. There is no closest-build fallback.
